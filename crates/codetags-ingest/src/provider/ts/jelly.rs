//! Jelly (github.com/cs-au-dk/jelly): call edges for TypeScript and
//! JavaScript (brief §4.4 TypeScript row, V83).
//!
//! The runner writes Jelly's JSON call graph (`-j`) and parses it into
//! [`CallEdge`]s: caller, callee and call site. A silent Jelly failure once
//! cut a call graph by about 81% (brief §4.4), and Jelly exits 0 even when
//! it aborts, so every one of these is an error carrying Jelly's log: a
//! non-zero exit, an error in the log, a missing, empty or malformed graph, a
//! graph with no files, and a graph with call sites and functions but no call
//! edges.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use serde::Deserialize;

use super::{finish, npm_program};
use crate::provider::scip::Range;

/// The file name the runner asks Jelly to write in the output directory.
pub const CALL_GRAPH_FILE: &str = "callgraph.json";

/// A span in a source file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Location {
    /// Path relative to the analyzed root, with `/` separators.
    pub file: String,
    /// The span: 0-based lines and columns, end exclusive, like SCIP's
    /// ranges. Jelly's own are 1-based (V83); columns count UTF-16 code
    /// units, as Babel's do.
    pub range: Range,
}

/// What a call edge reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CalleeKind {
    /// A function, method, class (its constructor) or arrow function.
    Function,
    /// A whole module, loaded by an `import` or `require` at the site.
    Module,
}

/// One call edge.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CallEdge {
    /// The call expression (or import).
    pub site: Location,
    /// The innermost function or module containing the site.
    pub caller: Location,
    /// The function or module called.
    pub callee: Location,
    /// Whether the callee is a function or a module.
    pub kind: CalleeKind,
}

/// A parsed Jelly call graph.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallGraph {
    /// Every analyzed file, relative to the root, with `/` separators.
    pub files: Vec<String>,
    /// Every function Jelly found, modules excluded.
    pub functions: Vec<Location>,
    /// Every call site, resolved or not (calls to natives and externals have
    /// no edge).
    pub sites: Vec<Location>,
    /// Every call edge.
    pub edges: Vec<CallEdge>,
}

impl CallGraph {
    /// The edges that call functions (not modules).
    pub fn function_edges(&self) -> impl Iterator<Item = &CallEdge> {
        self.edges
            .iter()
            .filter(|edge| edge.kind == CalleeKind::Function)
    }
}

/// The JSON Jelly writes with `-j` (`lib/typings/callgraph.d.ts`, V83).
#[derive(Deserialize)]
struct RawGraph {
    files: Vec<String>,
    functions: BTreeMap<String, String>,
    calls: BTreeMap<String, String>,
    call2fun: Vec<[u64; 2]>,
}

/// Parses a Jelly JSON call graph.
///
/// Jelly numbers functions first, then calls. The last `files.len()`
/// functions are the modules, in file order (its `saveCallGraph` lists
/// `functionInfos` then `moduleInfos`, V83); a graph that breaks this is
/// rejected rather than misread.
pub fn parse_call_graph(json: &str) -> Result<CallGraph, String> {
    let raw: RawGraph = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let files: Vec<String> = raw.files.iter().map(|f| f.replace('\\', "/")).collect();
    let ids = |map: &BTreeMap<String, String>| -> Result<BTreeMap<u64, Location>, String> {
        map.iter()
            .map(|(id, loc)| {
                let id = id.parse::<u64>().map_err(|_| format!("bad id {id:?}"))?;
                Ok((id, parse_location(&files, loc)?))
            })
            .collect()
    };
    let functions = ids(&raw.functions)?;
    let calls = ids(&raw.calls)?;
    let first_module = functions
        .len()
        .checked_sub(files.len())
        .ok_or_else(|| format!("{} files but {} functions", files.len(), functions.len()))?;
    let mut kinds = BTreeMap::new();
    for (position, (id, location)) in functions.iter().enumerate() {
        let kind = if position < first_module {
            CalleeKind::Function
        } else {
            let file = &files[position - first_module];
            if &location.file != file {
                return Err(format!(
                    "function {id} should be the module {file}, but is in {}",
                    location.file
                ));
            }
            CalleeKind::Module
        };
        kinds.insert(*id, kind);
    }
    let mut edges = Vec::with_capacity(raw.call2fun.len());
    for [call, callee] in raw.call2fun {
        let site = calls
            .get(&call)
            .ok_or_else(|| format!("call2fun names unknown call {call}"))?;
        let (callee_location, kind) = functions
            .get(&callee)
            .zip(kinds.get(&callee))
            .ok_or_else(|| format!("call2fun names unknown function {callee}"))?;
        let caller = innermost(functions.values(), site)
            .ok_or_else(|| format!("no function or module contains the call {call}"))?;
        edges.push(CallEdge {
            site: site.clone(),
            caller: caller.clone(),
            callee: callee_location.clone(),
            kind: *kind,
        });
    }
    let functions = functions
        .iter()
        .filter(|(id, _)| kinds.get(id) == Some(&CalleeKind::Function))
        .map(|(_, location)| location.clone())
        .collect();
    Ok(CallGraph {
        files,
        functions,
        sites: calls.into_values().collect(),
        edges,
    })
}

/// Parses Jelly's `<file index>:<line>:<column>:<end line>:<end column>`
/// (1-based, end exclusive) into a 0-based [`Location`].
fn parse_location(files: &[String], text: &str) -> Result<Location, String> {
    let bad = || format!("bad location {text:?}");
    let fields: Vec<u32> = text
        .split(':')
        .map(|field| field.parse::<u32>().ok())
        .collect::<Option<Vec<u32>>>()
        .ok_or_else(bad)?;
    let [file, position @ ..] = &fields[..] else {
        return Err(bad());
    };
    let [start_line, start_character, end_line, end_character] = position
        .iter()
        .map(|n| n.checked_sub(1))
        .collect::<Option<Vec<u32>>>()
        .and_then(|v| <[u32; 4]>::try_from(v).ok())
        .ok_or_else(bad)?;
    let file = usize::try_from(*file)
        .ok()
        .and_then(|file| files.get(file))
        .ok_or_else(|| format!("location {text:?} names an unknown file"))?;
    Ok(Location {
        file: file.clone(),
        range: Range {
            start_line,
            start_character,
            end_line,
            end_character,
        },
    })
}

/// The innermost of `candidates` that contains `site`.
fn innermost<'a>(
    candidates: impl Iterator<Item = &'a Location>,
    site: &Location,
) -> Option<&'a Location> {
    candidates
        .filter(|c| c.file == site.file && c.range.contains(&site.range))
        .max_by_key(|c| {
            let r = c.range;
            (
                r.start_line,
                r.start_character,
                std::cmp::Reverse((r.end_line, r.end_character)),
            )
        })
}

/// The lines of Jelly's log that report a failure: `Error: ...`, an
/// `aborting` notice, and `Analysis errors: N` with N > 0 (V83).
pub fn reported_errors(log: &str) -> Vec<String> {
    log.lines()
        .map(str::trim)
        .filter(|line| {
            line.starts_with("Error:")
                || line.ends_with("aborting")
                || line
                    .strip_prefix("Analysis errors: ")
                    .and_then(|rest| rest.split(',').next())
                    .is_some_and(|count| count.trim() != "0")
        })
        .map(str::to_string)
        .collect()
}

/// A Jelly run that passed every check.
#[derive(Debug)]
pub struct JellyRun {
    /// The call graph file Jelly wrote, inside the output directory.
    pub call_graph_path: PathBuf,
    /// The parsed call graph.
    pub graph: CallGraph,
    /// Jelly's log (stdout, then stderr).
    pub log: String,
}

/// Why a Jelly run failed. Every variant that ran Jelly carries its log.
#[derive(Debug)]
pub enum JellyError {
    /// Jelly could not be started (not installed, not on PATH).
    Spawn {
        /// The program that was run.
        program: String,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The output directory could not be prepared.
    OutputDir {
        /// The output directory.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// Jelly exited non-zero or was killed.
    Exit {
        /// Its exit status.
        status: ExitStatus,
        /// Its log.
        log: String,
    },
    /// Jelly's log reports errors (it exits 0 regardless).
    Reported {
        /// The lines reporting them.
        errors: Vec<String>,
        /// Its log.
        log: String,
    },
    /// Jelly wrote no call graph.
    MissingOutput {
        /// Where the call graph should be.
        path: PathBuf,
        /// Its log.
        log: String,
    },
    /// Jelly wrote an empty call graph file.
    EmptyOutput {
        /// The call graph file.
        path: PathBuf,
        /// Its log.
        log: String,
    },
    /// The call graph could not be read or parsed.
    BadOutput {
        /// Why.
        message: String,
        /// Its log.
        log: String,
    },
    /// The call graph lists no files.
    NoFiles {
        /// Its log.
        log: String,
    },
    /// The graph has call sites and functions, but no call reaches a
    /// function: what a silent failure looks like.
    NoEdges {
        /// How many call sites it has.
        sites: usize,
        /// Its log.
        log: String,
    },
}

impl JellyError {
    /// Jelly's log, when it ran.
    pub fn log(&self) -> Option<&str> {
        match self {
            Self::Spawn { .. } | Self::OutputDir { .. } => None,
            Self::Exit { log, .. }
            | Self::Reported { log, .. }
            | Self::MissingOutput { log, .. }
            | Self::EmptyOutput { log, .. }
            | Self::BadOutput { log, .. }
            | Self::NoFiles { log }
            | Self::NoEdges { log, .. } => Some(log),
        }
    }
}

impl fmt::Display for JellyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { program, source } => {
                return write!(f, "jelly ({program}) could not start: {source}");
            }
            Self::OutputDir { path, source } => {
                return write!(f, "output directory {}: {source}", path.display());
            }
            Self::Exit { status, .. } => write!(f, "jelly failed ({status})")?,
            Self::Reported { errors, .. } => {
                write!(f, "jelly reported errors: {}", errors.join("; "))?
            }
            Self::MissingOutput { path, .. } => {
                write!(f, "jelly wrote no call graph {}", path.display())?
            }
            Self::EmptyOutput { path, .. } => {
                write!(f, "jelly wrote an empty call graph {}", path.display())?
            }
            Self::BadOutput { message, .. } => write!(f, "jelly call graph unreadable: {message}")?,
            Self::NoFiles { .. } => write!(f, "jelly analyzed no files")?,
            Self::NoEdges { sites, .. } => write!(
                f,
                "jelly found {sites} call sites and some functions but no call edges"
            )?,
        }
        if let Some(log) = self.log() {
            write!(f, "\n--- provider stderr ---\n{}", log.trim_end())?;
        }
        Ok(())
    }
}

impl std::error::Error for JellyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } | Self::OutputDir { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Runs `jelly`.
#[derive(Debug, Clone)]
pub struct Jelly {
    program: OsString,
    envs: Vec<(OsString, OsString)>,
}

impl Default for Jelly {
    fn default() -> Self {
        Self::new()
    }
}

impl Jelly {
    /// The `jelly` found on `PATH`.
    pub fn new() -> Self {
        Self::with_program(npm_program("jelly"))
    }

    /// A specific Jelly executable.
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            envs: Vec::new(),
        }
    }

    /// Sets an environment variable for Jelly.
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.envs
            .push((key.as_ref().to_os_string(), value.as_ref().to_os_string()));
        self
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.envs(self.envs.iter().map(|(k, v)| (k, v)));
        command
    }

    fn spawn_error(&self, source: std::io::Error) -> JellyError {
        JellyError::Spawn {
            program: self.program.to_string_lossy().into_owned(),
            source,
        }
    }

    /// `jelly --version`, e.g. `0.13.0`.
    pub fn version(&self) -> Result<String, JellyError> {
        let run = finish(self.command().arg("--version")).map_err(|e| self.spawn_error(e))?;
        if !run.status.success() {
            return Err(JellyError::Exit {
                status: run.status,
                log: run.log,
            });
        }
        Ok(run.log.trim().to_string())
    }

    /// Analyzes every source file under `root` (the base directory, `-b`)
    /// into `out_dir/callgraph.json`, creating `out_dir` if needed and
    /// replacing a previous graph there, and checks the result (see
    /// [`self`](super::jelly) for what fails).
    pub fn analyze(&self, root: &Path, out_dir: &Path) -> Result<JellyRun, JellyError> {
        let path = out_dir.join(CALL_GRAPH_FILE);
        let out_error = |source| JellyError::OutputDir {
            path: out_dir.to_path_buf(),
            source,
        };
        std::fs::create_dir_all(out_dir).map_err(out_error)?;
        // A stale graph must never pass for this run's output.
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(out_error(error)),
        }
        let run = finish(
            self.command()
                // Without -b Jelly looks for a package.json and, finding
                // none, aborts with exit status 0 (V83).
                .arg("-b")
                .arg(root)
                .arg("-j")
                .arg(&path)
                .arg("--no-print-progress")
                .arg("--no-tty")
                .arg("--")
                .arg(root),
        )
        .map_err(|e| self.spawn_error(e))?;
        check_run(run.status, run.log, &path)
    }
}

/// Checks a finished Jelly run whose graph should be at `path`.
fn check_run(status: ExitStatus, log: String, path: &Path) -> Result<JellyRun, JellyError> {
    if !status.success() {
        return Err(JellyError::Exit { status, log });
    }
    let errors = reported_errors(&log);
    if !errors.is_empty() {
        return Err(JellyError::Reported { errors, log });
    }
    let json = match std::fs::read_to_string(path) {
        Ok(json) => json,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(JellyError::MissingOutput {
                path: path.to_path_buf(),
                log,
            });
        }
        Err(error) => {
            return Err(JellyError::BadOutput {
                message: format!("reading {}: {error}", path.display()),
                log,
            });
        }
    };
    if json.trim().is_empty() {
        return Err(JellyError::EmptyOutput {
            path: path.to_path_buf(),
            log,
        });
    }
    let graph = match parse_call_graph(&json) {
        Ok(graph) => graph,
        Err(message) => return Err(JellyError::BadOutput { message, log }),
    };
    if graph.files.is_empty() {
        return Err(JellyError::NoFiles { log });
    }
    if !graph.sites.is_empty()
        && !graph.functions.is_empty()
        && graph.function_edges().next().is_none()
    {
        return Err(JellyError::NoEdges {
            sites: graph.sites.len(),
            log,
        });
    }
    Ok(JellyRun {
        call_graph_path: path.to_path_buf(),
        graph,
        log,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    /// `a.ts`: `function f() { g(); }` and `function g() {}` (lines 1-2),
    /// `import "./b"` on line 3; `b.ts`: one native call on line 1.
    const GRAPH: &str = r#"{
     "files": ["src\\a.ts", "src/b.ts"],
     "functions": {"0": "0:1:1:1:23", "1": "0:2:1:2:16", "2": "0:1:1:4:1", "3": "1:1:1:2:1"},
     "calls": {"4": "0:1:16:1:19", "5": "0:3:1:3:13", "6": "1:1:1:1:9"},
     "fun2fun": [[0, 1], [2, 3]],
     "call2fun": [[4, 1], [5, 3]],
     "ignore": []
    }"#;

    fn loc(file: &str, a: u32, b: u32, c: u32, d: u32) -> Location {
        Location {
            file: file.into(),
            range: Range {
                start_line: a,
                start_character: b,
                end_line: c,
                end_character: d,
            },
        }
    }

    #[test]
    fn a_graph_parses_into_edges() {
        let graph = parse_call_graph(GRAPH).unwrap();
        assert_eq!(graph.files, ["src/a.ts", "src/b.ts"]);
        assert_eq!(
            graph.functions,
            [loc("src/a.ts", 0, 0, 0, 22), loc("src/a.ts", 1, 0, 1, 15)]
        );
        assert_eq!(graph.sites.len(), 3);
        assert_eq!(
            graph.edges,
            [
                CallEdge {
                    site: loc("src/a.ts", 0, 15, 0, 18),
                    caller: loc("src/a.ts", 0, 0, 0, 22),
                    callee: loc("src/a.ts", 1, 0, 1, 15),
                    kind: CalleeKind::Function,
                },
                CallEdge {
                    site: loc("src/a.ts", 2, 0, 2, 12),
                    caller: loc("src/a.ts", 0, 0, 3, 0),
                    callee: loc("src/b.ts", 0, 0, 1, 0),
                    kind: CalleeKind::Module,
                },
            ]
        );
        assert_eq!(graph.function_edges().count(), 1);
    }

    #[test]
    fn unknown_locations_are_rejected() {
        let json = GRAPH.replace("0:2:1:2:16", "0:?:?:?:?");
        assert!(
            parse_call_graph(&json)
                .unwrap_err()
                .contains("bad location")
        );
        let json = GRAPH.replace("0:2:1:2:16", "7:2:1:2:16");
        assert!(
            parse_call_graph(&json)
                .unwrap_err()
                .contains("unknown file")
        );
    }

    #[test]
    fn modules_out_of_order_are_rejected() {
        let json = GRAPH.replace("\"3\": \"1:1:1:2:1\"", "\"3\": \"0:1:1:2:1\"");
        assert!(
            parse_call_graph(&json)
                .unwrap_err()
                .contains("should be the module")
        );
    }

    #[test]
    fn edges_to_unknown_ids_are_rejected() {
        let json = GRAPH.replace("[4, 1]", "[4, 9]");
        assert!(
            parse_call_graph(&json)
                .unwrap_err()
                .contains("unknown function")
        );
    }

    #[test]
    fn errors_in_the_log_are_found() {
        let log = "Error: Unrecoverable parse error for /p/bad.ts: Unexpected token\nAnalyzed packages: 1\nAnalysis errors: 2, warnings: 0";
        assert_eq!(reported_errors(log).len(), 2);
        assert_eq!(
            reported_errors(
                "Can't auto-detect basedir, package.json not found (use option -b), aborting"
            )
            .len(),
            1
        );
        assert!(reported_errors("Analysis errors: 0, warnings: 3\nCall graph written").is_empty());
    }

    #[cfg(unix)]
    fn exit(code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }

    #[cfg(windows)]
    fn exit(code: i32) -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(code as u32)
    }

    fn check(json: Option<&str>, log: &str, code: i32) -> Result<JellyRun, JellyError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CALL_GRAPH_FILE);
        if let Some(json) = json {
            std::fs::write(&path, json).unwrap();
        }
        check_run(exit(code), log.to_string(), &path)
    }

    #[test]
    fn a_good_run_passes() {
        let run = check(Some(GRAPH), "Analysis errors: 0, warnings: 0", 0).unwrap();
        assert_eq!(run.graph.edges.len(), 2);
    }

    #[test]
    fn every_failure_is_loud_and_carries_the_log() {
        let no_edges = GRAPH.replace("[[4, 1], [5, 3]]", "[[5, 3]]");
        let no_files =
            r#"{"files": [], "functions": {}, "calls": {}, "fun2fun": [], "call2fun": []}"#;
        let cases: [(Option<&str>, &str, i32, &str); 7] = [
            (Some(GRAPH), "the log", 1, "jelly failed"),
            (
                Some(GRAPH),
                "Error: No files to analyze",
                0,
                "reported errors",
            ),
            (None, "the log", 0, "wrote no call graph"),
            (Some("  \n"), "the log", 0, "empty call graph"),
            (Some("{"), "the log", 0, "unreadable"),
            (Some(no_files), "the log", 0, "analyzed no files"),
            (Some(&no_edges), "the log", 0, "no call edges"),
        ];
        for (json, log, code, expected) in cases {
            let error = check(json, log, code).unwrap_err();
            let text = error.to_string();
            assert!(text.contains(expected), "{text}");
            assert!(
                text.contains(&format!("--- provider stderr ---\n{log}")),
                "{text}"
            );
        }
    }

    #[test]
    fn a_missing_program_fails_to_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let jelly = Jelly::with_program(dir.path().join("no-such-jelly"));
        let error = jelly
            .analyze(dir.path(), &dir.path().join("out"))
            .unwrap_err();
        assert!(matches!(error, JellyError::Spawn { .. }), "{error}");
        assert!(matches!(
            jelly.version().unwrap_err(),
            JellyError::Spawn { .. }
        ));
    }
}
