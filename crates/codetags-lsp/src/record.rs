//! `codetags-lsp record`: run a real language server, pass its stdio through
//! unchanged, and log every message as one JSON line (M3 stage 0, D18).
//!
//! Log records, one JSON object per line, all with `ts_ms` (Unix time in
//! milliseconds) and `t_ms` (milliseconds since the recorder started):
//!
//! - `{"event":"start","server":…,"args":[…],"pid":…,"recorder_pid":…,"cwd":…,"format":1}`
//! - one per message: `{"from":"client"|"server","kind":"request"|"response"|"notification"|"batch"|"invalid","method":…,"id":…,"bytes":…,"msg":…}`,
//!   where `msg` is the parsed body (`null`, plus `raw` and `parse_error`,
//!   if the body is not JSON) and `headers` lists the headers when there are
//!   any besides `Content-Length`;
//! - `{"event":"malformed","side":…,"reason":…,"raw":…}` when a stream stops
//!   being framed LSP; the rest of it is copied unrecorded;
//! - `{"event":"eof","side":…}` when a side closes its output, with `error`
//!   if passing it on failed;
//! - `{"event":"exit","status":…,"signal":…}` when the server has exited.
//!
//! The editor sees the server's bytes, stderr and exit status unchanged.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use crate::framing::{Frame, FrameError, FrameReader};
use crate::invocation::is_lsp_session;
use crate::logpath;
use crate::message::{Kind, Side};

/// What `record` runs, and where it logs.
#[derive(Debug, Clone)]
pub struct Options {
    /// The real server binary.
    pub server: PathBuf,
    /// Arguments for the server.
    pub args: Vec<OsString>,
    /// The log file; `{ts}` and `{pid}` are expanded ([`logpath::expand`]).
    pub log: PathBuf,
    /// Subcommands that still start an LSP session ([`is_lsp_session`]).
    pub lsp_subcommands: Vec<String>,
}

/// Runs the server as `options` say and returns its exit status. A non-LSP
/// invocation runs the server with this process's stdio and writes no log.
pub fn run(options: &Options) -> Result<ExitStatus, String> {
    let mut command = Command::new(&options.server);
    command.args(&options.args);
    if !is_lsp_session(&options.args, &options.lsp_subcommands) {
        return command
            .status()
            .map_err(|error| format!("cannot run {}: {error}", options.server.display()));
    }
    let log_path = logpath::expand(&options.log, unix_ms() / 1000, std::process::id());
    let log = Arc::new(Log::create(&log_path)?);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("cannot start {}: {error}", options.server.display()))?;
    log.write(json!({
        "event": "start",
        "format": 1,
        "server": options.server.to_string_lossy(),
        "args": options.args.iter().map(|arg| arg.to_string_lossy()).collect::<Vec<_>>(),
        "pid": child.id(),
        "recorder_pid": std::process::id(),
        "cwd": std::env::current_dir().map(|dir| dir.to_string_lossy().into_owned()).ok(),
    }));
    let (Some(to_server), Some(from_server)) = (child.stdin.take(), child.stdout.take()) else {
        return Err("the server's stdio was not piped".to_string());
    };
    let up_log = Arc::clone(&log);
    // Not joined: it may be blocked reading the editor's stdin when the
    // server exits, and process exit ends it.
    std::thread::spawn(move || {
        let result = pump(io::stdin().lock(), to_server, Side::Client, &up_log);
        up_log.eof(Side::Client, result.err());
    });
    let down_log = Arc::clone(&log);
    let down = std::thread::spawn(move || {
        let result = pump(from_server, io::stdout().lock(), Side::Server, &down_log);
        down_log.eof(Side::Server, result.err());
    });
    let status = child
        .wait()
        .map_err(|error| format!("waiting for the server failed: {error}"))?;
    // The server's stdout is closed once it has exited (unless a process it
    // started holds it); drain it, so the editor gets every byte.
    let _ = down.join();
    log.write(json!({
        "event": "exit",
        "status": status.code(),
        "signal": signal_of(status),
    }));
    Ok(status)
}

#[cfg(unix)]
fn signal_of(status: ExitStatus) -> Option<i32> {
    std::os::unix::process::ExitStatusExt::signal(&status)
}

#[cfg(not(unix))]
fn signal_of(_status: ExitStatus) -> Option<i32> {
    None
}

/// Copies framed messages from `input` to `output`, logging each, until
/// `input` ends. Each message is flushed as soon as it is written. If the
/// input stops being framed LSP, what was read is passed on and logged as
/// malformed, and the rest is copied unframed and unlogged.
pub fn pump(input: impl Read, mut output: impl Write, side: Side, log: &Log) -> io::Result<()> {
    let mut reader = FrameReader::new(BufReader::new(input));
    loop {
        match reader.read_frame() {
            Ok(Some(frame)) => {
                log.message(side, &frame);
                output.write_all(frame.raw())?;
                output.flush()?;
            }
            Ok(None) => return Ok(()),
            Err(FrameError::Io(error)) => return Err(error),
            Err(FrameError::Malformed { raw, reason }) => {
                log.write(json!({
                    "event": "malformed",
                    "side": side.as_str(),
                    "reason": reason,
                    "raw": String::from_utf8_lossy(&raw),
                }));
                output.write_all(&raw)?;
                output.flush()?;
                return copy_flushing(reader.into_inner(), output);
            }
        }
    }
}

pub(crate) fn copy_flushing(mut input: impl Read, mut output: impl Write) -> io::Result<()> {
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = match input.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        output.write_all(&buffer[..read])?;
        output.flush()?;
    }
}

/// The JSON-lines log. Each record is written with one unbuffered write, so
/// a killed recorder loses nothing it had logged. A failing write is
/// reported once on stderr and never stops the session.
#[derive(Debug)]
pub struct Log {
    file: Mutex<File>,
    start: Instant,
    reported: std::sync::atomic::AtomicBool,
    path: PathBuf,
}

impl Log {
    /// Creates (or appends to) the log at `path`, creating its directory.
    pub fn create(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
        Ok(Self {
            file: Mutex::new(file),
            start: Instant::now(),
            reported: std::sync::atomic::AtomicBool::new(false),
            path: path.to_path_buf(),
        })
    }

    /// Appends `record`, with `ts_ms` and `t_ms` added first.
    pub fn write(&self, record: Value) {
        let mut line = Map::new();
        line.insert("ts_ms".into(), json!(unix_ms()));
        let elapsed = self.start.elapsed().as_secs_f64() * 1000.0;
        line.insert("t_ms".into(), json!((elapsed * 1000.0).round() / 1000.0));
        if let Value::Object(fields) = record {
            line.extend(fields);
        }
        let mut text = Value::Object(line).to_string();
        text.push('\n');
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Err(error) = file.write_all(text.as_bytes())
            && !self
                .reported
                .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            eprintln!(
                "codetags-lsp: cannot write the log {}: {error}",
                self.path.display()
            );
        }
    }

    /// Logs one message `side` sent.
    pub fn message(&self, side: Side, frame: &Frame) {
        self.body(side.as_str(), frame.body(), frame.headers(), None);
    }

    /// Logs one message `from` sent (`"client"`, `"server"`, or `"shim"`
    /// for a message the shim wrote itself), with its headers, and with
    /// `"shim": note` when the shim did something other than pass it on.
    pub fn body(&self, from: &str, body: &[u8], headers: &[(String, String)], note: Option<&str>) {
        let mut record = Map::new();
        record.insert("from".into(), json!(from));
        if let Some(note) = note {
            record.insert("shim".into(), json!(note));
        }
        match serde_json::from_slice::<Value>(body) {
            Ok(message) => {
                let kind = Kind::of(&message);
                record.insert("kind".into(), json!(kind.as_str()));
                if let Some(method) = message.get("method") {
                    record.insert("method".into(), method.clone());
                }
                if let Some(id) = message.get("id") {
                    record.insert("id".into(), id.clone());
                }
                record.insert("bytes".into(), json!(body.len()));
                record.insert("msg".into(), message);
            }
            Err(error) => {
                record.insert("kind".into(), json!(Kind::Invalid.as_str()));
                record.insert("bytes".into(), json!(body.len()));
                record.insert("msg".into(), Value::Null);
                record.insert("parse_error".into(), json!(error.to_string()));
                record.insert("raw".into(), json!(String::from_utf8_lossy(body)));
            }
        }
        if headers
            .iter()
            .any(|(name, _)| !name.eq_ignore_ascii_case("content-length"))
        {
            record.insert("headers".into(), json!(headers));
        }
        self.write(Value::Object(record));
    }

    /// Logs that `side`'s stream ended, and why if it failed.
    pub fn eof(&self, side: Side, error: Option<io::Error>) {
        self.write(json!({
            "event": "eof",
            "side": side.as_str(),
            "error": error.map(|error| error.to_string()),
        }));
    }
}

pub(crate) fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::Value;

    use super::{Log, pump};
    use crate::message::Side;

    fn records(path: &std::path::Path) -> Vec<Value> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn pump_passes_bytes_through_and_logs_each_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("log.jsonl");
        let log = Log::create(&path).unwrap();
        let input = b"Content-Length: 37\r\n\r\n{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"a\"}Content-Length: 4\r\n\r\nnope".to_vec();
        let mut output = Vec::new();
        pump(&input[..], &mut output, Side::Client, &log).unwrap();
        assert_eq!(output, input);
        let records = records(&path);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["from"], "client");
        assert_eq!(records[0]["kind"], "request");
        assert_eq!(records[0]["method"], "a");
        assert_eq!(records[0]["id"], 7);
        assert_eq!(records[1]["kind"], "invalid");
        assert_eq!(records[1]["raw"], "nope");
        assert!(records[0]["ts_ms"].is_u64() && records[0]["t_ms"].is_number());
    }

    #[test]
    fn malformed_input_is_still_passed_through() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let log = Log::create(&path).unwrap();
        let input = b"Content-Length: 2\r\n\r\n{}garbage without framing\nmore".to_vec();
        let mut output = Vec::new();
        pump(&input[..], &mut output, Side::Server, &log).unwrap();
        assert_eq!(output, input);
        let records = records(&path);
        assert_eq!(records.len(), 2);
        assert_eq!(records[1]["event"], "malformed");
        assert_eq!(records[1]["side"], "server");
    }
}
