//! Asking rust-analyzer for the implementations of trait methods, over LSP
//! (PLAN.md D31; V64, V160).
//!
//! rust-analyzer writes no `is_implementation` relationships into SCIP
//! (V64), so the Rust provider runs it a second time, as a language server
//! on the project, and asks `textDocument/implementation` at each trait
//! method's definition. [`RustAnalyzer::implementations`]:
//!
//! 1. starts the same program as the SCIP run, with the same environment,
//!    its stderr written to a file;
//! 2. initializes it with the project root, UTF-8 positions (SCIP's, V65),
//!    `experimental.serverStatusNotification`, cache priming and check on
//!    save off;
//! 3. waits until it reports the workspace loaded: an
//!    `experimental/serverStatus` notification with `quiescent: true`, which
//!    comes only after the workspace, build scripts and proc macros are
//!    loaded (V160). An unhealthy status (`health: error`) fails, and so
//!    does a server that is not ready within the timeout: the run is loud,
//!    like every provider failure;
//! 4. asks each question, answering a `ContentModified` error by asking
//!    again (V130), and shuts the server down.
//!
//! The standalone server is started per run. Asking the warm shared server
//! that the LSP shim keeps (PLAN.md §11) is a later optimization.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::RustAnalyzer;
use crate::provider::lsp::{Client, LspError, file_uri, relative_path};

/// How long one `textDocument/implementation` request may take once the
/// workspace is loaded.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(120);

/// How many times a request answered with `ContentModified` is sent again.
const CONTENT_MODIFIED_RETRIES: usize = 5;

/// LSP's `ContentModified` error code.
const CONTENT_MODIFIED: i64 = -32801;

/// A position in a project file: its root-relative path with `/`
/// separators, and a 0-based line and UTF-8 column, as SCIP gives them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    /// The file, relative to the root.
    pub path: String,
    /// 0-based line.
    pub line: u32,
    /// 0-based column, in UTF-8 bytes.
    pub character: u32,
}

/// What rust-analyzer answered.
#[derive(Debug, Clone, Default)]
pub struct Answers {
    /// For each question, in order, the implementations inside the project.
    pub locations: Vec<Vec<Position>>,
    /// Locations outside the project root (a dependency's or the standard
    /// library's), which are dropped.
    pub outside_root: usize,
    /// How long the server took to report the workspace loaded.
    pub load_time: Duration,
    /// How long it then took to answer every question.
    pub answer_time: Duration,
}

/// Why the implementation pass failed. Every variant that started the
/// server carries the end of its stderr.
#[derive(Debug)]
pub enum ImplError {
    /// The server could not be started.
    Spawn {
        /// The program.
        program: String,
        /// Why.
        source: std::io::Error,
    },
    /// The server did not report the workspace loaded within the timeout.
    NotReady {
        /// The timeout.
        timeout: Duration,
        /// The end of its stderr.
        stderr: String,
    },
    /// The server reported the workspace loaded but unhealthy.
    Unhealthy {
        /// Its status message.
        message: String,
        /// The end of its stderr.
        stderr: String,
    },
    /// The server does not use UTF-8 positions, which SCIP's are.
    Encoding {
        /// The encoding it chose.
        found: String,
        /// The end of its stderr.
        stderr: String,
    },
    /// The exchange with the server failed.
    Lsp {
        /// What was being done.
        stage: &'static str,
        /// Why.
        source: LspError,
        /// The end of its stderr.
        stderr: String,
    },
}

impl ImplError {
    fn stderr(&self) -> Option<&str> {
        match self {
            Self::Spawn { .. } => None,
            Self::NotReady { stderr, .. }
            | Self::Unhealthy { stderr, .. }
            | Self::Encoding { stderr, .. }
            | Self::Lsp { stderr, .. } => Some(stderr),
        }
    }
}

impl std::fmt::Display for ImplError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn { program, source } => {
                return write!(f, "rust-analyzer {program} could not start: {source}");
            }
            Self::NotReady { timeout, .. } => write!(
                f,
                "rust-analyzer did not finish loading the workspace within {} s",
                timeout.as_secs()
            )?,
            Self::Unhealthy { message, .. } => {
                write!(f, "rust-analyzer could not load the workspace: {message}")?
            }
            Self::Encoding { found, .. } => write!(
                f,
                "rust-analyzer chose the position encoding {found:?}, not UTF-8"
            )?,
            Self::Lsp { stage, source, .. } => write!(f, "rust-analyzer {stage}: {source}")?,
        }
        if let Some(stderr) = self.stderr().filter(|s| !s.trim().is_empty()) {
            write!(f, "\n--- rust-analyzer stderr ---\n{}", stderr.trim_end())?;
        }
        Ok(())
    }
}

impl std::error::Error for ImplError {}

/// The last 4 KiB or so of the file at `path`, lossily decoded.
fn tail(path: &Path) -> String {
    const KEEP: usize = 4096;
    let bytes = std::fs::read(path).unwrap_or_default();
    let start = bytes.len().saturating_sub(KEEP);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

/// Kills the server when dropped, unless it already exited.
struct Server {
    child: Child,
}

impl Drop for Server {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

/// The implementations in an implementation response: `null`, a
/// `Location`, or an array of `Location` or `LocationLink` (for a link, its
/// target selection range, the name).
fn response_locations(result: &Value) -> Vec<(String, u32, u32)> {
    let one = |value: &Value| {
        let (uri, range) = match value.get("targetUri") {
            Some(uri) => (uri, &value["targetSelectionRange"]),
            None => (&value["uri"], &value["range"]),
        };
        let line = range["start"]["line"].as_u64()?;
        let character = range["start"]["character"].as_u64()?;
        Some((
            uri.as_str()?.to_string(),
            u32::try_from(line).ok()?,
            u32::try_from(character).ok()?,
        ))
    };
    match result {
        Value::Array(items) => items.iter().filter_map(one).collect(),
        Value::Object(_) => one(result).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// Waits for the initialize response, sends `initialized`, then waits for a
/// quiescent server status. Returns when the workspace is loaded.
fn wait_until_loaded(
    client: &mut Client<ChildStdin>,
    root_uri: &str,
    timeout: Duration,
    stderr: &dyn Fn() -> String,
) -> Result<(), ImplError> {
    let deadline = Instant::now() + timeout;
    let lsp = |stage: &'static str| {
        move |source: LspError| match source {
            LspError::Timeout => ImplError::NotReady {
                timeout,
                stderr: stderr(),
            },
            source => ImplError::Lsp {
                stage,
                source,
                stderr: stderr(),
            },
        }
    };
    let initialize = json!({
        "processId": std::process::id(),
        "rootUri": root_uri,
        "workspaceFolders": [{"uri": root_uri, "name": "root"}],
        "capabilities": {
            "general": {"positionEncodings": ["utf-8"]},
            "experimental": {"serverStatusNotification": true},
            "textDocument": {"implementation": {"linkSupport": false}},
        },
        "initializationOptions": {
            "cachePriming": {"enable": false},
            "checkOnSave": false,
        },
    });
    let result = client
        .request("initialize", initialize, deadline)
        .map_err(lsp("initialize"))?;
    let encoding = result["capabilities"]["positionEncoding"]
        .as_str()
        .unwrap_or("utf-16");
    if encoding != "utf-8" {
        return Err(ImplError::Encoding {
            found: encoding.to_string(),
            stderr: stderr(),
        });
    }
    client
        .notify("initialized", json!({}))
        .map_err(lsp("initialized"))?;
    loop {
        let message = client.next(deadline).map_err(lsp("loading"))?;
        if message["method"] != "experimental/serverStatus" {
            continue;
        }
        let status = &message["params"];
        if status["quiescent"] != true {
            continue;
        }
        if status["health"] == "error" {
            return Err(ImplError::Unhealthy {
                message: status["message"]
                    .as_str()
                    .unwrap_or("(no message)")
                    .to_string(),
                stderr: stderr(),
            });
        }
        return Ok(());
    }
}

/// Asks `textDocument/implementation` at `position` of `root`, again while
/// the server answers `ContentModified`.
fn implementation(
    client: &mut Client<ChildStdin>,
    root: &Path,
    position: &Position,
) -> Result<Value, LspError> {
    let params = json!({
        "textDocument": {"uri": file_uri(&root.join(&position.path))},
        "position": {"line": position.line, "character": position.character},
    });
    let mut attempt = 0;
    loop {
        let deadline = Instant::now() + ANSWER_TIMEOUT;
        match client.request("textDocument/implementation", params.clone(), deadline) {
            Err(LspError::Response { error, .. })
                if attempt < CONTENT_MODIFIED_RETRIES
                    && serde_json::from_str::<Value>(&error)
                        .is_ok_and(|e| e["code"] == CONTENT_MODIFIED) =>
            {
                attempt += 1;
            }
            other => return other,
        }
    }
}

impl RustAnalyzer {
    /// Starts rust-analyzer as a language server on the project at `root`
    /// (absolute), waits up to `load_timeout` for it to load the workspace,
    /// and asks `textDocument/implementation` at each of `questions`. The
    /// server's stderr is written to `stderr_path`.
    pub fn implementations(
        &self,
        root: &Path,
        questions: &[Position],
        load_timeout: Duration,
        stderr_path: &Path,
    ) -> Result<Answers, ImplError> {
        let spawn_error = |source| ImplError::Spawn {
            program: self.program(),
            source,
        };
        if let Some(parent) = stderr_path.parent() {
            std::fs::create_dir_all(parent).map_err(spawn_error)?;
        }
        let log = File::create(stderr_path).map_err(spawn_error)?;
        let started = Instant::now();
        let mut child = self
            .command()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log)
            .spawn()
            .map_err(spawn_error)?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(spawn_error(std::io::Error::other("no stdio pipes")));
        };
        let server = Server { child };
        let stderr_path: PathBuf = stderr_path.to_path_buf();
        let stderr = move || tail(&stderr_path);
        let mut client = Client::new(stdin, stdout);

        wait_until_loaded(&mut client, &file_uri(root), load_timeout, &stderr)?;
        let load_time = started.elapsed();

        let mut answers = Answers {
            load_time,
            ..Answers::default()
        };
        for question in questions {
            let result =
                implementation(&mut client, root, question).map_err(|source| ImplError::Lsp {
                    stage: "textDocument/implementation",
                    source,
                    stderr: stderr(),
                })?;
            let mut found = Vec::new();
            for (uri, line, character) in response_locations(&result) {
                match relative_path(&uri, root) {
                    Some(path) => found.push(Position {
                        path,
                        line,
                        character,
                    }),
                    None => answers.outside_root += 1,
                }
            }
            answers.locations.push(found);
        }
        answers.answer_time = started.elapsed().saturating_sub(load_time);

        // A clean shutdown; the guard kills a server that does not exit.
        let soon = Instant::now() + Duration::from_secs(10);
        if client.request("shutdown", Value::Null, soon).is_ok() {
            let _ = client.notify("exit", Value::Null);
        }
        drop(client);
        let mut server = server;
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until && matches!(server.child.try_wait(), Ok(None)) {
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(answers)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn every_response_shape_gives_its_locations() {
        assert!(response_locations(&Value::Null).is_empty());
        let location = json!({"uri": "file:///a.rs",
                              "range": {"start": {"line": 3, "character": 7},
                                        "end": {"line": 3, "character": 12}}});
        assert_eq!(
            response_locations(&location),
            [("file:///a.rs".to_string(), 3, 7)]
        );
        let link = json!({"targetUri": "file:///b.rs",
                          "targetRange": {"start": {"line": 1, "character": 0},
                                          "end": {"line": 9, "character": 1}},
                          "targetSelectionRange": {"start": {"line": 2, "character": 4},
                                                   "end": {"line": 2, "character": 9}}});
        assert_eq!(
            response_locations(&json!([location, link])),
            [
                ("file:///a.rs".to_string(), 3, 7),
                ("file:///b.rs".to_string(), 2, 4)
            ]
        );
    }

    #[test]
    fn a_missing_program_fails_to_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let provider = RustAnalyzer::with_program(dir.path().join("no-such-rust-analyzer"));
        let error = provider
            .implementations(
                dir.path(),
                &[],
                Duration::from_secs(1),
                &dir.path().join("log").join("stderr.log"),
            )
            .unwrap_err();
        assert!(matches!(error, ImplError::Spawn { .. }), "{error}");
    }

    #[test]
    fn not_ready_names_the_timeout_and_carries_stderr() {
        let error = ImplError::NotReady {
            timeout: Duration::from_secs(7),
            stderr: "loading\n".to_string(),
        };
        let text = error.to_string();
        assert!(
            text.starts_with("rust-analyzer did not finish loading the workspace within 7 s"),
            "{text}"
        );
        assert!(
            text.contains("--- rust-analyzer stderr ---\nloading"),
            "{text}"
        );
    }
}
