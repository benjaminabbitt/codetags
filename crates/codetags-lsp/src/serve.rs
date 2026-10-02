//! `codetags-lsp serve`: the shim (M3 stages 1 and 2; PLAN.md D14-D22,
//! docs/proxy-zero-change.md §3.5, §7).
//!
//! An editor or agent starts it in place of the language server. A non-LSP
//! invocation (`--version`, a subcommand) runs the real server directly.
//! Otherwise it reads the client's `initialize`, refuses a multi-root one
//! with an LSP error, and rewrites the rest ([`policy::prepare_initialize`]);
//! makes sure an lspmux daemon answers ([`daemon::ensure_running`]); runs
//! `lspmux client --server-path <server>`, with the routing-key variable
//! `CODETAGS_KEY_ROOT` set; and relays the session through it, message by
//! message, applying the role's rules ([`policy`]).
//!
//! The session ends with the editor's `exit` or end of input; after
//! `shutdown`, which lspmux answers itself by detaching this session only,
//! the shim waits for either, so an editor that kills it instead (Claude
//! Code, V115) detaches nothing else.
//!
//! Agent sessions' requests pass through the readiness gate ([`ready`], P3.12).
//!
//! Not built yet: stage 3's watcher client, configuration
//! merge (P3.6), and crash recovery beyond ending the session (P3.7).

use std::ffi::OsString;
use std::io::{self, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::daemon;
use crate::framing::{FrameError, FrameReader, write_frame};
use crate::invocation::is_lsp_session;
use crate::logpath;
use crate::lspmux;
use crate::policy::{self, FromClient, FromServer, Initialize, Role};
use crate::ready;
use crate::record::{self, Log};
use crate::root::RootRule;
use crate::tools;
use crate::uri::{self, Rewrite};

/// The routing-key variable holding the canonical project root (R11, D26):
/// every spelling of one directory shares one server.
pub const KEY_ROOT_ENV: &str = "CODETAGS_KEY_ROOT";

/// What `serve` runs.
#[derive(Debug, Clone)]
pub struct Options {
    /// The session's role.
    pub role: Role,
    /// The real language server.
    pub server: PathBuf,
    /// Arguments for the server.
    pub args: Vec<OsString>,
    /// The lspmux binary; by default [`tools::find_lspmux`].
    pub lspmux: Option<PathBuf>,
    /// A log of the session, as `record` writes it (`{ts}`, `{pid}`
    /// expanded), with dropped and locally answered messages marked.
    pub log: Option<PathBuf>,
    /// Subcommands that still start an LSP session ([`is_lsp_session`]).
    pub lsp_subcommands: Vec<String>,
    /// How to find the project root; by default from the server's name.
    pub root_rule: Option<RootRule>,
    /// The readiness gate's bound in seconds ([`ready::timeout`]); by
    /// default from the environment.
    pub ready_timeout: Option<u64>,
}

/// How a `serve` run ended.
#[derive(Debug)]
pub enum Outcome {
    /// End with this process's exit status (a non-LSP invocation, or lspmux
    /// failing).
    Status(ExitStatus),
    /// End with this exit code.
    Code(u8),
}

/// Runs the shim as `options` say.
pub fn run(options: &Options) -> Result<Outcome, String> {
    let server = tools::resolve(&options.server)
        .ok_or_else(|| format!("{} not found", options.server.display()))?;
    let bound = ready::timeout(options.ready_timeout)?;
    if !is_lsp_session(&options.args, &options.lsp_subcommands) {
        return Command::new(&server)
            .args(&options.args)
            .status()
            .map(Outcome::Status)
            .map_err(|error| format!("cannot run {}: {error}", server.display()));
    }
    let log = match &options.log {
        Some(template) => {
            let path = logpath::expand(template, record::unix_ms() / 1000, std::process::id());
            Some(Arc::new(Log::create(&path)?))
        }
        None => None,
    };
    if let Some(log) = &log {
        log.write(json!({
            "event": "start",
            "format": 1,
            "shim": options.role.as_str(),
            "server": server.to_string_lossy(),
            "args": options.args.iter().map(|arg| arg.to_string_lossy()).collect::<Vec<_>>(),
            "pid": Value::Null,
            "recorder_pid": std::process::id(),
            "cwd": std::env::current_dir().map(|dir| dir.to_string_lossy().into_owned()).ok(),
        }));
    }
    let mut editor = FrameReader::new(BufReader::new(io::stdin()));
    let first = match editor.read_frame() {
        Ok(Some(frame)) => frame,
        Ok(None) => return Ok(Outcome::Code(0)),
        Err(error) => return Err(format!("reading the client's first message: {error}")),
    };
    note(&log, "client", first.body(), first.headers(), None);
    let message: Value = serde_json::from_slice(first.body())
        .map_err(|error| format!("the client's first message is not JSON: {error}"))?;
    let rule = options
        .root_rule
        .unwrap_or_else(|| RootRule::for_server(&server));
    let (message, root, rewrite) = match policy::prepare_initialize(message, rule) {
        Initialize::Reject(error) => {
            let body = error.to_string();
            note(
                &log,
                "shim",
                body.as_bytes(),
                &[],
                Some("refused initialize"),
            );
            let mut out = io::stdout().lock();
            write_frame(&mut out, body.as_bytes())
                .and_then(|()| out.flush())
                .map_err(|error| format!("writing to the client: {error}"))?;
            drop(out);
            return Ok(refused(editor, &log));
        }
        Initialize::Forward {
            message,
            root,
            rewrite,
        } => (message, root, rewrite),
    };
    let mut message = message;
    // P3.12: every shim asks for the server's status, since lspmux
    // initializes the server with the first session's `initialize` (V151).
    let client_asked_status = ready::request_status(&mut message);
    let initialize_id = message.get("id").cloned().unwrap_or(Value::Null);
    let lspmux_binary = match &options.lspmux {
        Some(binary) => binary.clone(),
        None => tools::find_lspmux().ok_or(
            "lspmux not found: set CODETAGS_LSPMUX, put it on PATH, or run: just setup-lspmux",
        )?,
    };
    let (config, text) = lspmux::read_config()?;
    if text.is_none() {
        eprintln!(
            "codetags-lsp: {} does not exist; lspmux uses its defaults (run: codetags lsp setup)",
            config.display()
        );
    }
    let address = lspmux::effective_connect(text.as_deref());
    daemon::ensure_running(&lspmux_binary, &address)?;
    let mut key_env: Vec<(String, String)> = Vec::new();
    if let Some(root) = &root {
        key_env.push((
            KEY_ROOT_ENV.to_string(),
            root.to_string_lossy().into_owned(),
        ));
    }
    let mark = ready::LoadingMark::new(
        &daemon::state_dir(&address),
        &instance_key(&server, &options.args, &key_env),
    );
    let gate = ready::Gate::new(
        options.role == Role::Agent,
        bound,
        ready::GRACE,
        Some(mark),
        Instant::now(),
    );
    let mut command = Command::new(&lspmux_binary);
    command
        .arg("client")
        .arg("--server-path")
        .arg(&server)
        .arg("--")
        .args(&options.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    command.envs(key_env.iter().map(|(name, value)| (name, value)));
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start {} client: {error}", lspmux_binary.display()))?;
    let (Some(mut to_lspmux), Some(from_lspmux)) = (child.stdin.take(), child.stdout.take()) else {
        return Err("lspmux's stdio was not piped".to_string());
    };
    let body = message.to_string();
    if let Some(log) = &log {
        log.write(json!({"event": "initialize", "root": root, "sent": message}));
    }
    write_frame(&mut to_lspmux, body.as_bytes())
        .and_then(|()| to_lspmux.flush())
        .map_err(|error| format!("writing to lspmux: {error}"))?;
    relay(
        Shared {
            role: options.role,
            rewrite,
            gate,
            initialize_id,
            client_asked_status,
            log,
        },
        editor,
        child,
        (to_lspmux, from_lspmux),
    )
}

/// The instance a session lands on, as text: lspmux keys an instance on the
/// server, its arguments, the passed variables and the root (V3), which
/// [`KEY_ROOT_ENV`] holds.
fn instance_key(server: &std::path::Path, args: &[OsString], env: &[(String, String)]) -> String {
    let mut key = server.to_string_lossy().into_owned();
    for arg in args {
        key.push('\0');
        key.push_str(&arg.to_string_lossy());
    }
    let mut env = env.to_vec();
    env.sort();
    for (name, value) in env {
        key.push('\0');
        key.push_str(&name);
        key.push('=');
        key.push_str(&value);
    }
    key
}

/// Logs one message, if there is a log.
fn note(
    log: &Option<Arc<Log>>,
    from: &str,
    body: &[u8],
    headers: &[(String, String)],
    what: Option<&str>,
) {
    if let Some(log) = log {
        log.body(from, body, headers, what);
    }
}

/// After a refused `initialize`: answers `shutdown` and refuses other
/// requests until `exit` or the end of input.
fn refused(mut editor: FrameReader<BufReader<io::Stdin>>, log: &Option<Arc<Log>>) -> Outcome {
    let mut shutdown = false;
    let mut out = io::stdout().lock();
    loop {
        let frame = match editor.read_frame() {
            Ok(Some(frame)) => frame,
            _ => return Outcome::Code(u8::from(!shutdown)),
        };
        note(log, "client", frame.body(), frame.headers(), None);
        let Ok(message) = serde_json::from_slice::<Value>(frame.body()) else {
            continue;
        };
        let method = message.get("method").and_then(Value::as_str);
        let id = message.get("id").filter(|id| !id.is_null());
        let answer = match (method, id) {
            (Some("exit"), _) => return Outcome::Code(u8::from(!shutdown)),
            (Some("shutdown"), Some(id)) => {
                shutdown = true;
                json!({"jsonrpc": "2.0", "id": id, "result": null})
            }
            (Some(_), Some(id)) => json!({"jsonrpc": "2.0", "id": id, "error": {
                "code": policy::SERVER_NOT_INITIALIZED,
                "message": "codetags-lsp: the session's initialize was refused",
            }}),
            _ => continue,
        };
        let body = answer.to_string();
        note(log, "shim", body.as_bytes(), &[], Some("answered"));
        if write_frame(&mut out, body.as_bytes())
            .and_then(|()| out.flush())
            .is_err()
        {
            return Outcome::Code(1);
        }
    }
}

/// What the relay threads report.
enum Event {
    /// The editor sent `exit`.
    Exit,
    /// The editor's input ended.
    EditorEnded,
    /// lspmux's output ended.
    LspmuxEnded,
}

/// What [`run`] hands the relay.
struct Shared {
    role: Role,
    rewrite: Option<Rewrite>,
    gate: ready::Gate,
    /// The id of the client's `initialize`, to recognize its answer.
    initialize_id: Value,
    /// Whether the client asked for `experimental/serverStatus` itself.
    client_asked_status: bool,
    log: Option<Arc<Log>>,
}

/// Shared between the two relay directions.
struct Session {
    role: Role,
    /// The client's spelling of the root and the canonical one, when they
    /// differ (D26).
    rewrite: Option<Rewrite>,
    /// The readiness gate (P3.12). Held requests are written to lspmux, and
    /// requests the gate passes too, while its lock is held, so they keep
    /// their order.
    gate: Mutex<ready::Gate>,
    initialize_id: Value,
    client_asked_status: bool,
    to_lspmux: Mutex<Option<ChildStdin>>,
    /// The id of the editor's `shutdown` request, once sent.
    shutdown_id: Mutex<Option<Value>>,
    /// Whether that `shutdown` has been answered.
    shut_down: AtomicBool,
    log: Option<Arc<Log>>,
}

impl Session {
    /// Writes raw bytes to lspmux; false once its input is gone.
    fn to_lspmux(&self, bytes: &[u8]) -> bool {
        let mut guard = self
            .to_lspmux
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match guard.as_mut() {
            Some(stdin) => stdin.write_all(bytes).and_then(|()| stdin.flush()).is_ok(),
            None => false,
        }
    }

    /// Runs `change` on the gate, then forwards what it released and logs
    /// why, all under the gate's lock.
    fn gate(&self, change: impl FnOnce(&mut ready::Gate) -> ready::Opened) {
        let mut gate = self
            .gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let opened = change(&mut gate);
        for frame in &opened.frames {
            self.to_lspmux(frame);
        }
        if let Some(note) = &opened.note {
            self.gate_note(note);
        }
    }

    /// Says what the gate did, on stderr and in the session log.
    fn gate_note(&self, note: &str) {
        eprintln!("codetags-lsp: {note}");
        if let Some(log) = &self.log {
            log.write(json!({"event": "gate", "note": note}));
        }
    }

    /// Forwards a request from the client through the gate: held while it
    /// is closed, else written now.
    fn request_through_gate(&self, bytes: &[u8]) {
        let mut gate = self
            .gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let was_holding = gate.holding();
        match gate.pass(bytes.to_vec(), Instant::now()) {
            Some(bytes) => {
                self.to_lspmux(&bytes);
            }
            None if was_holding == 0 => self.gate_note(&format!(
                "readiness gate: holding requests until the server has loaded the workspace \
                 (at most {}s)",
                gate.timeout().as_secs()
            )),
            None => {}
        }
    }

    /// Closes lspmux's input, which ends this session's connection.
    fn close_lspmux(&self) {
        let mut guard = self
            .to_lspmux
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.take();
    }
}

/// How often the gate's clock advances (its grace period and bound).
const GATE_TICK: Duration = Duration::from_millis(100);

fn relay(
    shared: Shared,
    editor: FrameReader<BufReader<io::Stdin>>,
    mut child: Child,
    (to_lspmux, from_lspmux): (ChildStdin, impl Read + Send + 'static),
) -> Result<Outcome, String> {
    let session = Arc::new(Session {
        role: shared.role,
        rewrite: shared.rewrite,
        gate: Mutex::new(shared.gate),
        initialize_id: shared.initialize_id,
        client_asked_status: shared.client_asked_status,
        to_lspmux: Mutex::new(Some(to_lspmux)),
        shutdown_id: Mutex::new(None),
        shut_down: AtomicBool::new(false),
        log: shared.log,
    });
    {
        let session = Arc::clone(&session);
        // Not joined: process exit ends it.
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(GATE_TICK);
                session.gate(|gate| gate.tick(Instant::now()));
            }
        });
    }
    let (events, received) = channel();
    {
        let session = Arc::clone(&session);
        let events = events.clone();
        // Not joined: it may be blocked reading the editor when the session
        // ends, and process exit ends it.
        std::thread::spawn(move || client_to_lspmux(editor, &session, &events));
    }
    {
        let session = Arc::clone(&session);
        std::thread::spawn(move || lspmux_to_client(from_lspmux, &session, &events));
    }
    loop {
        let Ok(event) = received.recv() else {
            return Ok(Outcome::Code(1));
        };
        match event {
            Event::Exit => {
                session.close_lspmux();
                finish(&mut child);
                let asked = session
                    .shutdown_id
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .is_some();
                return Ok(Outcome::Code(u8::from(!asked)));
            }
            Event::EditorEnded => {
                session.close_lspmux();
                finish(&mut child);
                return Ok(Outcome::Code(0));
            }
            Event::LspmuxEnded if session.shut_down.load(Ordering::SeqCst) => {
                // Detached after shutdown: wait for the editor's exit, or
                // for it to kill this process.
            }
            Event::LspmuxEnded => {
                let status = child
                    .wait()
                    .map_err(|error| format!("waiting for lspmux client: {error}"))?;
                eprintln!("codetags-lsp: the lspmux session ended unexpectedly ({status})");
                // TODO(P3.7): crash recovery; until then the editor restarts
                // the server.
                return Ok(if status.success() {
                    Outcome::Code(1)
                } else {
                    Outcome::Status(status)
                });
            }
        }
    }
}

/// Waits briefly for lspmux's client to end on its own, then kills it.
fn finish(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn client_to_lspmux(
    mut editor: FrameReader<BufReader<io::Stdin>>,
    session: &Session,
    events: &Sender<Event>,
) {
    loop {
        let frame = match editor.read_frame() {
            Ok(Some(frame)) => frame,
            Ok(None) | Err(FrameError::Io(_)) => break,
            Err(FrameError::Malformed { raw, .. }) => {
                // No longer framed LSP: pass the rest on unfiltered, as the
                // recorder does, and let lspmux judge it.
                if session.to_lspmux(&raw) {
                    let mut rest = Vec::new();
                    let _ = editor.into_inner().read_to_end(&mut rest);
                    session.to_lspmux(&rest);
                }
                break;
            }
        };
        let message = serde_json::from_slice::<Value>(frame.body()).ok();
        let method = message
            .as_ref()
            .and_then(|message| message.get("method"))
            .and_then(Value::as_str);
        let verdict = policy::from_client(session.role, method);
        let what = match &verdict {
            FromClient::Drop(reason) => Some(*reason),
            FromClient::Exit => Some("exit: the session ends"),
            FromClient::Forward => None,
        };
        note(&session.log, "client", frame.body(), frame.headers(), what);
        match verdict {
            FromClient::Exit => {
                let _ = events.send(Event::Exit);
                return;
            }
            FromClient::Drop(_) => {}
            FromClient::Forward => {
                if method == Some("shutdown")
                    && let Some(id) = message.as_ref().and_then(|message| message.get("id"))
                {
                    *session
                        .shutdown_id
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(id.clone());
                }
                // lspmux gone: keep reading, so `exit` still ends the session.
                let respelled =
                    respelled(session, frame.body(), message.as_ref(), Rewrite::to_server);
                let bytes = respelled.as_deref().unwrap_or(frame.raw());
                let is_request = message
                    .as_ref()
                    .is_some_and(|message| message.get("id").is_some_and(|id| !id.is_null()));
                if method == Some("shutdown") {
                    // Whatever is held goes first; lspmux answers `shutdown`
                    // and detaches the session.
                    session.gate(|gate| ready::Opened {
                        frames: gate.release(),
                        note: None,
                    });
                    session.to_lspmux(bytes);
                } else if method.is_some() && is_request {
                    session.request_through_gate(bytes);
                } else {
                    session.to_lspmux(bytes);
                }
            }
        }
    }
    let _ = events.send(Event::EditorEnded);
}

fn lspmux_to_client(from_lspmux: impl Read, session: &Session, events: &Sender<Event>) {
    let mut reader = FrameReader::new(BufReader::new(from_lspmux));
    let stdout = io::stdout();
    loop {
        let frame = match reader.read_frame() {
            Ok(Some(frame)) => frame,
            Ok(None) | Err(FrameError::Io(_)) => break,
            Err(FrameError::Malformed { raw, .. }) => {
                let mut out = stdout.lock();
                let _ = out.write_all(&raw).and_then(|()| out.flush());
                let _ = record::copy_flushing(reader.into_inner(), &mut out);
                break;
            }
        };
        let message = serde_json::from_slice::<Value>(frame.body()).ok();
        if let Some(message) = &message {
            if is_response(message) && message.get("id") == Some(&session.initialize_id) {
                let reports = ready::reports_status(message);
                session.gate(|gate| gate.initialized(reports, Instant::now()));
            } else if let Some(quiescent) = ready::quiescent(message) {
                session.gate(|gate| gate.status(quiescent, Instant::now()));
                if !session.client_asked_status {
                    note(
                        &session.log,
                        "server",
                        frame.body(),
                        frame.headers(),
                        Some("dropped: the client did not ask for the server's status (P3.12)"),
                    );
                    continue;
                }
            }
        }
        let verdict = message.as_ref().map_or(FromServer::Forward, |message| {
            policy::from_server(session.role, message)
        });
        match verdict {
            FromServer::Answer(answer) => {
                note(
                    &session.log,
                    "server",
                    frame.body(),
                    frame.headers(),
                    Some("answered by the shim"),
                );
                let body = answer.to_string();
                note(&session.log, "shim", body.as_bytes(), &[], Some("answer"));
                let mut bytes = Vec::new();
                let _ = write_frame(&mut bytes, body.as_bytes());
                session.to_lspmux(&bytes);
            }
            FromServer::Forward => {
                note(&session.log, "server", frame.body(), frame.headers(), None);
                let respelled =
                    respelled(session, frame.body(), message.as_ref(), Rewrite::to_client);
                let mut out = stdout.lock();
                if out
                    .write_all(respelled.as_deref().unwrap_or(frame.raw()))
                    .and_then(|()| out.flush())
                    .is_err()
                {
                    break;
                }
                drop(out);
                if let Some(id) = message.as_ref().and_then(|message| message.get("id")) {
                    let asked = session
                        .shutdown_id
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if asked.as_ref() == Some(id) && message.as_ref().is_some_and(is_response) {
                        session.shut_down.store(true, Ordering::SeqCst);
                    }
                }
            }
        }
    }
    let _ = events.send(Event::LspmuxEnded);
}

/// The framed message, respelled by `direction` (D26), if the session
/// rewrites URIs and this message held one under the root.
fn respelled(
    session: &Session,
    body: &[u8],
    message: Option<&Value>,
    direction: fn(&Rewrite, &mut Value) -> bool,
) -> Option<Vec<u8>> {
    let rewrite = session.rewrite.as_ref()?;
    if !uri::may_hold_file_uri(body) {
        return None;
    }
    let mut message = message?.clone();
    if !direction(rewrite, &mut message) {
        return None;
    }
    let mut bytes = Vec::new();
    write_frame(&mut bytes, message.to_string().as_bytes()).ok()?;
    Some(bytes)
}

fn is_response(message: &Value) -> bool {
    message.get("method").is_none()
        && (message.get("result").is_some() || message.get("error").is_some())
}
