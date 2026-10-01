//! Steps for `features/lsp/`: `codetags-lsp record` between a scripted client
//! and a fake language server (M3 stage 0, PLAN.md D18).
//!
//! The fake server is this test executable in child mode `fake-lsp`
//! ([`fake_server`]). It frames messages with its own small reader, not
//! `codetags-lsp`'s, so the two cannot share a bug.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use cucumber::{given, then, when};

use crate::{CODETAGS_BIN, CodetagsWorld};

/// The child mode that turns this executable into the fake server.
pub(crate) const CHILD_MODE: &str = "fake-lsp";
/// File the fake server writes every byte it received to.
const IN_ENV: &str = "CODETAGS_BDD_FAKE_LSP_IN";
/// File the fake server writes every byte it sent to.
const OUT_ENV: &str = "CODETAGS_BDD_FAKE_LSP_OUT";
/// The fake server's exit status, for `exit`, end of input, and `--version`.
const STATUS_ENV: &str = "CODETAGS_BDD_FAKE_LSP_STATUS";
const MODE_ENV: &str = "CODETAGS_BDD_CHILD";

/// What an `features/lsp` scenario set up and observed.
#[derive(Debug, Default)]
pub(crate) struct LspState {
    /// Exit status the fake server uses; `None` until a `Given` step.
    status: Option<i32>,
    /// The bytes the scripted client sent.
    sent: Vec<u8>,
    /// The bytes the scripted client received.
    received: Vec<u8>,
    /// Where the fake server wrote what it received and sent, and the log.
    fake_in: Option<PathBuf>,
    fake_out: Option<PathBuf>,
    log: Option<PathBuf>,
}

/// The `codetags-lsp` binary, built beside `codetags` (`just bdd` builds
/// every workspace binary first).
fn lsp_binary() -> PathBuf {
    let codetags = CODETAGS_BIN.get().expect("run() sets the binary path");
    let binary = codetags.with_file_name(format!("codetags-lsp{}", std::env::consts::EXE_SUFFIX));
    assert!(
        binary.exists(),
        "{} is missing; build it with `cargo build --workspace --bins` (just bdd does)",
        binary.display()
    );
    binary
}

/// One framed message: the header block as given, then `body`.
fn frame(headers: &str, body: &str) -> Vec<u8> {
    let mut out = headers
        .replace("{len}", &body.len().to_string())
        .into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

const PLAIN: &str = "Content-Length: {len}\r\n\r\n";
const TYPED: &str =
    "Content-Length: {len}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n";
const TYPE_FIRST: &str =
    "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\nContent-Length: {len}\r\n\r\n";

/// The scripted client's messages. Bodies use spacing and key order no
/// serializer would produce, and non-ASCII text, so re-serializing or
/// counting characters instead of bytes shows up as a difference.
fn client_script() -> Vec<u8> {
    [
        frame(
            TYPED,
            r#"{ "jsonrpc" : "2.0", "id" : 1, "method" : "initialize", "params" : { "rootUri" : null, "clientInfo" : { "name" : "scripted client é" }, "capabilities" : {} } }"#,
        ),
        frame(
            PLAIN,
            r#"{"method":"initialized","jsonrpc":"2.0","params":{}}"#,
        ),
        frame(PLAIN, r#"{"jsonrpc":"2.0","id":900,"result":null}"#),
        frame(
            PLAIN,
            r#"{"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}"#,
        ),
        frame(PLAIN, r#"{"jsonrpc":"2.0","method":"exit"}"#),
    ]
    .concat()
}

#[given("a fake language server")]
fn a_fake_language_server(world: &mut CodetagsWorld) {
    world.lsp.status = Some(0);
}

#[given(expr = "a fake language server that exits with status {int}")]
fn a_fake_language_server_that_exits_with(world: &mut CodetagsWorld, status: i32) {
    world.lsp.status = Some(status);
}

/// `codetags-lsp record --server <fake> --log <log> -- args`, with the fake
/// server's environment set; the caller adds stdio and runs it.
fn record_command(world: &mut CodetagsWorld, args: &[&str]) -> Command {
    let status = world
        .lsp
        .status
        .expect("a Given step set up the fake server");
    let scratch = world.scratch().to_path_buf();
    let (fake_in, fake_out, log) = (
        scratch.join("fake-in.bin"),
        scratch.join("fake-out.bin"),
        scratch.join("recording.jsonl"),
    );
    let fake = std::env::current_exe().expect("locate the test executable");
    let mut command = Command::new(lsp_binary());
    command
        .arg("record")
        .arg("--server")
        .arg(&fake)
        .arg("--log")
        .arg(&log)
        .arg("--")
        .args(args)
        .env(MODE_ENV, CHILD_MODE)
        .env(IN_ENV, &fake_in)
        .env(OUT_ENV, &fake_out)
        .env(STATUS_ENV, status.to_string());
    world.lsp.fake_in = Some(fake_in);
    world.lsp.fake_out = Some(fake_out);
    world.lsp.log = Some(log);
    command
}

#[when("a scripted LSP session runs through codetags-lsp record")]
fn a_scripted_session_runs(world: &mut CodetagsWorld) {
    let mut command = record_command(world, &[]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn codetags-lsp record");
    let script = client_script();
    let mut stdin = child.stdin.take().expect("piped stdin");
    let writer = std::thread::spawn(move || {
        // The recorder may exit before reading everything if it fails; the
        // assertions on the bytes then report it.
        let _ = stdin.write_all(&script);
    });
    let output = child.wait_with_output().expect("wait for codetags-lsp");
    writer.join().expect("the writer thread does not panic");
    world.lsp.sent = client_script();
    world.lsp.received = output.stdout.clone();
    world.last = Some(output.into());
}

#[when(expr = "codetags-lsp record runs the fake server with the arguments {string}")]
fn record_runs_with_arguments(world: &mut CodetagsWorld, args: String) {
    let args: Vec<&str> = args.split_whitespace().collect();
    let mut command = record_command(world, &args);
    command.stdin(Stdio::null());
    let output = command.output().expect("run codetags-lsp record");
    world.last = Some(output.into());
}

fn read(path: &Option<PathBuf>) -> Vec<u8> {
    let path = path.as_ref().expect("a When step ran the recorder");
    std::fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

#[then("the fake server received exactly the bytes the client sent")]
fn server_received_client_bytes(world: &mut CodetagsWorld) {
    let received = read(&world.lsp.fake_in);
    assert!(
        received == world.lsp.sent,
        "the fake server received {:?}\nthe client sent {:?}\nstderr: {}",
        String::from_utf8_lossy(&received),
        String::from_utf8_lossy(&world.lsp.sent),
        world.last().stderr
    );
}

#[then("the client received exactly the bytes the fake server sent")]
fn client_received_server_bytes(world: &mut CodetagsWorld) {
    let sent = read(&world.lsp.fake_out);
    assert!(
        sent == world.lsp.received,
        "the client received {:?}\nthe fake server sent {:?}\nstderr: {}",
        String::from_utf8_lossy(&world.lsp.received),
        String::from_utf8_lossy(&sent),
        world.last().stderr
    );
}

/// The log's message records (those with a `from` field), parsed.
fn messages(world: &CodetagsWorld) -> Vec<serde_json::Value> {
    let log = read(&world.lsp.log);
    String::from_utf8(log)
        .expect("the recording is UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line of the recording is JSON"))
        .filter(|record: &serde_json::Value| record.get("from").is_some())
        .collect()
}

#[then(expr = "the recording holds {int} messages")]
fn the_recording_holds(world: &mut CodetagsWorld, count: usize) {
    let messages = messages(world);
    assert_eq!(messages.len(), count, "recorded messages: {messages:#?}");
}

#[then(expr = "the recording shows {string} sent by the {word}")]
fn the_recording_shows(world: &mut CodetagsWorld, what: String, from: String) {
    let messages = messages(world);
    let found = messages
        .iter()
        .filter(|record| record["from"] == from.as_str())
        .filter(|record| match what.strip_prefix("response ") {
            Some(id) => {
                let id: serde_json::Value = serde_json::from_str(id).expect("the id is JSON");
                record["kind"] == "response" && record["msg"]["id"] == id
            }
            None => record["method"] == what.as_str(),
        });
    assert_eq!(
        found.count(),
        1,
        "expected exactly one {what:?} from the {from}; recorded: {messages:#?}"
    );
}

#[then("no recording was written")]
fn no_recording_was_written(world: &mut CodetagsWorld) {
    let log = world
        .lsp
        .log
        .as_ref()
        .expect("a When step ran the recorder");
    assert!(!log.exists(), "{} exists", log.display());
}

/// Child mode [`CHILD_MODE`]: a tiny language server. `--version` prints
/// `fake-lsp 9.9.9`. Otherwise it answers `initialize` (then sends a
/// `window/logMessage` notification and a `client/registerCapability`
/// request with id 900) and `shutdown`, and exits on `exit` or end of input.
/// It always exits with the status in [`STATUS_ENV`], after writing every
/// byte it received and sent to the files in [`IN_ENV`] and [`OUT_ENV`].
pub(crate) fn fake_server() -> ! {
    let status: i32 = std::env::var(STATUS_ENV)
        .ok()
        .and_then(|status| status.parse().ok())
        .unwrap_or(0);
    if std::env::args().skip(1).any(|arg| arg == "--version") {
        println!("fake-lsp 9.9.9");
        std::process::exit(status);
    }
    let mut stdin = BufReader::new(std::io::stdin().lock());
    let mut stdout = std::io::stdout().lock();
    let (mut received, mut sent) = (Vec::new(), Vec::new());
    let mut send = |bytes: Vec<u8>, sent: &mut Vec<u8>| {
        stdout.write_all(&bytes).expect("write to stdout");
        stdout.flush().expect("flush stdout");
        sent.extend_from_slice(&bytes);
    };
    while let Some(body) = fake_read(&mut stdin, &mut received) {
        let message: serde_json::Value = serde_json::from_slice(&body).expect("JSON body");
        match message["method"].as_str() {
            Some("initialize") => {
                send(
                    frame(
                        TYPE_FIRST,
                        r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}, "serverInfo":{"name":"fake-lsp é"}}}"#,
                    ),
                    &mut sent,
                );
                send(
                    frame(
                        PLAIN,
                        r#"{"jsonrpc":"2.0","method":"window/logMessage","params":{"type":3,"message":"ready"}}"#,
                    ),
                    &mut sent,
                );
                send(
                    frame(
                        PLAIN,
                        r#"{"jsonrpc":"2.0","id":900,"method":"client/registerCapability","params":{"registrations":[]}}"#,
                    ),
                    &mut sent,
                );
            }
            Some("shutdown") => send(
                frame(PLAIN, r#"{"jsonrpc":"2.0","id":2,"result":null}"#),
                &mut sent,
            ),
            Some("exit") => break,
            _ => {}
        }
    }
    for (env, bytes) in [(IN_ENV, &received), (OUT_ENV, &sent)] {
        let path = std::env::var_os(env).expect("the step sets the fake server's files");
        std::fs::write(&path, bytes).expect("write the fake server's record");
    }
    std::process::exit(status)
}

/// Reads one message, appending its raw bytes to `raw`; `None` at the end
/// of input.
fn fake_read(input: &mut impl BufRead, raw: &mut Vec<u8>) -> Option<Vec<u8>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).expect("read a header line") == 0 {
            return None;
        }
        raw.extend_from_slice(line.as_bytes());
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = Some(value.trim().parse::<usize>().expect("a numeric length"));
        }
    }
    let mut body = vec![0; length.expect("a Content-Length header")];
    input.read_exact(&mut body).expect("read the body");
    raw.extend_from_slice(&body);
    Some(body)
}
