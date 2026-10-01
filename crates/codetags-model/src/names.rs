//! File names in the index directory.
//!
//! Generation `N` owns every file named `gen-<N>.<suffix>`: the database
//! `gen-<N>.duckdb`, DuckDB's side files (`gen-<N>.duckdb.wal`,
//! `gen-<N>.duckdb.tmp`), and the marker `gen-<N>.complete`.
//!
//! `N` is written in plain decimal, with no zero padding, and listings sort by
//! the parsed number, never by name. A number with a leading zero is not a
//! generation name, so each generation has exactly one spelling.

/// Suffix of a generation's database file.
pub(crate) const DATABASE: &str = "duckdb";
/// Suffix of a generation's completion marker.
pub(crate) const COMPLETE: &str = "complete";
/// The writer's lock file, in the index directory.
pub(crate) const WRITE_LOCK: &str = "write.lock";

/// The name of generation `number`'s file with `suffix`.
pub(crate) fn file_name(number: u64, suffix: &str) -> String {
    format!("gen-{number}.{suffix}")
}

/// Parses `gen-<N>.<suffix>` into `(N, suffix)`. Returns `None` for any other
/// name, including an `N` with a leading zero or one that overflows `u64`.
pub(crate) fn parse(name: &str) -> Option<(u64, &str)> {
    let rest = name.strip_prefix("gen-")?;
    let (digits, suffix) = rest.split_once('.')?;
    let canonical = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    if !canonical || suffix.is_empty() {
        return None;
    }
    Some((digits.parse().ok()?, suffix))
}

#[cfg(test)]
mod tests {
    use super::{file_name, parse};

    #[test]
    fn names_round_trip() {
        for number in [0, 1, 9, 10, 12_345, u64::MAX] {
            assert_eq!(
                parse(&file_name(number, "duckdb")),
                Some((number, "duckdb"))
            );
        }
    }

    #[test]
    fn side_files_belong_to_their_generation() {
        assert_eq!(parse("gen-7.duckdb.wal"), Some((7, "duckdb.wal")));
        assert_eq!(parse("gen-7.complete"), Some((7, "complete")));
    }

    #[test]
    fn other_names_are_not_generations() {
        for name in [
            "write.lock",
            "gen-.duckdb",
            "gen-07.duckdb",
            "gen-1",
            "gen-1.",
            "gen-x.duckdb",
            "gen--1.duckdb",
            "gen-99999999999999999999.duckdb",
            "generation-1.duckdb",
        ] {
            assert_eq!(parse(name), None, "{name}");
        }
    }

    #[test]
    fn numbers_sort_numerically_not_by_name() {
        let mut names = ["gen-10.duckdb", "gen-9.duckdb", "gen-100.duckdb"];
        names.sort_by_key(|name| parse(name).map(|(n, _)| n));
        assert_eq!(names, ["gen-9.duckdb", "gen-10.duckdb", "gen-100.duckdb"]);
    }
}
