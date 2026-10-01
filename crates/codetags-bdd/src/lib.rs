//! Cucumber world and step definitions for `features/` (test support only).
//!
//! The step vocabulary is frozen in `docs/steps.md` (PLAN.md §0.3); a step
//! added here without a matching entry there is a contract violation.
//!
//! Steps fail by panicking, as cucumber expects, so `expect`/`unwrap` are the
//! assertion mechanism in this crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

mod child;
pub mod ran;
mod steps_claude;
mod steps_cli;
mod steps_index;
mod steps_index_go;
mod steps_index_python;
mod steps_index_ts;
mod steps_lsp;
mod steps_lspmux;
mod steps_model;
mod steps_mount;
mod steps_names;
mod steps_privhelper;
mod steps_query;
mod steps_report;
mod steps_store;
mod steps_watch;
mod tags;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::OnceLock;

use cucumber::World;

pub use child::maybe_run_child;
pub use ran::RAN_ENV;
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
    /// A live mount. Declared before `scratch` so it is unmounted before the
    /// scratch directory holding its mount point is deleted.
    #[cfg(target_os = "linux")]
    mount: Option<codetags_mount_fuse::spike::Mounted>,
    /// A live NFS loopback mount (macOS), declared before `scratch` for the
    /// same reason.
    #[cfg(target_os = "macos")]
    nfs_mount: Option<codetags_mount_nfs::mount::Mounted>,
    /// A live WinFsp mount, likewise declared before `scratch`.
    #[cfg(windows)]
    mount: Option<codetags_mount_winfsp::spike::Mounted>,
    /// Where the live mount is (or was).
    mount_dir: Option<PathBuf>,
    /// The last process a step ran.
    last: Option<CommandOutcome>,
    /// Environment overrides for processes the scenario runs.
    env: Vec<(OsString, OsString)>,
    /// Environment variables removed for processes the scenario runs.
    env_removed: Vec<OsString>,
    /// Scratch directory, deleted when the scenario ends.
    scratch: Option<tempfile::TempDir>,
    /// The DuckDB file a `Given` step created.
    database: Option<PathBuf>,
    /// Items the tagma steps added.
    tagma: tagma_core::Index,
    /// Result of the last tagma query.
    matched: Option<Result<Vec<String>, String>>,
    /// The generation reader a store step opened.
    reader: Option<codetags_model::GenerationReader>,
    /// The generation the reader served before it last switched.
    held: Option<std::sync::Arc<codetags_model::Generation>>,
    /// A generation writer this process holds open, with its write lock.
    writer: Option<codetags_model::GenerationWriter>,
    // P1.2 (codetags-names): state for features/names/.
    /// Path-profile, item-id and collision-suffix state.
    names: steps_names::NamesState,
    // P1.4 (codetags-ingest): state for features/index/.
    /// The last SCIP provider run.
    index: steps_index::IndexState,
    // P4.1, P4.2 (codetags-watch): state for features/watch/.
    /// The coalescer, the watcher, and its project directory.
    watch: steps_watch::WatchState,
    // P4.4 (codetags-privhelper): state for features/watch/privhelper.feature.
    /// The helper started with sudo, and a client of it. Declared after
    /// `watch`, so the watcher stops before the helper does.
    #[cfg(unix)]
    privhelper: steps_privhelper::HelperState,
    // M3 stage 0 (codetags-lsp): state for features/lsp/.
    /// The fake language server and what the recorder passed through.
    lsp: steps_lsp::LspState,
    // M3 stage 0 (codetags-lsp): state for features/lsp/claude-client*.
    /// The scenario's plan, its Claude Code session, and the analysis.
    claude: steps_claude::ClaudeState,
    // M3 stage 1 (codetags lsp setup): state for features/lsp/setup.feature.
    /// The isolated home and its lspmux daemons.
    lspmux: steps_lspmux::LspmuxState,
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
///
/// When [`RAN_ENV`] is set, each scenario that runs is recorded in the file it
/// names, for CI's cross-job coverage check (PLAN.md §0.11, [`ran`]).
pub async fn run(features: impl AsRef<Path>, codetags: PathBuf) {
    CODETAGS_BIN
        .set(codetags)
        .expect("run() is called once per process");
    let features = features.as_ref().to_path_buf();
    let record = std::env::var_os(RAN_ENV).map(PathBuf::from);
    let root = features.clone();
    CodetagsWorld::cucumber()
        .fail_on_skipped()
        // The Claude Code steps share a live session between scenarios with
        // the same plan, so each needs its scenario's plan up front.
        .before(|feature, rule, scenario, world| {
            steps_claude::plan(world, feature, rule, scenario);
            Box::pin(std::future::ready(()))
        })
        .after(move |feature, _rule, scenario, _finished, _world| {
            if let Some(record) = &record {
                let path = feature
                    .path
                    .as_deref()
                    .expect("cucumber parsed this feature from a file");
                let line = ran::ran_line(&root, path, &scenario.name).unwrap_or_else(|| {
                    panic!("{} is not under {}", path.display(), root.display())
                });
                ran::append(record, &line)
                    .unwrap_or_else(|error| panic!("append to {}: {error}", record.display()));
            }
            Box::pin(std::future::ready(()))
        })
        .filter_run_and_exit(features, tags::filter)
        .await;
}
