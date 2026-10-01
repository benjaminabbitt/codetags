//! Cucumber world and step definitions for `features/` (test support only).
//!
//! The step vocabulary is frozen in `docs/steps.md` (PLAN.md §0.3); a step
//! added here without a matching entry there is a contract violation.
//!
//! Steps fail by panicking, as cucumber expects, so `expect`/`unwrap` are the
//! assertion mechanism in this crate.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use cucumber::gherkin::{Feature, Rule, Scenario};
use cucumber::{World, then, when};

/// The `codetags` binary under test, set once by [`run`].
static CODETAGS_BIN: OnceLock<PathBuf> = OnceLock::new();

/// Scenario tags naming an operating system (PLAN.md §3).
const PLATFORM_TAGS: [&str; 3] = ["linux", "macos", "windows"];

/// Scenario tags naming a capability the job must opt into (PLAN.md §3).
const CAPABILITY_TAGS: [&str; 5] = ["mount", "winfsp", "providers", "privileged", "slow"];

/// Environment variable listing the capabilities a job provides, comma-separated.
pub const CAPABILITIES_ENV: &str = "CODETAGS_BDD_CAPABILITIES";

/// What the last command a scenario ran produced.
#[derive(Debug)]
pub struct CommandOutcome {
    /// Exit code; `None` if the process was killed by a signal.
    pub status: Option<i32>,
    /// Captured stdout, lossily decoded as UTF-8.
    pub stdout: String,
    /// Captured stderr, lossily decoded as UTF-8.
    pub stderr: String,
}

/// Per-scenario state. Cucumber builds a fresh one for every scenario.
#[derive(Debug, Default, World)]
pub struct CodetagsWorld {
    last: Option<CommandOutcome>,
}

impl CodetagsWorld {
    fn last(&self) -> &CommandOutcome {
        self.last.as_ref().expect("an earlier step ran a command")
    }
}

#[when(expr = "codetags is run with {string}")]
fn codetags_is_run_with(world: &mut CodetagsWorld, args: String) {
    let binary = CODETAGS_BIN.get().expect("run() sets the binary path");
    let output = Command::new(binary)
        .args(args.split_whitespace())
        .output()
        .expect("spawn codetags");
    world.last = Some(CommandOutcome {
        status: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    });
}

#[then(expr = "it exits with status {int}")]
fn it_exits_with_status(world: &mut CodetagsWorld, code: i32) {
    let last = world.last();
    assert_eq!(last.status, Some(code), "stderr was: {}", last.stderr);
}

#[then(expr = "stdout matches {string}")]
fn stdout_matches(world: &mut CodetagsWorld, pattern: String) {
    let regex = regex::Regex::new(&pattern).expect("the step's pattern is a valid regex");
    let stdout = &world.last().stdout;
    assert!(
        regex.is_match(stdout),
        "stdout {stdout:?} does not match {pattern:?}"
    );
}

/// Whether a scenario carrying `tags` runs on `os` with `capabilities`.
///
/// Platform tags restrict a scenario to the named OSes; with none it runs on
/// every OS. Each capability tag must appear in `capabilities`.
pub fn scenario_enabled(tags: &[String], os: &str, capabilities: &[String]) -> bool {
    let platforms: Vec<&str> = tags
        .iter()
        .map(String::as_str)
        .filter(|tag| PLATFORM_TAGS.contains(tag))
        .collect();
    let platform_ok = platforms.is_empty() || platforms.contains(&os);
    let capabilities_ok = tags
        .iter()
        .filter(|tag| CAPABILITY_TAGS.contains(&tag.as_str()))
        .all(|tag| capabilities.contains(tag));
    platform_ok && capabilities_ok
}

fn capabilities_from_env() -> Vec<String> {
    std::env::var(CAPABILITIES_ENV)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|cap| !cap.is_empty())
        .map(str::to_string)
        .collect()
}

fn filter(feature: &Feature, rule: Option<&Rule>, scenario: &Scenario) -> bool {
    let tags: Vec<String> = feature
        .tags
        .iter()
        .chain(rule.map(|rule| rule.tags.iter()).into_iter().flatten())
        .chain(scenario.tags.iter())
        .cloned()
        .collect();
    let enabled = scenario_enabled(&tags, std::env::consts::OS, &capabilities_from_env());
    if !enabled {
        eprintln!(
            "bdd: not run here: {}: {} (tags: {})",
            feature.name,
            scenario.name,
            tags.join(" ")
        );
    }
    enabled
}

/// Runs every feature under `features` against the `codetags` binary at
/// `codetags`, then exits the process with cucumber's verdict. Skipped steps
/// fail the run (PLAN.md §0.11).
pub async fn run(features: impl AsRef<Path>, codetags: PathBuf) {
    CODETAGS_BIN
        .set(codetags)
        .expect("run() is called once per process");
    CodetagsWorld::cucumber()
        .fail_on_skipped()
        .filter_run_and_exit(features.as_ref().to_path_buf(), filter)
        .await;
}

#[cfg(test)]
mod tests {
    use super::scenario_enabled;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn untagged_scenarios_run_everywhere() {
        assert!(scenario_enabled(&[], "windows", &[]));
    }

    #[test]
    fn platform_tags_restrict_to_the_named_oses() {
        let tags = strings(&["linux", "macos"]);
        assert!(scenario_enabled(&tags, "macos", &[]));
        assert!(!scenario_enabled(&tags, "windows", &[]));
    }

    #[test]
    fn capability_tags_need_every_capability() {
        let tags = strings(&["mount", "slow"]);
        assert!(!scenario_enabled(&tags, "linux", &strings(&["mount"])));
        assert!(scenario_enabled(
            &tags,
            "linux",
            &strings(&["slow", "mount"])
        ));
    }

    #[test]
    fn unrelated_tags_do_not_gate() {
        assert!(scenario_enabled(&strings(&["wip-note"]), "linux", &[]));
    }
}
