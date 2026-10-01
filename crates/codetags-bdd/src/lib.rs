//! Cucumber world and step definitions for `features/` (test support only).
//!
//! The step vocabulary is frozen in `docs/steps.md` (PLAN.md §0.3); a step
//! added here without a matching entry there is a contract violation.
//!
//! Steps fail by panicking, as cucumber expects, so `expect`/`unwrap` are the
//! assertion mechanism in this crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod child;
mod steps_cli;
mod steps_model;
mod steps_query;
mod tags;

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::OnceLock;

use cucumber::World;

pub use child::maybe_run_child;
pub use tags::{CAPABILITIES_ENV, scenario_enabled};

/// The `codetags` binary under test, set once by [`run`].
static CODETAGS_BIN: OnceLock<PathBuf> = OnceLock::new();

/// What the last process a scenario ran produced.
#[derive(Debug)]
pub struct CommandOutcome {
    /// Exit code; `None` if the process was killed by a signal.
    pub status: Option<i32>,
    /// Captured stdout, lossily decoded as UTF-8.
    pub stdout: String,
    /// Captured stderr, lossily decoded as UTF-8.
    pub stderr: String,
}

impl From<Output> for CommandOutcome {
    fn from(output: Output) -> Self {
        Self {
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
}

/// Per-scenario state. Cucumber builds a fresh one for every scenario.
#[derive(Debug, Default, World)]
pub struct CodetagsWorld {
    /// The last process a step ran.
    last: Option<CommandOutcome>,
    /// Scratch directory, deleted when the scenario ends.
    scratch: Option<tempfile::TempDir>,
    /// The DuckDB file a `Given` step created.
    database: Option<PathBuf>,
    /// Items the tagma steps added.
    tagma: tagma_core::Index,
    /// Result of the last tagma query.
    matched: Option<Result<Vec<String>, String>>,
}

impl CodetagsWorld {
    fn last(&self) -> &CommandOutcome {
        self.last.as_ref().expect("an earlier step ran a process")
    }

    fn scratch(&mut self) -> &Path {
        self.scratch
            .get_or_insert_with(|| tempfile::tempdir().expect("create scratch dir"))
            .path()
    }
}

/// Runs every feature under `features` against the `codetags` binary at
/// `codetags`, then exits the process with cucumber's verdict. Skipped steps
/// fail the run (PLAN.md §0.11).
///
/// Call [`maybe_run_child`] first: steps that need "another process" re-run
/// the current executable in child mode.
pub async fn run(features: impl AsRef<Path>, codetags: PathBuf) {
    CODETAGS_BIN
        .set(codetags)
        .expect("run() is called once per process");
    CodetagsWorld::cucumber()
        .fail_on_skipped()
        .filter_run_and_exit(features.as_ref().to_path_buf(), tags::filter)
        .await;
}
