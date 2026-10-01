//! What the watcher knows about the project tree: scanning and diffing.

use std::collections::BTreeMap;
use std::fs::Metadata;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::change::{Change, ChangeKind};
use crate::exclude::{PathClass, classify};

/// What the watcher records about one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Entry {
    /// Whether the path is a directory (not following symlinks).
    pub is_dir: bool,
    /// The file's length; 0 for a directory.
    pub len: u64,
    /// The file's modification time; `None` for a directory, whose
    /// modification time changes with its entries and is not a change.
    pub modified: Option<SystemTime>,
}

impl Entry {
    /// The entry for `meta`, from `symlink_metadata`.
    pub fn of(meta: &Metadata) -> Self {
        if meta.is_dir() {
            Self {
                is_dir: true,
                len: 0,
                modified: None,
            }
        } else {
            Self {
                is_dir: false,
                len: meta.len(),
                modified: meta.modified().ok(),
            }
        }
    }
}

/// Every watched path under the root, keyed by its path relative to it.
///
/// `PathBuf` orders component-wise, so a directory's descendants sort
/// directly after it ([`descendants`]).
pub(crate) type Snapshot = BTreeMap<PathBuf, Entry>;

/// `dir` and every path under it in `snapshot`, in order.
pub(crate) fn subtree<'a>(
    snapshot: &'a Snapshot,
    dir: &'a Path,
) -> impl Iterator<Item = (&'a PathBuf, &'a Entry)> + 'a {
    snapshot
        .range::<Path, _>((Bound::Included(dir), Bound::Unbounded))
        .take_while(move |(path, _)| path.starts_with(dir))
}

/// Paths strictly under `dir` in `snapshot`.
pub(crate) fn descendants<'a>(
    snapshot: &'a Snapshot,
    dir: &'a Path,
) -> impl Iterator<Item = (&'a PathBuf, &'a Entry)> + 'a {
    subtree(snapshot, dir).filter(move |(path, _)| path.as_path() != dir)
}

/// Scans the directory `rel` under `root` recursively into `out`, skipping
/// excluded paths and not following symlinks. `rel` itself is not added.
///
/// `before_reading` runs on each directory (`rel` included) before its
/// entries are read, so a watch added there sees any entry created after the
/// read. It may stop the scan with an error. Directories that vanish or
/// cannot be read are skipped: a later event or rescan accounts for them.
pub(crate) fn scan<E>(
    root: &Path,
    rel: &Path,
    out: &mut Snapshot,
    before_reading: &mut dyn FnMut(&Path) -> Result<(), E>,
) -> Result<(), E> {
    before_reading(rel)?;
    let Ok(entries) = std::fs::read_dir(root.join(rel)) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let child = rel.join(entry.file_name());
        if classify(&child) != PathClass::Watched {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        let found = Entry::of(&meta);
        out.insert(child.clone(), found);
        if found.is_dir {
            scan(root, &child, out, before_reading)?;
        }
    }
    Ok(())
}

/// The changes that turn `old` into `new`, sorted by path. A directory is
/// created or deleted, never changed.
pub(crate) fn diff<'a>(
    old: impl IntoIterator<Item = (&'a PathBuf, &'a Entry)>,
    new: impl IntoIterator<Item = (&'a PathBuf, &'a Entry)>,
) -> Vec<Change> {
    let old: BTreeMap<_, _> = old.into_iter().collect();
    let new: BTreeMap<_, _> = new.into_iter().collect();
    let mut changes = Vec::new();
    for (path, before) in &old {
        let kind = match new.get(path) {
            None => Some(ChangeKind::Deleted),
            Some(after) if after.is_dir != before.is_dir => Some(ChangeKind::Changed),
            Some(after) if !after.is_dir && after != before => Some(ChangeKind::Changed),
            Some(_) => None,
        };
        if let Some(kind) = kind {
            changes.push(Change {
                path: (*path).clone(),
                kind,
            });
        }
    }
    for path in new.keys().filter(|path| !old.contains_key(*path)) {
        changes.push(Change {
            path: (*path).clone(),
            kind: ChangeKind::Created,
        });
    }
    changes.sort();
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use std::fs;

    fn file(len: u64) -> Entry {
        Entry {
            is_dir: false,
            len,
            modified: None,
        }
    }

    const DIR: Entry = Entry {
        is_dir: true,
        len: 0,
        modified: None,
    };

    fn snapshot(entries: &[(&str, Entry)]) -> Snapshot {
        entries
            .iter()
            .map(|(path, entry)| (PathBuf::from(path), *entry))
            .collect()
    }

    fn rendered(changes: &[Change]) -> Vec<String> {
        changes
            .iter()
            .map(|c| format!("{}:{}", c.kind, c.path.to_string_lossy().replace('\\', "/")))
            .collect()
    }

    #[test]
    fn diff_reports_created_changed_and_deleted_files() {
        let old = snapshot(&[("a", file(1)), ("b", file(1)), ("d", DIR)]);
        let new = snapshot(&[("a", file(2)), ("c", file(1)), ("d", DIR)]);
        assert_eq!(
            rendered(&diff(&old, &new)),
            ["changed:a", "deleted:b", "created:c"]
        );
    }

    #[test]
    fn a_path_that_changes_type_is_changed() {
        let old = snapshot(&[("a", DIR)]);
        let new = snapshot(&[("a", file(0))]);
        assert_eq!(rendered(&diff(&old, &new)), ["changed:a"]);
    }

    #[test]
    fn descendants_are_contiguous_after_their_directory() {
        let tree = snapshot(&[
            ("src/gone", DIR),
            ("src/gone/a.rs", file(1)),
            ("src/gone/z/b.rs", file(1)),
            ("src/gone.rs", file(1)),
            ("src/gone2", DIR),
            ("src/goner/c.rs", file(1)),
        ]);
        let under: Vec<_> = descendants(&tree, Path::new("src/gone"))
            .map(|(path, _)| path.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(under, ["src/gone/a.rs", "src/gone/z/b.rs"]);
        assert_eq!(subtree(&tree, Path::new("src/gone")).count(), 3);
    }

    #[test]
    fn scan_skips_excluded_paths_and_visits_directories_first() {
        let root = tempfile::tempdir().unwrap();
        for path in ["src/lib.rs", "target/debug/app", ".git/HEAD", "a/b/c.txt"] {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "x").unwrap();
        }
        let mut found = Snapshot::new();
        let mut visited = Vec::new();
        scan::<Infallible>(root.path(), Path::new(""), &mut found, &mut |dir| {
            visited.push(dir.to_string_lossy().replace('\\', "/"));
            Ok(())
        })
        .unwrap();
        let paths: Vec<_> = found
            .keys()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(paths, ["a", "a/b", "a/b/c.txt", "src", "src/lib.rs"]);
        visited.sort();
        assert_eq!(visited, ["", "a", "a/b", "src"]);
        assert!(!found[Path::new("src/lib.rs")].is_dir);
        assert_eq!(found[Path::new("src/lib.rs")].len, 1);
    }
}
