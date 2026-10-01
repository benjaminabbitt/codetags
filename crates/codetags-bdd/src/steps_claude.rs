//! Steps for `features/lsp/claude-client*.feature`: Claude Code's LSP client,
//! observed through `codetags-lsp record` (M3 stage 0, PLAN.md D16-D18).
//!
//! The live feature (`@claude`) runs `claude -p` headless in a scratch copy of
//! a fixture, with the recorder plugin loaded through `--plugin-dir`. One
//! session takes the scenario's actions as successive user turns
//! (`--input-format stream-json`), and each action records a step mark, so
//! `codetags-lsp analyze --marks` can say what the client sent during it.
//!
//! A live session costs money and minutes, so scenarios whose Background and
//! `When` steps are identical share one session per test run: the first
//! scenario runs it as its `When` steps execute, and the others replay its
//! outcome ([`sessions`]). The `before` hook gives each scenario its plan
//! ([`plan`]), which is how the first action knows whether to run or replay.
//!
//! The recordings are sanitized ([`sanitize`]) before they are analyzed, so
//! the live and the replayed analyses read the same: the project directory is
//! `/project` and the home directory `/home/user`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cucumber::gherkin::{Feature, Rule, Scenario, StepType};
use cucumber::{given, then, when};
use serde_json::{Value, json};

use crate::CodetagsWorld;

/// The model every live session uses: the cheapest that drives the tools.
const MODEL: &str = "claude-haiku-4-5-20251001";
/// How long one action (one user turn) may take.
const ACTION_TIMEOUT: Duration = Duration::from_secs(300);
/// How long Claude Code may take to exit once its input is closed.
const EXIT_TIMEOUT: Duration = Duration::from_secs(120);
/// Spending cap for one session (`--max-budget-usd`).
const BUDGET_USD: &str = "1";
/// The sanitized project directory, and home directory.
const PROJECT: &str = "/project";
const HOME: &str = "/home/user";
/// If set, a directory each live session's sanitized recordings and marks
/// are saved under, as `claude-code-<version>/<rule>.jsonl` (the replay
/// fixtures in `tests/fixtures/lsp` are made this way).
const SAVE_ENV: &str = "CODETAGS_BDD_CLAUDE_SAVE";
/// Appended to Claude Code's system prompt, to keep Haiku to the script.
const SYSTEM_PROMPT: &str = "This is an automated test of the LSP tool. Do exactly what each message asks, using only the tools it names, each once unless it says otherwise, and nothing else. Do not read, search, or change any other file.";

/// One scripted action: a user turn and the step mark it records.
#[derive(Debug)]
struct Action {
    /// The step mark, named as the `Then` steps name it ("Edit", "sed").
    label: String,
    prompt: String,
    /// Tools Claude must have used in the turn, or the action failed.
    tools: &'static [&'static str],
}

/// What a finished live session left behind, shared between scenarios.
#[derive(Debug)]
struct Outcome {
    version: String,
    plugins: Vec<String>,
    /// Sanitized recordings, one per server process, in file-name order.
    /// Through the shim, one per shim process: the client's side.
    recordings: Vec<String>,
    /// Through the shim: sanitized recordings between lspmux and
    /// rust-analyzer, one per server process (`server-*.jsonl`).
    server_recordings: Vec<String>,
    /// `codetags-lsp analyze --marks` of the first recording.
    analysis: Option<String>,
    /// The session's step marks, in order (`{"step", "ts_ms"}`).
    marks: Vec<Value>,
    /// The tail of Claude Code's stderr and stream, for failure messages.
    diagnostics: String,
}

type Shared = Arc<Result<Outcome, String>>;

/// Finished sessions, by plan.
fn sessions() -> &'static Mutex<HashMap<Vec<String>, Shared>> {
    static SESSIONS: OnceLock<Mutex<HashMap<Vec<String>, Shared>>> = OnceLock::new();
    SESSIONS.get_or_init(Mutex::default)
}

/// A running `claude -p` and what it has said so far.
#[derive(Debug)]
struct Live {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    events: Vec<Value>,
    marks: Vec<Value>,
    project: PathBuf,
    stderr: PathBuf,
    /// Through the shim: the lspmux daemon's log, naming the daemons the
    /// shim started, which [`Live::finish`] kills.
    daemon_log: Option<PathBuf>,
}

impl Drop for Live {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// State for the Claude Code features.
#[derive(Debug, Default)]
pub(crate) struct ClaudeState {
    /// The scenario's non-`Then` steps, from the `before` hook: the key a
    /// session is shared by.
    plan: Vec<String>,
    /// How many of the plan's steps are actions.
    actions: usize,
    /// The rule's name, which names saved recordings.
    rule: String,
    project: Option<PathBuf>,
    /// Actions done so far in this scenario.
    done: usize,
    live: Option<Live>,
    outcome: Option<Shared>,
    /// The analysis the `Then` steps read: the session's, or a replay's.
    analysis: Option<String>,
    /// A recorded session's log and step marks, when a scenario analyzes
    /// one from `tests/fixtures/lsp` instead of running a live session.
    recorded: Option<(String, Vec<Value>)>,
    /// Through the shim (M3): the plugin to load instead of the recorder,
    /// the environment Claude Code passes it, and the daemon's log.
    shim: Option<ShimSetup>,
}

/// The live test's shim setup: the codetags-lsp plugin, a separate lspmux
/// config and daemon, and session logs on both sides of lspmux.
#[derive(Debug)]
struct ShimSetup {
    plugin: PathBuf,
    env: Vec<(String, PathBuf)>,
    daemon_log: PathBuf,
    /// Holds the socket's directory until the scenario ends.
    _runtime: tempfile::TempDir,
}

/// Whether `text` is a step that drives Claude Code.
fn is_action(text: &str) -> bool {
    text.starts_with("Claude Code ")
}

/// The `before` hook: records the scenario's plan.
pub(crate) fn plan(
    world: &mut CodetagsWorld,
    feature: &Feature,
    rule: Option<&Rule>,
    scenario: &Scenario,
) {
    let steps = feature
        .background
        .iter()
        .flat_map(|background| &background.steps)
        .chain(
            rule.and_then(|rule| rule.background.as_ref())
                .into_iter()
                .flat_map(|background| &background.steps),
        )
        .chain(&scenario.steps)
        .filter(|step| step.ty != StepType::Then);
    let state = &mut world.claude;
    state.plan = steps.map(|step| step.value.clone()).collect();
    state.actions = state.plan.iter().filter(|text| is_action(text)).count();
    state.rule = rule.map(|rule| rule.name.clone()).unwrap_or_default();
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/codetags-bdd sits two levels below the repo root")
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

pub(crate) fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap_or_else(|e| panic!("create {}: {e}", to.display()));
    for entry in std::fs::read_dir(from).unwrap_or_else(|e| panic!("list {}: {e}", from.display()))
    {
        let entry = entry.expect("read a directory entry");
        let name = entry.file_name();
        if name == "target" {
            continue;
        }
        let (source, target) = (entry.path(), to.join(&name));
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&source, &target);
        } else {
            std::fs::copy(&source, &target)
                .unwrap_or_else(|e| panic!("copy {}: {e}", source.display()));
        }
    }
}

fn run_git(project: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=codetags",
            "-c",
            "user.email=codetags@invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(project)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[given(expr = "a scratch Rust project from the fixture {string} with the LSP recorder plugin")]
fn a_scratch_project(world: &mut CodetagsWorld, fixture: String) {
    let project = world.scratch().join("project");
    copy_tree(
        &repo_root().join("tests").join("fixtures").join(&fixture),
        &project,
    );
    std::fs::write(project.join(".gitignore"), "/target/\n/.codetags/\n")
        .expect("write .gitignore");
    run_git(&project, &["init", "-q"]);
    run_git(&project, &["add", "-A"]);
    run_git(&project, &["commit", "-q", "-m", "fixture"]);
    // The wrapper's setup, in the project (the recorder plugin's wrapper
    // prefers it to the checkout's).
    let local = project.join(".codetags").join("local");
    std::fs::create_dir_all(local.join("bin")).expect("create .codetags/local/bin");
    std::fs::copy(
        crate::steps_lsp::lsp_binary(),
        local.join("bin").join("codetags-lsp"),
    )
    .expect("copy codetags-lsp into the project");
    let rustup = Command::new("rustup")
        .args(["which", "rust-analyzer"])
        .current_dir(repo_root())
        .output()
        .expect("run rustup (it must be on PATH)");
    assert!(
        rustup.status.success(),
        "no rust-analyzer for this toolchain; add it with: rustup component add rust-analyzer\n{}",
        String::from_utf8_lossy(&rustup.stderr)
    );
    std::fs::write(local.join("rust-analyzer.path"), &rustup.stdout)
        .expect("write rust-analyzer.path");
    world.claude.project = Some(project);
}

/// Every `Bash` command the plan's actions run, for `--allowedTools`.
fn bash_commands(plan: &[String]) -> Vec<String> {
    let pattern =
        regex::Regex::new(r#"^Claude Code runs "(.*)" through Bash"#).expect("valid regex");
    plan.iter()
        .filter_map(|text| pattern.captures(text))
        .flat_map(|captures| {
            captures[1]
                .split("&&")
                .map(|command| command.trim().to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Starts `claude -p` in the project, headless, with the recorder plugin.
fn start(state: &ClaudeState) -> Live {
    let project = state
        .project
        .clone()
        .expect("the Given step made the scratch project");
    let plugin = state.shim.as_ref().map_or_else(
        || {
            repo_root()
                .join("tools")
                .join("claude-plugins")
                .join("codetags-lsp-recorder")
        },
        |shim| shim.plugin.clone(),
    );
    let stderr = project.with_file_name("claude-stderr.txt");
    let mut allowed: Vec<String> = ["Read", "Edit", "Write", "LSP"].map(String::from).to_vec();
    allowed.push(format!("Bash({SETTLE})"));
    allowed.extend(
        bash_commands(&state.plan)
            .iter()
            .map(|command| format!("Bash({command})")),
    );
    let mut command = Command::new("claude");
    command
        .args(["-p", "--model", MODEL])
        .args([
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
        ])
        .arg("--plugin-dir")
        .arg(&plugin)
        // Only the project's settings (it has none): no user plugins, hooks,
        // or permissions. The official rust-analyzer plugin is off besides.
        .args(["--setting-sources", "project"])
        .args([
            "--settings",
            r#"{"enabledPlugins":{"rust-analyzer-lsp@claude-plugins-official":false}}"#,
        ])
        .arg("--strict-mcp-config")
        .args(["--tools", "Read,Edit,Write,Bash,LSP"])
        .arg("--allowedTools")
        .args(&allowed)
        // Anything not allowed above is denied, never prompted for.
        .args(["--permission-mode", "dontAsk"])
        .args(["--max-turns", "8", "--max-budget-usd", BUDGET_USD])
        .arg("--no-session-persistence")
        .args(["--append-system-prompt", SYSTEM_PROMPT])
        .current_dir(&project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(&stderr).expect("create the stderr file"));
    // The calling Claude Code session's variables (when this runs inside
    // one): none stops `claude -p` from starting (V112), but they tie the
    // child to the parent's session, so all go, except configuration and
    // credentials.
    for (name, _) in std::env::vars_os() {
        let name_text = name.to_string_lossy();
        let keep = matches!(
            name_text.as_ref(),
            "CLAUDE_CONFIG_DIR" | "CLAUDE_CODE_OAUTH_TOKEN"
        );
        if (name_text.starts_with("CLAUDE") || name_text == "AI_AGENT") && !keep {
            command.env_remove(&name);
        }
    }
    for (name, value) in state.shim.iter().flat_map(|shim| &shim.env) {
        command.env(name, value);
    }
    let mut child = command
        .spawn()
        .unwrap_or_else(|e| panic!("start claude (is it on PATH?): {e}"));
    let stdout = child.stdout.take().expect("piped stdout");
    let (sender, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    Live {
        stdin: child.stdin.take(),
        child,
        lines,
        events: Vec::new(),
        marks: Vec::new(),
        project,
        stderr,
        daemon_log: state.shim.as_ref().map(|shim| shim.daemon_log.clone()),
    }
}

impl Live {
    /// Stderr and the last stream events, for a failure message.
    fn diagnostics(&self) -> String {
        let stderr = std::fs::read_to_string(&self.stderr).unwrap_or_default();
        let tail: Vec<String> = self
            .events
            .iter()
            .rev()
            .take(8)
            .rev()
            .map(|event| {
                let text = event.to_string();
                text.chars().take(600).collect()
            })
            .collect();
        format!(
            "claude stderr:\n{stderr}\nlast stream events:\n{}",
            tail.join("\n")
        )
    }

    /// Sends `action` as a user turn and waits for its result.
    fn act(&mut self, action: &Action) -> Result<(), String> {
        self.marks
            .push(json!({"ts_ms": unix_ms(), "step": action.label}));
        let message = json!({
            "type": "user",
            "message": {"role": "user", "content": action.prompt},
        });
        let stdin = self.stdin.as_mut().ok_or("claude's input is closed")?;
        writeln!(stdin, "{message}")
            .and_then(|()| stdin.flush())
            .map_err(|e| format!("write to claude: {e}"))?;
        let deadline = Instant::now() + ACTION_TIMEOUT;
        let mut used: Vec<String> = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = match self.lines.recv_timeout(left) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("{:?} took over {ACTION_TIMEOUT:?}", action.label));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(format!("claude exited during {:?}", action.label));
                }
            };
            let Ok(event) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if event["type"] == "assistant" {
                for block in event["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_use" {
                        used.push(block["name"].as_str().unwrap_or("").to_string());
                    }
                }
            }
            let result = event["type"] == "result";
            let failed = result && (event["is_error"] == true || event["subtype"] != "success");
            self.events.push(event);
            if failed {
                return Err(format!("{:?} ended in an error result", action.label));
            }
            if result {
                break;
            }
        }
        // A denied tool call means the turn did not do what the step says
        // (a change that never happened would read as a stale server).
        let denials = &self.events.last().unwrap_or(&Value::Null)["permission_denials"];
        if denials.as_array().is_some_and(|denied| !denied.is_empty()) {
            return Err(format!(
                "{:?}: Claude Code denied {denials} (used {used:?})",
                action.label
            ));
        }
        let missing: Vec<&str> = action
            .tools
            .iter()
            .copied()
            .filter(|tool| !used.iter().any(|name| name == tool))
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            // The turn's result lists every tool call the permission rules
            // denied, in full (the diagnostics' event tails are cut short).
            let denials = self
                .events
                .last()
                .map(|result| result["permission_denials"].to_string())
                .unwrap_or_default();
            Err(format!(
                "{:?}: Claude did not use {missing:?} (used {used:?}); permission denials: {denials}",
                action.label
            ))
        }
    }

    /// Closes the input, waits for Claude Code to exit, and collects the
    /// sanitized recordings and their analysis.
    fn finish(mut self, rule: &str) -> Result<Outcome, String> {
        drop(self.stdin.take());
        let deadline = Instant::now() + EXIT_TIMEOUT;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(200));
                }
                Ok(None) => return Err(format!("claude did not exit within {EXIT_TIMEOUT:?}")),
                Err(e) => return Err(format!("wait for claude: {e}")),
            }
        }
        // Through the shim, the daemon and its rust-analyzer outlive Claude
        // Code by design (D21): stop the ones this session started.
        if let Some(log) = &self.daemon_log {
            for pid in crate::steps_lspmux::started_daemons(log) {
                crate::steps_lspmux::kill_tree(pid);
            }
        }
        while let Ok(line) = self.lines.recv_timeout(Duration::from_secs(1)) {
            if let Ok(event) = serde_json::from_str::<Value>(&line) {
                self.events.push(event);
            }
        }
        let init = self
            .events
            .iter()
            .find(|event| event["type"] == "system" && event["subtype"] == "init")
            .cloned()
            .unwrap_or(Value::Null);
        let version = init["claude_code_version"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let plugins = init["plugins"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|plugin| plugin["name"].as_str().map(str::to_string))
            .collect();
        let local = self.project.join(".codetags").join("local");
        let recordings = self.recordings(&local, "lsp-")?;
        let server_recordings = self.recordings(&local, "server-")?;
        let marks: String = self.marks.iter().map(|mark| format!("{mark}\n")).collect();
        let scratch = self
            .project
            .parent()
            .expect("the project is in the scratch directory")
            .to_path_buf();
        let analysis = match recordings.first() {
            None => None,
            Some(recording) => {
                let (log, marks_file) = (
                    scratch.join("session.jsonl"),
                    scratch.join("session.marks.jsonl"),
                );
                std::fs::write(&log, recording).map_err(|e| e.to_string())?;
                std::fs::write(&marks_file, &marks).map_err(|e| e.to_string())?;
                Some(run_analyze(&log, Some(&marks_file))?)
            }
        };
        if let Some(save) = std::env::var_os(SAVE_ENV) {
            let dir = PathBuf::from(save).join(format!("claude-code-{version}"));
            std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
            let stem = slug(rule);
            for (index, recording) in recordings.iter().enumerate() {
                let name = if index == 0 {
                    stem.clone()
                } else {
                    format!("{stem}-{}", index + 1)
                };
                std::fs::write(dir.join(format!("{name}.jsonl")), recording)
                    .map_err(|e| e.to_string())?;
            }
            std::fs::write(dir.join(format!("{stem}.marks.jsonl")), &marks)
                .map_err(|e| e.to_string())?;
        }
        Ok(Outcome {
            version,
            plugins,
            recordings,
            server_recordings,
            analysis,
            marks: self.marks.clone(),
            diagnostics: self.diagnostics(),
        })
    }
}

impl Live {
    /// The sanitized `<prefix>*.jsonl` logs in `local`, in file-name order.
    fn recordings(&self, local: &Path, prefix: &str) -> Result<Vec<String>, String> {
        let mut logs: Vec<PathBuf> = std::fs::read_dir(local)
            .map_err(|e| format!("list {}: {e}", local.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".jsonl"))
            })
            .collect();
        logs.sort();
        logs.iter()
            .map(|log| {
                let text = std::fs::read_to_string(log)
                    .map_err(|e| format!("read {}: {e}", log.display()))?;
                sanitize(&text, &self.project)
            })
            .collect()
    }
}

/// A file-name stem from a rule's name up to its first `,` or `:`:
/// lowercase words joined by `-`.
fn slug(text: &str) -> String {
    let head = text.split([',', ':']).next().unwrap_or(text);
    let words: Vec<String> = head
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if words.is_empty() {
        "session".to_string()
    } else {
        words.join("-")
    }
}

/// `text` with the project directory replaced by [`PROJECT`] and the home
/// directory by [`HOME`]. Fails if a credential would be left in it.
fn sanitize(text: &str, project: &Path) -> Result<String, String> {
    let mut text = text.to_string();
    let mut projects = vec![project.to_path_buf()];
    if let Ok(canonical) = project.canonicalize() {
        projects.insert(0, canonical);
    }
    for path in projects {
        text = text.replace(&*path.to_string_lossy(), PROJECT);
    }
    if let Some(home) = std::env::var_os("HOME").filter(|home| home.len() > 1) {
        text = text.replace(&*home.to_string_lossy(), HOME);
    }
    let secrets = [
        "ANTHROPIC_API_KEY",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_MESSAGING_TOKEN",
    ]
    .iter()
    .filter_map(|name| std::env::var(name).ok())
    .filter(|value| value.len() >= 8);
    for secret in secrets {
        if text.contains(&secret) {
            return Err("a recording holds a credential from the environment".to_string());
        }
    }
    if text.contains("sk-ant-") {
        return Err("a recording holds something shaped like an Anthropic key".to_string());
    }
    Ok(text)
}

/// `codetags-lsp analyze [--marks marks] log`; fails unless it exits 0.
fn run_analyze(log: &Path, marks: Option<&Path>) -> Result<String, String> {
    let mut command = Command::new(crate::steps_lsp::lsp_binary());
    command.arg("analyze").arg(log);
    if let Some(marks) = marks {
        command.arg("--marks").arg(marks);
    }
    let output = command
        .output()
        .map_err(|e| format!("run codetags-lsp analyze: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "codetags-lsp analyze {} failed: {}",
            log.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Runs `action` in this scenario's session: live, or replayed from an
/// earlier scenario with the same plan.
fn act(world: &mut CodetagsWorld, action: Action) {
    let state = &mut world.claude;
    assert!(
        state.actions > 0,
        "the before hook found no Claude Code actions in this scenario"
    );
    if state.done == 0 && state.live.is_none() && state.outcome.is_none() {
        let cached = sessions()
            .lock()
            .expect("session cache lock")
            .get(&state.plan)
            .cloned();
        match cached {
            Some(shared) => state.outcome = Some(shared),
            None => state.live = Some(start(state)),
        }
    }
    state.done += 1;
    if let Some(shared) = &state.outcome {
        match shared.as_ref() {
            Err(error) => panic!("this scenario's session failed in an earlier scenario: {error}"),
            Ok(outcome) => state.analysis.clone_from(&outcome.analysis),
        }
        return;
    }
    let live = state.live.as_mut().expect("a live session");
    let result = live.act(&action);
    let last = state.done == state.actions;
    let shared: Option<Shared> = match result {
        Err(error) => {
            let diagnostics = live.diagnostics();
            Some(Arc::new(Err(format!("{error}\n{diagnostics}"))))
        }
        Ok(()) if last => {
            let live = state.live.take().expect("a live session");
            Some(Arc::new(live.finish(&state.rule)))
        }
        Ok(()) => None,
    };
    if let Some(shared) = shared {
        state.live = None;
        sessions()
            .lock()
            .expect("session cache lock")
            .insert(state.plan.clone(), Arc::clone(&shared));
        state.outcome = Some(Arc::clone(&shared));
        match shared.as_ref() {
            Err(error) => panic!("Claude Code session failed: {error}"),
            Ok(outcome) => state.analysis.clone_from(&outcome.analysis),
        }
    }
}

const LSP: &[&str] = &["LSP"];

#[when(expr = "Claude Code reads {string} with the Read tool")]
fn reads(world: &mut CodetagsWorld, file: String) {
    act(
        world,
        Action {
            label: "Read".into(),
            prompt: format!("Read {file} with the Read tool and summarize it in one sentence."),
            tools: &["Read"],
        },
    );
}

#[when(expr = "Claude Code asks the LSP for the definition of {string} in {string}")]
fn definition(world: &mut CodetagsWorld, symbol: String, file: String) {
    act(
        world,
        Action {
            label: "definition".into(),
            prompt: format!(
                "Use the LSP tool (operation goToDefinition) on the use of {symbol} in {file}. Report the file and line it points to."
            ),
            tools: LSP,
        },
    );
}

#[when(expr = "Claude Code asks the LSP for references to {string} in {string}")]
fn references(world: &mut CodetagsWorld, symbol: String, file: String) {
    act(
        world,
        Action {
            label: "references".into(),
            prompt: format!(
                "Use the LSP tool (operation findReferences) on {symbol} where it is declared in {file}. List the files."
            ),
            tools: LSP,
        },
    );
}

#[when(expr = "Claude Code asks the LSP to hover on {string} in {string}")]
fn hover(world: &mut CodetagsWorld, symbol: String, file: String) {
    act(
        world,
        Action {
            label: "hover".into(),
            prompt: format!(
                "Use the LSP tool (operation hover) on {symbol} where it is declared in {file}. Quote the hover text."
            ),
            tools: LSP,
        },
    );
}

#[when(expr = "Claude Code asks the LSP for implementations of {string} in {string}")]
fn implementations(world: &mut CodetagsWorld, symbol: String, file: String) {
    act(
        world,
        Action {
            label: "implementations".into(),
            prompt: format!(
                "Use the LSP tool (operation goToImplementation) on {symbol} where it is declared in {file}. Report what it returns."
            ),
            tools: LSP,
        },
    );
}

#[when(expr = "Claude Code asks the LSP for the call hierarchy of {string} in {string}")]
fn call_hierarchy(world: &mut CodetagsWorld, symbol: String, file: String) {
    act(
        world,
        Action {
            label: "call hierarchy".into(),
            prompt: format!(
                "Use the LSP tool for the call hierarchy of {symbol} where it is declared in {file}: operation prepareCallHierarchy, then incomingCalls, then outgoingCalls. Report what each returns."
            ),
            tools: LSP,
        },
    );
}

#[when(
    expr = "Claude Code appends {string} to {string} with the Edit tool, then hovers on {string}"
)]
fn edits(world: &mut CodetagsWorld, line: String, file: String, symbol: String) {
    act(
        world,
        Action {
            label: "Edit".into(),
            prompt: format!(
                "With the Edit tool, add the line \"{line}\" at the end of {file}. Then use the LSP tool (operation hover) on {symbol} where it is declared in {file}. Quote the hover text."
            ),
            tools: &["Edit", "LSP"],
        },
    );
}

#[when(
    expr = "Claude Code creates {string} holding {string} with the Write tool, then lists its symbols"
)]
fn writes(world: &mut CodetagsWorld, file: String, content: String) {
    act(
        world,
        Action {
            label: "Write".into(),
            prompt: format!(
                "With the Write tool, create {file} containing exactly \"{content}\". Then use the LSP tool (operation documentSymbol) on {file}. List the symbols."
            ),
            tools: &["Write", "LSP"],
        },
    );
}

/// The step mark of a Bash command: its program, plus the subcommand for
/// git ("sed", "git checkout").
fn bash_label(command: &str) -> String {
    let mut words = command.split_whitespace();
    let program = words.next().unwrap_or("Bash").to_string();
    match (program.as_str(), words.next()) {
        ("git", Some(subcommand)) => format!("git {subcommand}"),
        _ => program,
    }
}

#[when(expr = "Claude Code runs {string} through Bash, then hovers on {string} in {string}")]
fn runs_bash(world: &mut CodetagsWorld, command: String, symbol: String, file: String) {
    act(
        world,
        Action {
            label: bash_label(&command),
            prompt: format!(
                "Run exactly this command with the Bash tool: {command}\nThen use the LSP tool (operation hover) on {symbol} where it is declared in {file}. Quote the hover text."
            ),
            tools: &["Bash", "LSP"],
        },
    );
}

/// The Bash command an external-change action runs between the change and
/// its LSP lookup: a bounded delay, since a server's own file watcher is
/// asynchronous. Every session allows it.
const SETTLE: &str = "sleep 2";

/// The LSP tool call that searches the workspace for `query`.
fn workspace_search(query: &str) -> String {
    format!(
        "use the LSP tool with operation workspaceSymbol, filePath src/lib.rs, line 1, character 1, and query \"{query}\". List the symbols it returns by name, or say it returned none."
    )
}

#[when(expr = "Claude Code searches the workspace for {string}")]
fn searches_workspace(world: &mut CodetagsWorld, query: String) {
    let search = workspace_search(&query);
    act(
        world,
        Action {
            label: "workspace search".into(),
            prompt: format!(
                "First, {search} The language server may still be indexing: while the search returns no symbols, run exactly `{SETTLE}` with the Bash tool and search again, at most 3 more times."
            ),
            tools: LSP,
        },
    );
}

#[when(expr = "Claude Code runs {string} through Bash, then searches the workspace for {string}")]
fn runs_bash_then_searches(world: &mut CodetagsWorld, command: String, query: String) {
    let search = workspace_search(&query);
    act(
        world,
        Action {
            label: bash_label(&command),
            prompt: format!(
                "Run exactly this command with the Bash tool: {command}\nThen run exactly `{SETTLE}` with the Bash tool, as a command of its own. Then {search}"
            ),
            tools: &["Bash", "LSP"],
        },
    );
}

#[when(expr = "Claude Code runs {string} through Bash, then lists the symbols of {string}")]
fn runs_bash_then_lists_symbols(world: &mut CodetagsWorld, command: String, file: String) {
    act(
        world,
        Action {
            label: bash_label(&command),
            prompt: format!(
                "Run exactly this command with the Bash tool: {command}\nThen run exactly `{SETTLE}` with the Bash tool, as a command of its own. Then use the LSP tool (operation documentSymbol) on {file}, line 1, character 1. List the symbols by name."
            ),
            tools: &["Bash", "LSP"],
        },
    );
}

#[when(expr = "codetags-lsp analyzes the recorded session {string}")]
fn analyzes_recorded(world: &mut CodetagsWorld, name: String) {
    let base = repo_root().join("tests").join("fixtures").join("lsp");
    let log = base.join(format!("{name}.jsonl"));
    let marks = base.join(format!("{name}.marks.jsonl"));
    let marks = marks.exists().then_some(marks);
    let analysis = run_analyze(&log, marks.as_deref()).unwrap_or_else(|error| panic!("{error}"));
    world.claude.analysis = Some(analysis);
    let text =
        std::fs::read_to_string(&log).unwrap_or_else(|e| panic!("read {}: {e}", log.display()));
    let marks = marks
        .map(|marks| {
            std::fs::read_to_string(&marks)
                .unwrap_or_else(|e| panic!("read {}: {e}", marks.display()))
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .collect()
        })
        .unwrap_or_default();
    world.claude.recorded = Some((text, marks));
}

/// The finished live session this scenario ran or replayed.
fn outcome(world: &CodetagsWorld) -> &Outcome {
    let shared = world
        .claude
        .outcome
        .as_ref()
        .expect("a Claude Code action ran this scenario's session");
    match shared.as_ref() {
        Ok(outcome) => outcome,
        Err(error) => panic!("the session failed: {error}"),
    }
}

/// The analysis the `Then` steps read.
fn analysis(world: &CodetagsWorld) -> &str {
    if let Some(analysis) = &world.claude.analysis {
        return analysis;
    }
    let outcome = outcome(world);
    panic!(
        "the session wrote no recording (Claude Code {}, plugins {:?})\n{}",
        outcome.version, outcome.plugins, outcome.diagnostics
    );
}

/// The rest of the analysis line starting with `prefix`.
fn line_after<'a>(analysis: &'a str, prefix: &str) -> Option<&'a str> {
    analysis.lines().find_map(|line| line.strip_prefix(prefix))
}

/// The JSON value of an `initialize` parameter in the analysis.
fn initialize_param(world: &CodetagsWorld, key: &str) -> Value {
    let analysis = analysis(world);
    let text = line_after(analysis, &format!("- `{key}`: "))
        .unwrap_or_else(|| panic!("no initialize {key} in the analysis:\n{analysis}"));
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{key} is not JSON ({e}): {text}"))
}

#[then(expr = "Claude Code loaded the plugin {string}")]
fn loaded_plugin(world: &mut CodetagsWorld, plugin: String) {
    let outcome = outcome(world);
    assert!(
        outcome.plugins.contains(&plugin),
        "Claude Code {} loaded only {:?}\n{}",
        outcome.version,
        outcome.plugins,
        outcome.diagnostics
    );
}

#[then(expr = "Claude Code did not load the plugin {string}")]
fn did_not_load_plugin(world: &mut CodetagsWorld, plugin: String) {
    let outcome = outcome(world);
    assert!(
        !outcome.plugins.contains(&plugin),
        "Claude Code {} loaded {:?}",
        outcome.version,
        outcome.plugins
    );
}

#[then(expr = "the recorder wrote {int} recording(s)")]
fn wrote_recordings(world: &mut CodetagsWorld, count: usize) {
    let outcome = outcome(world);
    assert_eq!(
        outcome.recordings.len(),
        count,
        "recordings of Claude Code {}\n{}",
        outcome.version,
        outcome.diagnostics
    );
}

#[then("the client's initialize rootUri is the project root")]
fn root_uri_is_project(world: &mut CodetagsWorld) {
    let root = initialize_param(world, "rootUri");
    assert_eq!(root, json!(format!("file://{PROJECT}")));
}

#[then("the client's initialize rootUri is null")]
fn root_uri_is_null(world: &mut CodetagsWorld) {
    assert_eq!(initialize_param(world, "rootUri"), Value::Null);
}

#[then("the client's initialize workspaceFolders are only the project root")]
fn workspace_folders(world: &mut CodetagsWorld) {
    let folders = initialize_param(world, "workspaceFolders");
    let uris: Vec<&Value> = folders
        .as_array()
        .unwrap_or_else(|| panic!("workspaceFolders is not an array: {folders}"))
        .iter()
        .map(|folder| &folder["uri"])
        .collect();
    assert_eq!(uris, vec![&json!(format!("file://{PROJECT}"))], "{folders}");
}

/// The client capability at the dotted `path`, from the full capabilities.
fn capability(world: &CodetagsWorld, path: &str) -> Option<Value> {
    let analysis = analysis(world);
    let start = analysis
        .find("Full client capabilities:\n\n```json\n")
        .unwrap_or_else(|| panic!("no client capabilities in the analysis:\n{analysis}"));
    let block = &analysis[start..];
    let json_start = block.find("```json\n").expect("the block opens") + "```json\n".len();
    let json_end = block[json_start..].find("\n```").expect("the block closes") + json_start;
    let capabilities: Value =
        serde_json::from_str(&block[json_start..json_end]).expect("the capabilities are JSON");
    path.split('.')
        .try_fold(&capabilities, |value, key| value.get(key))
        .cloned()
}

#[then(expr = "the client capability {string} is absent")]
fn capability_absent(world: &mut CodetagsWorld, path: String) {
    let value = capability(world, &path);
    assert!(value.is_none(), "{path} is {value:?}");
}

#[then(expr = "the client capability {string} is {string}")]
fn capability_is(world: &mut CodetagsWorld, path: String, expected: String) {
    let expected: Value = serde_json::from_str(&expected).expect("the expected value is JSON");
    assert_eq!(capability(world, &path), Some(expected), "{path}");
}

/// The client's distinct answers to the server's `method` requests.
fn answers(world: &CodetagsWorld, method: &str) -> Option<Vec<String>> {
    let analysis = analysis(world);
    let section = analysis
        .split("## Client answers by method\n")
        .nth(1)
        .unwrap_or_else(|| panic!("no answers section in the analysis:\n{analysis}"));
    let section = section.split("\n## ").next().unwrap_or(section);
    line_after(section, &format!("- `{method}`: "))
        .map(|answers| answers.split(" | ").map(str::to_string).collect())
}

#[then(expr = "the client answered {string} with error {int}")]
fn answered_error(world: &mut CodetagsWorld, method: String, code: i64) {
    let answers =
        answers(world, &method).unwrap_or_else(|| panic!("the server sent no {method} request"));
    let prefix = format!("error {code} ");
    assert!(
        answers.iter().all(|answer| answer.starts_with(&prefix)),
        "{method} answers: {answers:?}"
    );
}

#[then(expr = "the client answered {string} with the result {string}")]
fn answered_result(world: &mut CodetagsWorld, method: String, expected: String) {
    let answers =
        answers(world, &method).unwrap_or_else(|| panic!("the server sent no {method} request"));
    let expected: Value = serde_json::from_str(&expected).expect("the expected result is JSON");
    for answer in &answers {
        let result = answer
            .strip_prefix("result ")
            .and_then(|text| serde_json::from_str::<Value>(text).ok());
        assert_eq!(
            result.as_ref(),
            Some(&expected),
            "{method} answers: {answers:?}"
        );
    }
}

#[then(expr = "the server sent no {string} request")]
fn server_sent_no(world: &mut CodetagsWorld, method: String) {
    let answers = answers(world, &method);
    assert!(answers.is_none(), "{method} answers: {answers:?}");
}

#[then(expr = "after the {string} the client's document notifications were {string}")]
fn notifications_after(world: &mut CodetagsWorld, action: String, expected: String) {
    let analysis = analysis(world);
    let actual = line_after(analysis, &format!("- `{action}` sync: "))
        .unwrap_or_else(|| panic!("no step {action:?} in the analysis:\n{analysis}"));
    assert_eq!(
        actual, expected,
        "document notifications after the {action}"
    );
}

#[then(expr = "the client sent no {string}")]
fn client_sent_no(world: &mut CodetagsWorld, method: String) {
    let analysis = analysis(world);
    let row = format!("| client | `{method}` |");
    assert!(
        !analysis.contains(&row) && !analysis.contains(&format!("`{method}` ")),
        "the client sent {method}:\n{analysis}"
    );
}

#[then(expr = "after the {string} the client's first request was {string}")]
fn first_request_after(world: &mut CodetagsWorld, action: String, expected: String) {
    let analysis = analysis(world);
    let requests = line_after(analysis, &format!("- `{action}` requests: "))
        .unwrap_or_else(|| panic!("no step {action:?} in the analysis:\n{analysis}"));
    let first = requests.split(", ").next().unwrap_or(requests);
    assert_eq!(first, expected, "requests after the {action}: {requests}");
}

#[then(expr = "the client's last request was {string}")]
fn last_request(world: &mut CodetagsWorld, expected: String) {
    let analysis = analysis(world);
    let section = analysis
        .split("## Client-to-server requests\n")
        .nth(1)
        .unwrap_or_else(|| panic!("no client requests in the analysis:\n{analysis}"));
    // The first table: | at | id | `method` | answer | latency |
    let last = section
        .lines()
        .skip_while(|line| line.is_empty())
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split('`').nth(1))
        .last();
    assert_eq!(last, Some(expected.as_str()), "client requests:\n{section}");
}

#[then("every didChange the client sent carried the full text")]
fn did_change_full_text(world: &mut CodetagsWorld) {
    let analysis = analysis(world);
    let pattern =
        regex::Regex::new(r"`textDocument/didChange` .* \((\d+) changes, (\d+) full-text\)$")
            .expect("valid regex");
    let changes: Vec<(String, String)> = analysis
        .lines()
        .filter_map(|line| pattern.captures(line))
        .map(|captures| (captures[1].to_string(), captures[2].to_string()))
        .collect();
    assert!(!changes.is_empty(), "the client sent no didChange");
    assert!(
        changes.iter().all(|(all, full)| all == full),
        "didChange (changes, full-text): {changes:?}"
    );
}

#[then("the recording has no end-of-stream or exit record")]
fn no_end_records(world: &mut CodetagsWorld) {
    let analysis = analysis(world);
    assert!(
        !analysis.contains(" closed its stream at ") && !analysis.contains("- server exited at "),
        "the recording ends with:\n{analysis}"
    );
}

#[given("the scratch project uses the codetags-lsp plugin through lspmux")]
fn uses_the_shim(world: &mut CodetagsWorld) {
    let project = world
        .claude
        .project
        .clone()
        .expect("the Background made the scratch project");
    let scratch = project
        .parent()
        .expect("the project is in the scratch directory")
        .to_path_buf();
    let local = project.join(".codetags").join("local");
    let lspmux = crate::steps_lspmux::lspmux_binary()
        .expect("lspmux not found: run `just setup-lspmux`, or set CODETAGS_LSPMUX");
    std::fs::copy(&lspmux, local.join("bin").join("lspmux")).expect("copy lspmux into the project");
    // A separate lspmux config (Linux: XDG_CONFIG_HOME; the wrapper sets it
    // from CODETAGS_LSPMUX_XDG_CONFIG_HOME, so Claude Code keeps its own),
    // with the socket in a short private runtime directory.
    let config_home = scratch.join("xdg-config");
    let runtime = tempfile::Builder::new()
        .prefix("ct")
        .tempdir()
        .expect("create a runtime directory");
    let codetags = crate::CODETAGS_BIN
        .get()
        .expect("run() sets the binary path");
    let output = Command::new(codetags)
        .args(["lsp", "setup", "--yes"])
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_RUNTIME_DIR", runtime.path())
        .output()
        .expect("run codetags lsp setup");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success()
            && stdout.contains(&format!("lspmux config: {}", config_home.display())),
        "codetags lsp setup:\n{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    world.claude.shim = Some(ShimSetup {
        plugin: repo_root()
            .join("tools")
            .join("claude-plugins")
            .join("codetags-lsp"),
        env: vec![
            ("CODETAGS_LSPMUX_XDG_CONFIG_HOME".into(), config_home),
            ("CODETAGS_LSP_LOG_DIR".into(), local),
        ],
        daemon_log: runtime.path().join("codetags").join("lspmux.log"),
        _runtime: runtime,
    });
}

/// The messages of a recording, parsed.
fn records(recording: &str) -> Vec<Value> {
    recording
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|record| record.get("from").is_some())
        .collect()
}

#[then(expr = "a {string} request from the client got a non-empty result")]
fn request_got_result(world: &mut CodetagsWorld, method: String) {
    let outcome = outcome(world);
    let recording = outcome
        .recordings
        .first()
        .unwrap_or_else(|| panic!("the session wrote no recording\n{}", outcome.diagnostics));
    let records = records(recording);
    let ids: Vec<&Value> = records
        .iter()
        .filter(|record| record["from"] == "client" && record["msg"]["method"] == method.as_str())
        .filter_map(|record| record["msg"].get("id"))
        .collect();
    assert!(!ids.is_empty(), "the client sent no {method} request");
    let answered = records.iter().any(|record| {
        record["from"] == "server"
            && record["msg"].get("method").is_none()
            && record["msg"].get("id").is_some_and(|id| ids.contains(&id))
            && match &record["msg"]["result"] {
                Value::Null => false,
                Value::Array(items) => !items.is_empty(),
                _ => true,
            }
    });
    assert!(
        answered,
        "no {method} request got a non-empty result; ids {ids:?}"
    );
}

#[then("the language server received no document notifications")]
fn server_received_no_documents(world: &mut CodetagsWorld) {
    let outcome = outcome(world);
    assert!(
        !outcome.server_recordings.is_empty(),
        "no server-side recording"
    );
    for recording in &outcome.server_recordings {
        let documents: Vec<Value> = records(recording)
            .into_iter()
            .filter(|record| record["from"] == "client")
            .filter(|record| {
                record["msg"]["method"]
                    .as_str()
                    .is_some_and(is_document_notification)
            })
            .collect();
        assert!(documents.is_empty(), "the server received {documents:#?}");
    }
}

fn is_document_notification(method: &str) -> bool {
    matches!(
        method,
        "textDocument/didOpen"
            | "textDocument/didChange"
            | "textDocument/didSave"
            | "textDocument/didClose"
    )
}

#[then("one language server process served the session")]
fn one_server_process(world: &mut CodetagsWorld) {
    let outcome = outcome(world);
    assert_eq!(
        outcome.server_recordings.len(),
        1,
        "server-side recordings: {}",
        outcome.server_recordings.len()
    );
}

/// The client's log and the step marks the lookup steps read: a recorded
/// session's, or the live session's first (client-side) recording.
fn client_log(world: &CodetagsWorld) -> (String, Vec<Value>) {
    if let Some((text, marks)) = &world.claude.recorded {
        return (text.clone(), marks.clone());
    }
    let outcome = outcome(world);
    let recording = outcome
        .recordings
        .first()
        .unwrap_or_else(|| panic!("the session wrote no recording\n{}", outcome.diagnostics));
    (recording.clone(), outcome.marks.clone())
}

/// The result of the client's last `method` request that `wanted` accepts
/// (by its params), sent after the step mark `step` and before the next one.
fn last_answer(
    recording: &str,
    marks: &[Value],
    step: &str,
    method: &str,
    wanted: impl Fn(&Value) -> bool,
) -> Result<Value, String> {
    let position = marks
        .iter()
        .position(|mark| mark["step"] == step)
        .ok_or_else(|| format!("no step mark {step:?}"))?;
    let start = marks[position]["ts_ms"].as_u64().unwrap_or(0);
    let end = marks
        .get(position + 1)
        .and_then(|mark| mark["ts_ms"].as_u64())
        .unwrap_or(u64::MAX);
    let records = records(recording);
    let id = records
        .iter()
        .filter(|record| record["from"] == "client" && record["msg"]["method"] == method)
        .filter(|record| {
            record["ts_ms"]
                .as_u64()
                .is_some_and(|at| (start..end).contains(&at))
        })
        .filter(|record| wanted(&record["msg"]["params"]))
        .filter_map(|record| record["msg"].get("id"))
        .last()
        .ok_or_else(|| {
            format!("the client sent no matching {method} request after the {step:?}")
        })?;
    let answer = records
        .iter()
        .find(|record| {
            record["from"] == "server"
                && record["msg"].get("method").is_none()
                && record["msg"].get("id") == Some(id)
        })
        .ok_or_else(|| format!("{method} request {id} after the {step:?} got no answer"))?;
    match answer["msg"].get("error") {
        Some(error) => Err(format!("{method} after the {step:?} failed: {error}")),
        None => Ok(answer["msg"]["result"].clone()),
    }
}

/// Every symbol name in a `workspace/symbol` or `textDocument/documentSymbol`
/// result, children included.
fn symbol_names(result: &Value) -> Vec<String> {
    let mut names = Vec::new();
    let mut stack: Vec<&Value> = result.as_array().into_iter().flatten().collect();
    while let Some(symbol) = stack.pop() {
        if let Some(name) = symbol["name"].as_str() {
            names.push(name.to_string());
        }
        stack.extend(symbol["children"].as_array().into_iter().flatten());
    }
    names.sort();
    names
}

/// The names the last workspace search for `query` after `step` returned.
fn searched(world: &CodetagsWorld, step: &str, query: &str) -> Vec<String> {
    let (recording, marks) = client_log(world);
    let result = last_answer(&recording, &marks, step, "workspace/symbol", |params| {
        params["query"] == query
    })
    .unwrap_or_else(|error| panic!("{error}"));
    symbol_names(&result)
}

#[then(expr = "after the {string} a workspace symbol search for {string} found it")]
fn search_found(world: &mut CodetagsWorld, mark: String, query: String) {
    let names = searched(world, &mark, &query);
    assert!(
        names.contains(&query),
        "after the {mark:?}, workspace/symbol {query:?} returned {names:?}"
    );
}

#[then(expr = "after the {string} a workspace symbol search for {string} found nothing")]
fn search_found_nothing(world: &mut CodetagsWorld, mark: String, query: String) {
    let names = searched(world, &mark, &query);
    assert!(
        !names.contains(&query),
        "after the {mark:?}, workspace/symbol {query:?} returned {names:?}"
    );
}

/// The names the last documentSymbol request for `file` after `step` returned.
fn document_symbols(world: &CodetagsWorld, step: &str, file: &str) -> Vec<String> {
    let (recording, marks) = client_log(world);
    let suffix = format!("/{file}");
    let result = last_answer(
        &recording,
        &marks,
        step,
        "textDocument/documentSymbol",
        |params| {
            params["textDocument"]["uri"]
                .as_str()
                .is_some_and(|uri| uri.ends_with(&suffix))
        },
    )
    .unwrap_or_else(|error| panic!("{error}"));
    symbol_names(&result)
}

#[then(expr = "after the {string} the symbols of {string} include {string}")]
fn symbols_include(world: &mut CodetagsWorld, mark: String, file: String, name: String) {
    let names = document_symbols(world, &mark, &file);
    assert!(
        names.contains(&name),
        "after the {mark:?}, {file} has the symbols {names:?}"
    );
}

#[then(expr = "after the {string} the symbols of {string} do not include {string}")]
fn symbols_exclude(world: &mut CodetagsWorld, mark: String, file: String, name: String) {
    let names = document_symbols(world, &mark, &file);
    assert!(
        !names.is_empty() && !names.contains(&name),
        "after the {mark:?}, {file} has the symbols {names:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::{bash_commands, last_answer, slug, symbol_names};
    use serde_json::json;

    fn log(lines: &[serde_json::Value]) -> String {
        lines.iter().map(|line| format!("{line}\n")).collect()
    }

    #[test]
    fn last_answer_reads_the_marks_window() {
        let marks = vec![
            json!({"step": "sed", "ts_ms": 100}),
            json!({"step": "git checkout", "ts_ms": 200}),
        ];
        let request = |id: u32, at: u64, query: &str| json!({"from": "client", "ts_ms": at, "msg": {"id": id, "method": "workspace/symbol", "params": {"query": query}}});
        let answer = |id: u32, name: &str| json!({"from": "server", "ts_ms": 0, "msg": {"id": id, "result": [{"name": name}]}});
        let recording = log(&[
            request(1, 150, "x"),
            answer(1, "first"),
            request(2, 160, "x"),
            answer(2, "second"),
            request(3, 170, "y"),
            answer(3, "other query"),
            request(4, 250, "x"),
            answer(4, "after the next mark"),
        ]);
        let found = |step: &str| {
            last_answer(&recording, &marks, step, "workspace/symbol", |params| {
                params["query"] == "x"
            })
        };
        assert_eq!(found("sed"), Ok(json!([{"name": "second"}])));
        assert_eq!(
            found("git checkout"),
            Ok(json!([{"name": "after the next mark"}]))
        );
        assert!(found("cat").is_err());
    }

    #[test]
    fn symbol_names_include_children() {
        let result = json!([
            {"name": "Ledger", "children": [{"name": "record"}]},
            {"name": "settle"},
        ]);
        assert_eq!(symbol_names(&result), vec!["Ledger", "record", "settle"]);
        assert!(symbol_names(&serde_json::Value::Null).is_empty());
    }

    #[test]
    fn bash_commands_split_on_and() {
        let plan = vec![
            "a scratch Rust project".to_string(),
            r#"Claude Code runs "git checkout -- a.rs && rm b.rs" through Bash, then hovers on "X" in "a.rs""#.to_string(),
        ];
        assert_eq!(
            bash_commands(&plan),
            vec!["git checkout -- a.rs", "rm b.rs"]
        );
    }

    #[test]
    fn slugs_rule_names() {
        assert_eq!(slug("The handshake, from a session"), "the-handshake");
        assert_eq!(slug("Document sync: the script"), "document-sync");
        assert_eq!(slug(""), "session");
    }
}
