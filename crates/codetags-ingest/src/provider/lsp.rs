//! A minimal LSP client, for asking a language server questions at index
//! time (PLAN.md D31): JSON-RPC over stdio with `Content-Length` framing.
//!
//! It is deliberately small and standalone: it links nothing from
//! `codetags-lsp`, and it knows only what the implementation pass needs.
//!
//! - A reader thread decodes the server's messages into a channel, so every
//!   wait has a deadline ([`Client::next`]).
//! - Requests the server sends are answered at once with an empty result
//!   (`null`; for `workspace/configuration`, one `null` per item), so the
//!   server never waits on us. The client advertises no capability that
//!   would make those answers wrong.
//! - Notifications other than the one being waited for are dropped.
//!
//! [`file_uri`] and [`relative_path`] convert between paths and the `file:`
//! URIs a server speaks, without a URL library.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, Prefix};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Instant;

use serde_json::{Value, json};

/// Why a message exchange failed.
#[derive(Debug)]
pub enum LspError {
    /// Writing to the server failed.
    Write(std::io::Error),
    /// The server closed its output, or sent something that is not a framed
    /// JSON-RPC message.
    Closed(String),
    /// The deadline passed first.
    Timeout,
    /// The server answered a request with an error.
    Response {
        /// The request's method.
        method: String,
        /// The error object, as JSON.
        error: String,
    },
}

impl std::fmt::Display for LspError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Write(error) => write!(f, "writing to the server: {error}"),
            Self::Closed(why) => write!(f, "the server's output ended: {why}"),
            Self::Timeout => write!(f, "no answer before the deadline"),
            Self::Response { method, error } => write!(f, "{method} failed: {error}"),
        }
    }
}

impl std::error::Error for LspError {}

/// Reads one framed message from `reader`: `Ok(None)` at a clean end of
/// stream.
pub fn read_message(reader: &mut impl BufRead) -> Result<Option<Value>, String> {
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| format!("reading a header: {error}"))?;
        if read == 0 {
            return if length.is_none() {
                Ok(None)
            } else {
                Err("end of stream inside the headers".to_string())
            };
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|error| format!("bad Content-Length {value:?}: {error}"))?,
            );
        }
    }
    let length = length.ok_or("a message without Content-Length")?;
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .map_err(|error| format!("reading a {length}-byte body: {error}"))?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| format!("a body that is not JSON: {error}"))
}

/// Writes `message` to `writer`, framed, and flushes.
pub fn write_message(writer: &mut impl Write, message: &Value) -> std::io::Result<()> {
    let body = message.to_string();
    write!(writer, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    writer.flush()
}

/// The answer to a request the server sent us: `null`, or one `null` per
/// item for `workspace/configuration`.
fn answer_for(request: &Value) -> Value {
    let result = if request["method"] == "workspace/configuration" {
        let items = request["params"]["items"].as_array().map_or(0, Vec::len);
        Value::Array(vec![Value::Null; items])
    } else {
        Value::Null
    };
    json!({"jsonrpc": "2.0", "id": request["id"], "result": result})
}

/// A connection to a language server.
pub struct Client<W: Write> {
    writer: W,
    incoming: Receiver<Result<Value, String>>,
    next_id: i64,
}

impl<W: Write> Client<W> {
    /// A client writing to `writer` and reading from `reader`, which a
    /// thread drains until it ends.
    pub fn new(writer: W, reader: impl Read + Send + 'static) -> Self {
        let (send, incoming) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(reader);
            loop {
                match read_message(&mut reader) {
                    Ok(Some(message)) => {
                        if send.send(Ok(message)).is_err() {
                            return;
                        }
                    }
                    Ok(None) => {
                        let _ = send.send(Err("end of stream".to_string()));
                        return;
                    }
                    Err(error) => {
                        let _ = send.send(Err(error));
                        return;
                    }
                }
            }
        });
        Self {
            writer,
            incoming,
            next_id: 0,
        }
    }

    /// Sends a notification.
    pub fn notify(&mut self, method: &str, params: Value) -> Result<(), LspError> {
        write_message(
            &mut self.writer,
            &json!({"jsonrpc": "2.0", "method": method, "params": params}),
        )
        .map_err(LspError::Write)
    }

    /// Sends a request and returns its id.
    pub fn send_request(&mut self, method: &str, params: Value) -> Result<i64, LspError> {
        self.next_id += 1;
        let id = self.next_id;
        write_message(
            &mut self.writer,
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )
        .map_err(LspError::Write)?;
        Ok(id)
    }

    /// The next message that is not a request from the server (those are
    /// answered here), before `deadline`.
    pub fn next(&mut self, deadline: Instant) -> Result<Value, LspError> {
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let message = match self.incoming.recv_timeout(left) {
                Ok(Ok(message)) => message,
                Ok(Err(why)) => return Err(LspError::Closed(why)),
                Err(RecvTimeoutError::Timeout) => return Err(LspError::Timeout),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(LspError::Closed("the reader stopped".to_string()));
                }
            };
            if message.get("method").is_some() && message.get("id").is_some() {
                write_message(&mut self.writer, &answer_for(&message)).map_err(LspError::Write)?;
                continue;
            }
            return Ok(message);
        }
    }

    /// Sends a request and waits for its response's result, dropping
    /// notifications meanwhile.
    pub fn request(
        &mut self,
        method: &str,
        params: Value,
        deadline: Instant,
    ) -> Result<Value, LspError> {
        let id = self.send_request(method, params)?;
        loop {
            let message = self.next(deadline)?;
            if message.get("method").is_some() || message["id"] != id {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(LspError::Response {
                    method: method.to_string(),
                    error: error.to_string(),
                });
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

/// Whether `c` is left as is in a URI path.
fn unreserved(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~' | b'/' | b':')
}

/// `path`, absolute, with `/` separators and without Windows' verbatim
/// `\\?\` prefix: `C:/x/y` or `/x/y`.
fn slashed(path: &Path) -> String {
    let mut out = String::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => match prefix.kind() {
                Prefix::VerbatimDisk(disk) | Prefix::Disk(disk) => {
                    out.push(char::from(disk));
                    out.push(':');
                }
                _ => out.push_str(&prefix.as_os_str().to_string_lossy().replace('\\', "/")),
            },
            Component::RootDir => out.push('/'),
            Component::Normal(name) => {
                if !out.ends_with('/') {
                    out.push('/');
                }
                out.push_str(&name.to_string_lossy());
            }
            Component::CurDir | Component::ParentDir => {
                if !out.ends_with('/') {
                    out.push('/');
                }
                out.push_str(&component.as_os_str().to_string_lossy());
            }
        }
    }
    out
}

/// The `file:` URI of the absolute `path`, percent-encoded.
pub fn file_uri(path: &Path) -> String {
    let mut slashed = slashed(path);
    if !slashed.starts_with('/') {
        slashed.insert(0, '/');
    }
    let mut uri = String::from("file://");
    for byte in slashed.bytes() {
        if unreserved(byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// `text` with `%XX` escapes decoded; `None` if they do not decode to UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A path spelling for comparison: a Windows drive letter lowercased, as
/// rust-analyzer writes it in URIs.
fn comparable(path: &str) -> String {
    let mut path = path.to_string();
    if path.as_bytes().get(1) == Some(&b':') {
        path[..1].make_ascii_lowercase();
    }
    path
}

/// The path of the `file:` URI `uri` relative to `root`, with `/`
/// separators, as SCIP documents name files; `None` for a URI outside
/// `root` or not a `file:` URI.
pub fn relative_path(uri: &str, root: &Path) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let mut path = percent_decode(rest)?;
    // `/C:/x` is the drive path `C:/x`.
    if path.as_bytes().get(2) == Some(&b':') && path.starts_with('/') {
        path.remove(0);
    }
    let path = comparable(&path);
    let root = comparable(slashed(root).trim_end_matches('/'));
    let relative = path.strip_prefix(&root)?.strip_prefix('/')?;
    (!relative.is_empty()).then(|| relative.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::time::Duration;

    use super::*;

    #[test]
    fn messages_round_trip_through_the_framing() {
        let mut bytes = Vec::new();
        write_message(&mut bytes, &json!({"a": "é"})).unwrap();
        write_message(&mut bytes, &json!([1, 2])).unwrap();
        assert!(bytes.starts_with(b"Content-Length: 10\r\n\r\n"));
        let mut reader = BufReader::new(bytes.as_slice());
        assert_eq!(read_message(&mut reader).unwrap(), Some(json!({"a": "é"})));
        assert_eq!(read_message(&mut reader).unwrap(), Some(json!([1, 2])));
        assert_eq!(read_message(&mut reader).unwrap(), None);
    }

    #[test]
    fn bad_frames_are_errors() {
        let mut reader = BufReader::new(&b"Content-Type: x\r\n\r\n{}"[..]);
        assert!(read_message(&mut reader).is_err());
        let mut reader = BufReader::new(&b"Content-Length: 9\r\n\r\n{}"[..]);
        assert!(read_message(&mut reader).is_err());
        let mut reader = BufReader::new(&b"Content-Length: 2\r\n"[..]);
        assert!(read_message(&mut reader).is_err());
    }

    /// A scripted server on the other end of two pipes: it reads each
    /// message the client sends and replies with `script`'s answer.
    fn scripted(
        script: impl Fn(&Value) -> Vec<Value> + Send + 'static,
    ) -> (
        Client<std::io::PipeWriter>,
        std::thread::JoinHandle<Vec<Value>>,
    ) {
        let (client_reads, server_writes) = std::io::pipe().unwrap();
        let (server_reads, client_writes) = std::io::pipe().unwrap();
        let server = std::thread::spawn(move || {
            let mut reader = BufReader::new(server_reads);
            let mut writer = server_writes;
            let mut seen = Vec::new();
            while let Ok(Some(message)) = read_message(&mut reader) {
                for reply in script(&message) {
                    write_message(&mut writer, &reply).unwrap();
                }
                seen.push(message);
            }
            seen
        });
        (Client::new(client_writes, client_reads), server)
    }

    #[test]
    fn a_request_gets_its_own_response_and_server_requests_are_answered() {
        let (mut client, server) = scripted(|message| {
            if message["method"] == "ask" {
                vec![
                    json!({"jsonrpc": "2.0", "method": "note", "params": {}}),
                    json!({"jsonrpc": "2.0", "id": "s1", "method": "workspace/configuration",
                           "params": {"items": [{}, {}]}}),
                    json!({"jsonrpc": "2.0", "id": 99, "result": "not mine"}),
                    json!({"jsonrpc": "2.0", "id": message["id"], "result": [1]}),
                ]
            } else {
                Vec::new()
            }
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        assert_eq!(
            client.request("ask", json!({}), deadline).unwrap(),
            json!([1])
        );
        drop(client);
        let seen = server.join().unwrap();
        assert_eq!(seen[1]["id"], "s1");
        assert_eq!(seen[1]["result"], json!([null, null]));
    }

    #[test]
    fn an_error_response_and_a_silent_server_fail() {
        let (mut client, _server) = scripted(|message| {
            if message["method"] == "bad" {
                vec![json!({"jsonrpc": "2.0", "id": message["id"],
                            "error": {"code": -32601, "message": "no"}})]
            } else {
                Vec::new()
            }
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let error = client.request("bad", json!(null), deadline).unwrap_err();
        assert!(error.to_string().contains("-32601"), "{error}");
        let soon = Instant::now() + Duration::from_millis(50);
        assert!(matches!(
            client.request("silence", json!(null), soon),
            Err(LspError::Timeout)
        ));
    }

    #[test]
    fn a_closed_server_is_an_error() {
        let (client_reads, server_writes) = std::io::pipe().unwrap();
        drop(server_writes);
        let mut client = Client::new(Vec::new(), client_reads);
        let deadline = Instant::now() + Duration::from_secs(10);
        assert!(matches!(client.next(deadline), Err(LspError::Closed(_))));
    }

    #[test]
    fn uris_round_trip_relative_to_a_root() {
        let root = if cfg!(windows) {
            Path::new(r"C:\work\my project")
        } else {
            Path::new("/work/my project")
        };
        let file = root.join("src").join("a b.rs");
        let uri = file_uri(&file);
        if cfg!(windows) {
            assert_eq!(uri, "file:///C:/work/my%20project/src/a%20b.rs");
        } else {
            assert_eq!(uri, "file:///work/my%20project/src/a%20b.rs");
        }
        assert_eq!(relative_path(&uri, root).as_deref(), Some("src/a b.rs"));
        assert_eq!(relative_path("file:///elsewhere/x.rs", root), None);
        assert_eq!(relative_path("untitled:x", root), None);
    }

    #[test]
    fn a_lowercased_drive_letter_and_escaped_colon_still_match() {
        let root = Path::new(r"C:\work");
        if cfg!(windows) {
            assert_eq!(
                relative_path("file:///c%3A/work/src/x.rs", root).as_deref(),
                Some("src/x.rs")
            );
            let verbatim = Path::new(r"\\?\C:\work");
            assert_eq!(file_uri(&verbatim.join("x.rs")), "file:///C:/work/x.rs");
        }
        assert_eq!(comparable("C:/a"), "c:/a");
        assert_eq!(percent_decode("a%20b%3A").as_deref(), Some("a b:"));
        assert_eq!(percent_decode("%zz"), None);
    }
}
