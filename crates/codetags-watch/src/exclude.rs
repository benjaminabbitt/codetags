//! Which project paths the watcher reports (brief §4.3, PLAN.md §2.3).

use std::ffi::OsStr;
use std::path::{Component, Path};

/// Directory names excluded at any depth: build and dependency output
/// (brief §4.3), plus `.git`, of which only [`INDEX_LOCK`] is watched.
pub const EXCLUDED_NAMES: [&str; 6] = [
    "target",
    "node_modules",
    "dist",
    "__pycache__",
    ".venv",
    ".git",
];

/// git's index lock, relative to the project root. Watched only to pause the
/// coalescer; it is never reported as a change.
pub const INDEX_LOCK: [&str; 2] = [".git", "index.lock"];

/// codetags' own index, relative to the project root (PLAN.md §2.3).
pub const CODETAGS_INDEX: [&str; 2] = [".codetags", "index"];

/// How the watcher treats a project-relative path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathClass {
    /// Reported in batches.
    Watched,
    /// Never reported.
    Excluded,
    /// `.git/index.lock`: pauses the coalescer while it exists.
    IndexLock,
}

/// Classifies `rel`, a path relative to the project root.
///
/// A path is excluded when any of its components is one of
/// [`EXCLUDED_NAMES`], or when it is [`CODETAGS_INDEX`] or under it. This
/// looks at names only, not at whether the path is a directory, because a
/// deleted path can no longer be examined: a *file* named `dist` is excluded
/// too.
pub fn classify(rel: &Path) -> PathClass {
    let names: Vec<&OsStr> = rel
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    if names == INDEX_LOCK.map(OsStr::new) {
        return PathClass::IndexLock;
    }
    if names.starts_with(&CODETAGS_INDEX.map(OsStr::new)) {
        return PathClass::Excluded;
    }
    if names.iter().any(|name| {
        EXCLUDED_NAMES
            .iter()
            .any(|excluded| *name == OsStr::new(excluded))
    }) {
        return PathClass::Excluded;
    }
    PathClass::Watched
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(path: &str) -> PathClass {
        classify(Path::new(path))
    }

    #[test]
    fn ordinary_paths_are_watched() {
        for path in [
            "src/lib.rs",
            "README.md",
            ".codetags/tags",
            ".codetags/config.toml",
            "src/targets/x.rs",
            "distribution.md",
            ".gitignore",
        ] {
            assert_eq!(class(path), PathClass::Watched, "{path}");
        }
    }

    #[test]
    fn build_and_dependency_output_is_excluded_at_any_depth() {
        for path in [
            "target",
            "target/debug/app",
            "crates/x/target/debug/app",
            "node_modules/x/index.js",
            "web/node_modules/y",
            "dist/bundle.js",
            "pkg/__pycache__/m.pyc",
            ".venv/lib/site.py",
        ] {
            assert_eq!(class(path), PathClass::Excluded, "{path}");
        }
    }

    #[test]
    fn git_internals_are_excluded_except_the_index_lock() {
        assert_eq!(class(".git"), PathClass::Excluded);
        assert_eq!(class(".git/HEAD"), PathClass::Excluded);
        assert_eq!(class(".git/objects/ab/cdef"), PathClass::Excluded);
        assert_eq!(class("vendor/sub/.git/index.lock"), PathClass::Excluded);
        assert_eq!(class(".git/index.lock"), PathClass::IndexLock);
    }

    #[test]
    fn only_the_codetags_index_is_excluded_from_codetags_data() {
        assert_eq!(class(".codetags/index"), PathClass::Excluded);
        assert_eq!(class(".codetags/index/gen-1.duckdb"), PathClass::Excluded);
        assert_eq!(class("sub/.codetags/index/x"), PathClass::Watched);
    }
}
