//! Steps for `features/lsp/shim.feature` (M3 stage 2, PLAN.md D14-D22):
//! `codetags-lsp serve`, and wrappers that run it, between scripted sessions
//! and the fake language server, through a real lspmux. The scenario's home
//! (`steps_lspmux`) gives each its own lspmux config and daemon; on Windows
//! the daemon listens on a free loopback port.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use serde_json::{Value, json};

use crate::CodetagsWorld;
use crate::steps_lsp::{CHILD_MODE, LOG_ENV, MODE_ENV, STATUS_ENV, lsp_binary};
use crate::steps_lspmux::{lspmux_binary, repo_root};

/// How long a session may wait for one answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);

/// A running session: a shim (or wrapper) process and what it said.
#[derive(Debug)]
pub(crate) struct Session {
    child: Child,
    stdin: Option<ChildStdin>,
    messages: Receiver<Value>,
    /// Messages read so far that no step has claimed.
    pub(crate) seen: Vec<Value>,
    /// Answers by request method.
    pub(crate) answers: HashMap<String, Value>,
    next_id: i64,
    stderr: PathBuf,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Session {
    pub(crate) fn send(&mut self, message: &Value) {
        let body = message.to_string();
        let stdin = self.stdin.as_mut().expect("the session's input is open");
        let written = write!(stdin, "Content-Length: {}\r\n\r\n{body}", body.len())
            .and_then(|()| stdin.flush());
        if let Err(error) = written {
            panic!("write to the session: {error}\n{}", self.diagnostics());
        }
    }

    /// Waits for the response with `id`.
    pub(crate) fn answer(&mut self, id: &Value) -> Value {
        if let Some(index) = self
            .seen
            .iter()
            .position(|message| message.get("id") == Some(id) && message.get("method").is_none())
        {
            return self.seen.remove(index);
        }
        let deadline = Instant::now() + ANSWER_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.messages.recv_timeout(left) {
                Ok(message) if message.get("id") == Some(id) && message.get("method").is_none() => {
                    return message;
                }
                Ok(message) => self.seen.push(message),
                Err(RecvTimeoutError::Timeout) => {
                    panic!(
                        "no answer to request {id} within {ANSWER_TIMEOUT:?}\n{}",
                        self.diagnostics()
                    )
                }
                Err(RecvTimeoutError::Disconnected) => {
                    panic!(
                        "the session ended before answering request {id}\n{}",
                        self.diagnostics()
                    )
                }
            }
        }
    }

    /// Sends a request and waits for its answer.
    pub(crate) fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = json!(self.next_id);
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let answer = self.answer(&id);
        self.answers.insert(method.to_string(), answer.clone());
        answer
    }

    pub(crate) fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Waits for the process to exit; its exit code.
    fn wait_exit(&mut self, timeout: Duration) -> Option<i32> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return status.code(),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Ok(None) => panic!(
                    "the session did not exit within {timeout:?}\n{}",
                    self.diagnostics()
                ),
                Err(error) => panic!("wait for the session: {error}"),
            }
        }
    }

    pub(crate) fn diagnostics(&self) -> String {
        format!(
            "session stderr:\n{}",
            std::fs::read_to_string(&self.stderr).unwrap_or_default()
        )
    }
}

/// State for the shim scenarios. `CodetagsWorld` declares it before the
/// home (`steps_lspmux`), so sessions end before the daemons are killed.
#[derive(Debug, Default)]
pub(crate) struct ShimState {
    pub(crate) sessions: HashMap<String, Session>,
    /// The fake server's log (`LOG_ENV`).
    fake_log: Option<PathBuf>,
    /// The project the sessions open.
    pub(crate) project: Option<PathBuf>,
    /// Whether the sessions find the root as a Cargo workspace.
    cargo_root: bool,
    /// Wrappers by name.
    wrappers: HashMap<String, PathBuf>,
    /// Exit codes of sessions that ended.
    exited: HashMap<String, Option<i32>>,
}

/// A `file:` URI for `path`, as an editor would write it.
pub(crate) fn file_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

// --- the shim ------------------------------------------------------------

/// The project directory the sessions open: `project` in the scratch
/// directory, with `src/lib.rs`.
pub(crate) fn project(world: &mut CodetagsWorld) -> PathBuf {
    if let Some(project) = &world.shim.project {
        return project.clone();
    }
    let project = world.scratch().join("project");
    std::fs::create_dir_all(project.join("src")).expect("create the project");
    std::fs::write(project.join("src").join("lib.rs"), "pub fn f() {}\n").expect("write lib.rs");
    world.shim.project = Some(project.clone());
    project
}

#[given(expr = "the project is a Cargo workspace with the member crate {string}")]
fn a_cargo_workspace(world: &mut CodetagsWorld, member: String) {
    let project = project(world);
    std::fs::write(
        project.join("Cargo.toml"),
        format!("[workspace]\nmembers = [\"{member}\"]\n"),
    )
    .expect("write the workspace manifest");
    let crate_dir = project.join(&member);
    std::fs::create_dir_all(crate_dir.join("src")).expect("create the member");
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname = \"member\"\n",
    )
    .expect("write the member manifest");
    world.shim.cargo_root = true;
}

/// The fake server's environment: child mode, status 0, and its log.
fn fake_env(world: &mut CodetagsWorld) -> Vec<(OsString, OsString)> {
    let log = world.scratch().join("fake-server.jsonl");
    world.shim.fake_log = Some(log.clone());
    vec![
        (MODE_ENV.into(), CHILD_MODE.into()),
        (STATUS_ENV.into(), "0".into()),
        (LOG_ENV.into(), log.into()),
    ]
}

fn fake_server_path() -> PathBuf {
    std::env::current_exe().expect("locate the test executable")
}

/// `codetags-lsp serve --role ROLE --server <fake> --lspmux <lspmux>`.
fn serve_command(world: &mut CodetagsWorld, role: &str, args: &[&str]) -> Command {
    let mut command = Command::new(lsp_binary());
    command
        .args(["serve", "--role", role, "--server"])
        .arg(fake_server_path());
    if let Some(lspmux) = lspmux_binary() {
        command.arg("--lspmux").arg(lspmux);
    }
    if world.shim.cargo_root {
        command.args(["--root", "cargo"]);
    }
    command.arg("--").args(args);
    let env = fake_env(world);
    command.envs(world.env.iter().map(|(name, value)| (name, value)));
    command.envs(env);
    command
}

#[when(
    expr = "codetags-lsp serve runs the fake server as the {string} with the arguments {string}"
)]
fn serve_runs_with_arguments(world: &mut CodetagsWorld, role: String, args: String) {
    let args: Vec<&str> = args.split_whitespace().collect();
    let mut command = serve_command(world, &role, &args);
    command.stdin(Stdio::null());
    let output = command.output().expect("run codetags-lsp serve");
    world.last = Some(output.into());
}

/// Starts a session process: `command` with piped stdio, read on a thread.
pub(crate) fn spawn_session(
    world: &mut CodetagsWorld,
    name: &str,
    mut command: Command,
) -> Session {
    let stderr = world.scratch().join(format!("{name}.stderr"));
    command
        .current_dir(project(world))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(&stderr).expect("create the stderr file"));
    let mut child = command.spawn().expect("start the session");
    let stdout = child.stdout.take().expect("piped stdout");
    let (sender, messages) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Some(body) = read_message(&mut reader) {
            let Ok(message) = serde_json::from_slice::<Value>(&body) else {
                break;
            };
            if sender.send(message).is_err() {
                break;
            }
        }
    });
    Session {
        stdin: child.stdin.take(),
        child,
        messages,
        seen: Vec::new(),
        answers: HashMap::new(),
        next_id: 1,
        stderr,
    }
}

/// One framed message's body; `None` at the end of input.
fn read_message(input: &mut impl BufRead) -> Option<Vec<u8>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            if length.is_some() {
                break;
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length?];
    input.read_exact(&mut body).ok()?;
    Some(body)
}

/// The `initialize` a scripted client sends for `root`.
pub(crate) fn initialize(name: &str, root: &Path, capabilities: Value) -> Value {
    let uri = file_uri(root);
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": null,
            "clientInfo": {"name": name},
            "rootPath": root.to_string_lossy(),
            "rootUri": uri,
            "workspaceFolders": [{"uri": uri, "name": root.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()}],
            "capabilities": capabilities,
        },
    })
}

/// Sends `initialize`, waits for the answer to its id, sends `initialized`.
pub(crate) fn handshake(session: &mut Session, message: &Value) {
    session.send(message);
    let answer = session.answer(&message["id"]);
    assert!(
        answer.get("result").is_some(),
        "initialize was refused: {answer}\n{}",
        session.diagnostics()
    );
    session.notify("initialized", json!({}));
}

fn start_session(world: &mut CodetagsWorld, role: &str, name: &str, root: &Path) -> Session {
    let command = serve_command(world, role, &[]);
    let mut session = spawn_session(world, name, command);
    handshake(&mut session, &initialize(name, root, json!({})));
    session
}

#[when(expr = "an {string} session {string} starts through codetags-lsp serve")]
fn a_session_starts(world: &mut CodetagsWorld, role: String, name: String) {
    let root = project(world);
    let session = start_session(world, &role, &name, &root);
    world.shim.sessions.insert(name, session);
}

#[when(expr = "an {string} session {string} starts through codetags-lsp serve in {string}")]
fn a_session_starts_in(world: &mut CodetagsWorld, role: String, name: String, dir: String) {
    let root = project(world).join(dir);
    let session = start_session(world, &role, &name, &root);
    world.shim.sessions.insert(name, session);
}

#[when(
    expr = "the {string} sessions {string} and {string} start at once through codetags-lsp serve"
)]
fn sessions_start_at_once(world: &mut CodetagsWorld, role: String, first: String, second: String) {
    let root = project(world);
    let mut sessions = Vec::new();
    for name in [&first, &second] {
        let command = serve_command(world, &role, &[]);
        let mut session = spawn_session(world, name, command);
        session.send(&initialize(name, &root, json!({})));
        sessions.push(session);
    }
    for (name, mut session) in [first, second].into_iter().zip(sessions) {
        let answer = session.answer(&json!(1));
        assert!(
            answer.get("result").is_some(),
            "{name}: initialize refused: {answer}"
        );
        session.notify("initialized", json!({}));
        world.shim.sessions.insert(name, session);
    }
}

#[when(expr = "an {string} session {string} sends an initialize with {int} workspace folders")]
fn multi_root_initialize(world: &mut CodetagsWorld, role: String, name: String, folders: usize) {
    let root = project(world);
    let command = serve_command(world, &role, &[]);
    let mut session = spawn_session(world, &name, command);
    let mut message = initialize(&name, &root, json!({}));
    let workspace_folders: Vec<Value> = (0..folders)
        .map(|index| {
            let dir = root.join(format!("folder{index}"));
            std::fs::create_dir_all(&dir).expect("create a folder");
            json!({"uri": file_uri(&dir), "name": format!("folder{index}")})
        })
        .collect();
    message["params"]["workspaceFolders"] = json!(workspace_folders);
    session.send(&message);
    let answer = session.answer(&json!(1));
    session.answers.insert("initialize".into(), answer);
    session.stdin.take();
    let code = session.wait_exit(Duration::from_secs(30));
    world.shim.exited.insert(name.clone(), code);
    world.shim.sessions.insert(name, session);
}

pub(crate) fn session<'a>(world: &'a mut CodetagsWorld, name: &str) -> &'a mut Session {
    world
        .shim
        .sessions
        .get_mut(name)
        .unwrap_or_else(|| panic!("no session {name:?}"))
}

fn document(world: &mut CodetagsWorld, file: &str) -> String {
    file_uri(&project(world).join(file))
}

#[when(expr = "session {string} sends {string} for {string}")]
fn session_sends(world: &mut CodetagsWorld, name: String, method: String, file: String) {
    let uri = document(world, &file);
    let params = match method.as_str() {
        "textDocument/didOpen" => {
            json!({"textDocument": {"uri": uri, "languageId": "rust", "version": 1, "text": "pub fn f() {}\n"}})
        }
        "textDocument/didChange" => {
            json!({"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": "pub fn g() {}\n"}]})
        }
        "textDocument/didSave" | "textDocument/didClose" => json!({"textDocument": {"uri": uri}}),
        "workspace/didChangeWatchedFiles" => json!({"changes": [{"uri": uri, "type": 2}]}),
        other => panic!("no scripted notification {other:?}"),
    };
    session(world, &name).notify(&method, params);
}

#[when(expr = "session {string} asks {string} for {string}")]
fn session_asks(world: &mut CodetagsWorld, name: String, method: String, file: String) {
    let uri = document(world, &file);
    let params = json!({"textDocument": {"uri": uri}, "position": {"line": 0, "character": 7}});
    session(world, &name).request(&method, params);
}

#[when(expr = "session {string} shuts down and is killed without exit")]
fn session_shuts_down_and_is_killed(world: &mut CodetagsWorld, name: String) {
    let mut session = world
        .shim
        .sessions
        .remove(&name)
        .unwrap_or_else(|| panic!("no session {name:?}"));
    session.request("shutdown", Value::Null);
    let _ = session.child.kill();
    let _ = session.child.wait();
}

#[then(expr = "session {string} got the error {int} for its initialize")]
fn initialize_error(world: &mut CodetagsWorld, name: String, code: i64) {
    let session = session(world, &name);
    let answer = session
        .answers
        .get("initialize")
        .expect("an initialize was answered");
    assert_eq!(
        answer["error"]["code"],
        json!(code),
        "initialize answer: {answer}"
    );
}

#[then(expr = "session {string}'s answer to {string} was {string}")]
fn answer_was(world: &mut CodetagsWorld, name: String, method: String, expected: String) {
    let expected: Value = serde_json::from_str(&expected).expect("the expected result is JSON");
    let session = session(world, &name);
    let answer = session
        .answers
        .get(&method)
        .unwrap_or_else(|| panic!("no answer to {method}"));
    assert_eq!(answer.get("result"), Some(&expected), "answer: {answer}");
}

#[then(expr = "session {string} exited with status {int}")]
fn session_exited(world: &mut CodetagsWorld, name: String, code: i32) {
    let exited = world
        .shim
        .exited
        .get(&name)
        .unwrap_or_else(|| panic!("session {name:?} did not end"));
    assert_eq!(*exited, Some(code));
}

/// The fake server's log records.
fn fake_records(world: &CodetagsWorld) -> Vec<Value> {
    let path = world
        .shim
        .fake_log
        .as_ref()
        .expect("a session ran the fake server");
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("each fake-server log line is JSON"))
        .collect()
}

fn received(world: &CodetagsWorld, method: &str) -> usize {
    fake_records(world)
        .iter()
        .filter(|record| record["msg"]["method"] == method)
        .count()
}

#[then(expr = "the fake server was started {int} time(s)")]
fn fake_started(world: &mut CodetagsWorld, count: usize) {
    let starts = fake_records(world)
        .iter()
        .filter(|record| record["event"] == "start")
        .count();
    assert_eq!(starts, count, "fake server log: {:#?}", fake_records(world));
}

#[then(expr = "the fake server received {string} {int} time(s)")]
fn fake_received_times(world: &mut CodetagsWorld, method: String, count: usize) {
    assert_eq!(
        received(world, &method),
        count,
        "fake server log: {:#?}",
        fake_records(world)
    );
}

#[then(expr = "the fake server received no {string}")]
fn fake_received_none(world: &mut CodetagsWorld, method: String) {
    assert_eq!(
        received(world, &method),
        0,
        "fake server log: {:#?}",
        fake_records(world)
    );
}

#[then("the fake server received no document notifications")]
fn fake_received_no_documents(world: &mut CodetagsWorld) {
    for method in [
        "textDocument/didOpen",
        "textDocument/didChange",
        "textDocument/didSave",
        "textDocument/didClose",
    ] {
        assert_eq!(
            received(world, method),
            0,
            "{method}: {:#?}",
            fake_records(world)
        );
    }
}

fn fake_initialize(world: &CodetagsWorld) -> Value {
    fake_records(world)
        .into_iter()
        .find(|record| record["msg"]["method"] == "initialize")
        .map(|record| record["msg"].clone())
        .expect("the fake server received an initialize")
}

#[then("the fake server's initialize named the project root")]
fn fake_initialize_root(world: &mut CodetagsWorld) {
    let uri = file_uri(&project(world));
    let initialize = fake_initialize(world);
    assert_eq!(
        initialize["params"]["rootUri"],
        json!(uri),
        "{initialize:#}"
    );
    assert_eq!(
        initialize["params"]["workspaceFolders"][0]["uri"],
        json!(uri),
        "{initialize:#}"
    );
}

#[then(expr = "the fake server's initialize did not advertise {string}")]
fn fake_initialize_lacks(world: &mut CodetagsWorld, path: String) {
    let initialize = fake_initialize(world);
    let pointer = format!("/params/capabilities/{}", path.replace('.', "/"));
    assert!(initialize.pointer(&pointer).is_none(), "{initialize:#}");
}

// --- wrappers and scripted clients ---------------------------------------

#[given(expr = "a wrapper {string} that runs the fake server as the {string}")]
fn a_wrapper(world: &mut CodetagsWorld, name: String, role: String) {
    let dest = world.scratch().join("wrappers").join(&name);
    let mut command = Command::new(lsp_binary());
    command
        .args(["install-wrapper", "--role", &role, "--server"])
        .arg(fake_server_path());
    if let Some(lspmux) = lspmux_binary() {
        command.arg("--lspmux").arg(lspmux);
    }
    let output = command
        .arg(&dest)
        .output()
        .expect("run codetags-lsp install-wrapper");
    assert!(
        output.status.success(),
        "install-wrapper: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    world.shim.wrappers.insert(name, dest);
}

/// The wrapper's command, as an editor would start it: the path with no
/// extension, which Windows resolves to the `.exe` (V119).
fn wrapper_command(world: &mut CodetagsWorld, name: &str, args: &[&str]) -> Command {
    let wrapper = world
        .shim
        .wrappers
        .get(name)
        .unwrap_or_else(|| panic!("no wrapper {name:?}"))
        .clone();
    let mut command = Command::new(wrapper);
    command.args(args);
    let env = fake_env(world);
    command.envs(world.env.iter().map(|(name, value)| (name, value)));
    command.envs(env);
    command
}

#[when(expr = "the wrapper {string} runs with the arguments {string}")]
fn wrapper_runs(world: &mut CodetagsWorld, name: String, args: String) {
    let args: Vec<&str> = args.split_whitespace().collect();
    let mut command = wrapper_command(world, &name, &args);
    command.stdin(Stdio::null());
    let output = command.output().expect("run the wrapper");
    world.last = Some(output.into());
}

/// The client capabilities a VS Code-style client sends (the parts the
/// shim and lspmux care about).
pub(crate) fn vscode_capabilities() -> Value {
    json!({
        "general": {"positionEncodings": ["utf-16"]},
        "window": {"workDoneProgress": true},
        "workspace": {
            "configuration": true,
            "workspaceFolders": true,
            "didChangeWatchedFiles": {"dynamicRegistration": true, "relativePatternSupport": true},
        },
        "textDocument": {"synchronization": {"dynamicRegistration": true, "didSave": true}},
    })
}

#[when(expr = "a VS Code-style session {string} runs through the wrapper {string}")]
fn vscode_session(world: &mut CodetagsWorld, name: String, wrapper: String) {
    let root = project(world);
    let uri = document(world, "src/lib.rs");
    let command = wrapper_command(world, &wrapper, &[]);
    let mut session = spawn_session(world, &name, command);
    handshake(
        &mut session,
        &initialize(&name, &root, vscode_capabilities()),
    );
    session.notify(
        "textDocument/didOpen",
        json!({"textDocument": {"uri": uri, "languageId": "rust", "version": 1, "text": "pub fn f() {}\n"}}),
    );
    session.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": "pub fn g() {}\n"}]}),
    );
    session.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes": [{"uri": uri, "type": 2}]}),
    );
    session.request(
        "textDocument/hover",
        json!({"textDocument": {"uri": uri}, "position": {"line": 0, "character": 7}}),
    );
    session.request("shutdown", Value::Null);
    session.notify("exit", Value::Null);
    let code = session.wait_exit(Duration::from_secs(30));
    world.shim.exited.insert(name.clone(), code);
    world.shim.sessions.insert(name, session);
}

/// The recorded client messages of `tests/fixtures/lsp/<name>.jsonl`, with
/// the sanitized project (`/project`) replaced by `project`.
pub(crate) fn recorded_client_messages(name: &str, project: &Path) -> Vec<Value> {
    let path = repo_root()
        .join("tests")
        .join("fixtures")
        .join("lsp")
        .join(format!("{name}.jsonl"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let uri = file_uri(project);
    let path_json = serde_json::to_string(&project.to_string_lossy()).expect("a JSON string");
    let path_json = &path_json[1..path_json.len() - 1];
    text.lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("each fixture line is JSON"))
        .filter(|record| record["from"] == "client")
        .map(|record| {
            let message = record["msg"].to_string();
            let message = message
                .replace("file:///project", "\u{1}URI\u{1}")
                .replace("/project", path_json)
                .replace("\u{1}URI\u{1}", &uri);
            serde_json::from_str(&message).expect("the respelled message is JSON")
        })
        .collect()
}

#[when(
    expr = "the recorded Claude Code session {string} replays through codetags-lsp serve as the {string}"
)]
fn replay(world: &mut CodetagsWorld, name: String, role: String) {
    let root = project(world);
    let messages = recorded_client_messages(&name, &root);
    let shutdown = messages
        .iter()
        .find(|message| message["method"] == "shutdown")
        .and_then(|message| message.get("id").cloned())
        .expect("the recording ends with shutdown");
    let command = serve_command(world, &role, &[]);
    let mut session = spawn_session(world, "replay", command);
    for message in &messages {
        session.send(message);
    }
    // As Claude Code does (V115): wait for the shutdown answer, then kill.
    session.answer(&shutdown);
    let _ = session.child.kill();
    let _ = session.child.wait();
    world.shim.sessions.insert("replay".into(), session);
}
