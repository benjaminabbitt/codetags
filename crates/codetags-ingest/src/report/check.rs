//! The edge-count regression checks: against the previous generation, and
//! against a committed baseline.

use std::collections::BTreeSet;

use serde::Serialize;

use super::EdgeCounts;

/// A file fails the previous-generation check when it loses more than this
/// percentage of its edges (or all of them).
///
/// # Why 50
///
/// The failures this check exists for are large. Stale SCIP bindings once
/// read a whole index as zero edges with no error, and a Jelly failure cut a
/// TypeScript graph by about 81% (brief §4.4): both lose far more than half.
/// An edit that removes more than half of a file's calls in one indexing run
/// is rare, and when it happens the report names the file and its counts, so
/// one look settles it; the next run compares against the edited
/// generation and passes. A lower threshold would fail on ordinary
/// refactors of small files (three calls to one is a 67% loss), and a higher
/// one would let a provider that silently lost most of a file through.
pub const DROP_THRESHOLD_PERCENT: u32 = 50;

/// A file whose edge count dropped suspiciously since the previous
/// generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EdgeDrop {
    /// The provider whose run recorded the counts.
    pub provider: String,
    /// The file, project-relative.
    pub path: String,
    /// Its edge count in the previous generation.
    pub previous: i64,
    /// Its edge count now; `None` when the provider recorded no count for
    /// it although the file still exists.
    pub current: Option<i64>,
}

/// Whether going from `previous` to `current` edges is a suspicious drop:
/// to zero from non-zero, or a loss of more than
/// [`DROP_THRESHOLD_PERCENT`].
pub fn is_drop(previous: i64, current: i64) -> bool {
    if previous <= 0 {
        return false;
    }
    if current <= 0 {
        return true;
    }
    let lost = i128::from(previous) - i128::from(current);
    lost * 100 > i128::from(previous) * i128::from(DROP_THRESHOLD_PERCENT)
}

/// The files whose edge counts dropped from `previous` to `current` (see
/// [`is_drop`]).
///
/// A file the previous generation counted and the current one does not is
/// a drop to nothing if `exists(path)` says the file is still in the
/// project (the provider lost it), and is skipped otherwise (it was
/// deleted). Files only the current generation counts never fail.
pub fn regressions(
    previous: &EdgeCounts,
    current: &EdgeCounts,
    exists: impl Fn(&str) -> bool,
) -> Vec<EdgeDrop> {
    let mut drops = Vec::new();
    for (provider, files) in previous {
        for (path, &before) in files {
            let now = current.get(provider).and_then(|files| files.get(path));
            let dropped = match now {
                Some(&now) => is_drop(before, now),
                None => before > 0 && exists(path),
            };
            if dropped {
                drops.push(EdgeDrop {
                    provider: provider.clone(),
                    path: path.clone(),
                    previous: before,
                    current: now.copied(),
                });
            }
        }
    }
    drops
}

/// A file whose edge count differs from its baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaselineDifference {
    /// The provider.
    pub provider: String,
    /// The file, project-relative.
    pub path: String,
    /// The baseline's count; `None` when the baseline lists no such file.
    pub expected: Option<i64>,
    /// The generation's count; `None` when the generation counts no such
    /// file.
    pub found: Option<i64>,
}

/// Every file whose count differs between `expected` and `found`, ordered
/// by provider and path. Any difference fails the check.
pub fn diff_baseline(expected: &EdgeCounts, found: &EdgeCounts) -> Vec<BaselineDifference> {
    let get = |counts: &EdgeCounts, provider: &str, path: &str| {
        counts
            .get(provider)
            .and_then(|files| files.get(path))
            .copied()
    };
    let keys: BTreeSet<(&str, &str)> = [expected, found]
        .into_iter()
        .flat_map(|counts| {
            counts.iter().flat_map(|(provider, files)| {
                files
                    .keys()
                    .map(move |path| (provider.as_str(), path.as_str()))
            })
        })
        .collect();
    keys.into_iter()
        .filter_map(|(provider, path)| {
            let want = get(expected, provider, path);
            let have = get(found, provider, path);
            (want != have).then(|| BaselineDifference {
                provider: provider.to_string(),
                path: path.to_string(),
                expected: want,
                found: have,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn counts(entries: &[(&str, &str, i64)]) -> EdgeCounts {
        let mut counts: EdgeCounts = BTreeMap::new();
        for &(provider, path, edges) in entries {
            counts
                .entry(provider.to_string())
                .or_default()
                .insert(path.to_string(), edges);
        }
        counts
    }

    #[test]
    fn losing_more_than_half_is_a_drop() {
        assert!(!is_drop(10, 10));
        assert!(!is_drop(10, 5), "exactly half is not more than half");
        assert!(is_drop(10, 4));
        assert!(is_drop(3, 1));
        assert!(!is_drop(2, 1));
        assert!(!is_drop(5, 9), "a gain never fails");
    }

    #[test]
    fn dropping_to_zero_from_non_zero_is_a_drop() {
        assert!(is_drop(1, 0));
        assert!(is_drop(100, 0));
        assert!(!is_drop(0, 0));
        assert!(!is_drop(0, 7));
    }

    #[test]
    fn regressions_name_each_dropped_file() {
        let previous = counts(&[("p", "a.rs", 10), ("p", "b.rs", 4), ("p", "c.rs", 2)]);
        let current = counts(&[("p", "a.rs", 0), ("p", "b.rs", 3), ("p", "c.rs", 2)]);
        let drops = regressions(&previous, &current, |_| true);
        assert_eq!(
            drops,
            [EdgeDrop {
                provider: "p".to_string(),
                path: "a.rs".to_string(),
                previous: 10,
                current: Some(0),
            }]
        );
    }

    #[test]
    fn a_missing_file_fails_only_while_it_exists() {
        let previous = counts(&[
            ("p", "kept.rs", 3),
            ("p", "deleted.rs", 3),
            ("p", "empty.rs", 0),
        ]);
        let current = counts(&[]);
        let drops = regressions(&previous, &current, |path| path != "deleted.rs");
        assert_eq!(
            drops,
            [EdgeDrop {
                provider: "p".to_string(),
                path: "kept.rs".to_string(),
                previous: 3,
                current: None,
            }]
        );
    }

    #[test]
    fn counts_are_compared_per_provider() {
        let previous = counts(&[("p", "a.rs", 4)]);
        let current = counts(&[("q", "a.rs", 4)]);
        assert_eq!(regressions(&previous, &current, |_| true).len(), 1);
    }

    #[test]
    fn new_files_never_fail() {
        let previous = counts(&[]);
        let current = counts(&[("p", "new.rs", 0)]);
        assert!(regressions(&previous, &current, |_| true).is_empty());
    }

    #[test]
    fn a_baseline_diff_lists_changed_missing_and_extra_files() {
        let expected = counts(&[("p", "a.rs", 1), ("p", "b.rs", 2), ("p", "gone.rs", 3)]);
        let found = counts(&[("p", "a.rs", 1), ("p", "b.rs", 5), ("p", "new.rs", 0)]);
        let diff = diff_baseline(&expected, &found);
        let summary: Vec<(&str, Option<i64>, Option<i64>)> = diff
            .iter()
            .map(|d| (d.path.as_str(), d.expected, d.found))
            .collect();
        assert_eq!(
            summary,
            [
                ("b.rs", Some(2), Some(5)),
                ("gone.rs", Some(3), None),
                ("new.rs", None, Some(0)),
            ]
        );
        assert!(diff_baseline(&expected, &expected).is_empty());
    }
}
