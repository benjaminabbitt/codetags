//! The record of scenarios that ran, for cross-job coverage (PLAN.md §0.11).
//!
//! When [`RAN_ENV`] names a file, the runner appends one line per scenario it
//! runs: `<feature path relative to features/> :: <scenario name>`, with `/`
//! separators on every OS. CI uploads the file from every job that runs BDD,
//! and `ci/bdd-coverage.py` fails if a scenario ran in no job.

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

/// Environment variable naming the file to append the record to.
pub const RAN_ENV: &str = "CODETAGS_BDD_RAN";

/// Serializes appends from scenarios that finish concurrently.
static APPEND: Mutex<()> = Mutex::new(());

/// The record line for `scenario` in the feature file `feature`, which lies
/// under `features`. `None` if `feature` is not under `features`.
/// Paths are compared as given first, then canonicalized, so `a/../features`
/// and `features` agree.
pub fn ran_line(features: &Path, feature: &Path, scenario: &str) -> Option<String> {
    let relative: PathBuf = match feature.strip_prefix(features) {
        Ok(relative) => relative.to_path_buf(),
        Err(_) => {
            let features = features.canonicalize().ok()?;
            let feature = feature.canonicalize().ok()?;
            feature.strip_prefix(&features).ok()?.to_path_buf()
        }
    };
    let parts: Vec<String> = relative
        .components()
        .map(|part| match part {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Option<_>>()?;
    Some(format!("{} :: {scenario}", parts.join("/")))
}

/// Appends `line` and a newline to `file`, creating it if needed.
pub fn append(file: &Path, line: &str) -> std::io::Result<()> {
    let _guard = APPEND
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?;
    out.write_all(format!("{line}\n").as_bytes())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{append, ran_line};

    #[test]
    fn the_line_is_the_relative_path_and_the_scenario_name() {
        let features = Path::new("repo").join("features");
        let feature = features.join("mount").join("linux_fuse.feature");
        assert_eq!(
            ran_line(&features, &feature, "Mount, list, read").as_deref(),
            Some("mount/linux_fuse.feature :: Mount, list, read")
        );
    }

    #[test]
    fn spellings_of_the_same_directory_agree() {
        let root = tempfile::tempdir().unwrap();
        let features = root.path().join("features");
        std::fs::create_dir_all(features.join("cli")).unwrap();
        let feature = features.join("cli").join("version.feature");
        std::fs::write(&feature, "").unwrap();
        let dotted = root.path().join("features").join("..").join("features");
        assert_eq!(
            ran_line(&dotted, &feature, "x").as_deref(),
            Some("cli/version.feature :: x")
        );
    }

    #[test]
    fn a_feature_outside_the_directory_has_no_line() {
        assert_eq!(
            ran_line(Path::new("features"), Path::new("elsewhere/a.feature"), "x"),
            None
        );
    }

    #[test]
    fn append_adds_one_line_per_call() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ran.txt");
        append(&file, "a.feature :: one").unwrap();
        append(&file, "a.feature :: two").unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "a.feature :: one\na.feature :: two\n"
        );
    }
}
