//! The Python provider (brief §4.4 Python row): `scip-python` for symbols
//! and references, plus the call and attribute references Pyright could not
//! resolve, by name, so ingest can count name-match candidates.
//!
//! scip-python writes nothing at all for a reference it cannot resolve
//! (V94): a duck-typed `sink.send()` has no occurrence at `send`. So the
//! runner lists every *name site* in the indexed files with Python's own
//! `ast` module ([`SITES_SCRIPT`], run by the interpreter scip-python itself
//! needs), and a site is unresolved when no occurrence starts at its name, or
//! only a document-local symbol the document never defines does: that is how
//! scip-python writes a stdlib class or method it cannot name.
//!
//! Failures are loud and carry the failing program's stderr: either program
//! failing to start or exiting non-zero, a missing or unreadable index or
//! site file, and an index that resolves none of its call sites. scip-python
//! exits 0 on a file that does not parse (V95); the site extraction does not.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

use super::scip::{Document, Range, ScipIndex};
use super::{ProviderError, ScipRun, check_output};

/// The file name the runner asks scip-python to write in the output
/// directory.
pub const INDEX_FILE: &str = "index.scip";

/// The file name of the name sites the runner writes in the output
/// directory.
pub const SITES_FILE: &str = "sites.jsonl";

/// The file name the site script is written to in the output directory.
pub const SITES_SCRIPT_FILE: &str = "python_sites.py";

/// The script that lists name sites (see the module docs).
pub const SITES_SCRIPT: &str = include_str!("python_sites.py");

/// A call or attribute reference in the source, by name: every attribute
/// read (`x.name`), and every bare name that is called or used as a
/// decorator.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NameSite {
    /// The file, relative to the project root, with `/` separators.
    pub file: String,
    /// Where the name is: 0-based lines, characters in UTF-16 code units
    /// (as scip-python counts them).
    pub range: Range,
    /// The name as written.
    pub name: String,
    /// Whether the name is called (or is a decorator); otherwise it is an
    /// attribute read, e.g. a method passed as a callback.
    pub called: bool,
}

/// Why a name site is unresolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Unresolved {
    /// No occurrence starts at the name (a duck-typed receiver).
    NoOccurrence,
    /// Only a document-local symbol that the document never defines
    /// (scip-python's form for a stdlib symbol it cannot name, V94).
    UnnamedLocal,
}

impl Unresolved {
    /// The name the features use.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoOccurrence => "no-occurrence",
            Self::UnnamedLocal => "unnamed-local",
        }
    }
}

/// A name site scip-python did not resolve to a symbol.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UnresolvedReference {
    /// The site.
    pub site: NameSite,
    /// Why it is unresolved.
    pub reason: Unresolved,
}

/// One JSON line as the site script writes it.
#[derive(Deserialize)]
struct RawSite {
    file: String,
    line: u32,
    start: u32,
    end: u32,
    name: String,
    called: bool,
}

impl From<RawSite> for NameSite {
    fn from(raw: RawSite) -> Self {
        Self {
            file: raw.file,
            range: Range {
                start_line: raw.line,
                start_character: raw.start,
                end_line: raw.line,
                end_character: raw.end,
            },
            name: raw.name,
            called: raw.called,
        }
    }
}

/// A Python provider run that passed every check.
#[derive(Debug)]
pub struct PythonRun {
    /// scip-python's index.
    pub scip: ScipRun,
    /// The site file, inside the output directory.
    pub sites_path: PathBuf,
    /// Every name site in the indexed documents, sorted by file and
    /// position.
    pub sites: Vec<NameSite>,
    /// The sites scip-python did not resolve, in the same order.
    pub unresolved: Vec<UnresolvedReference>,
    /// The site script's stderr.
    pub sites_stderr: String,
}

/// Why a Python provider run failed. Every variant from a program that ran
/// carries its stderr.
#[derive(Debug)]
pub enum PythonError {
    /// scip-python failed, or its index failed the checks.
    Scip(ProviderError),
    /// The site script failed to start, exited non-zero (a file that does
    /// not parse), or wrote no site file.
    Sites(ProviderError),
    /// A line of the site file is not a name site.
    BadSite {
        /// The 1-based line number.
        line: usize,
        /// Why it did not parse.
        message: String,
        /// The script's stderr.
        stderr: String,
    },
    /// The sources call things, but the index resolves none of the calls.
    NothingResolved {
        /// How many call sites there are.
        calls: usize,
        /// A document holding one.
        example: String,
        /// scip-python's stderr.
        stderr: String,
    },
}

impl PythonError {
    /// The failing program's stderr, when it ran.
    pub fn stderr(&self) -> Option<&str> {
        match self {
            Self::Scip(error) | Self::Sites(error) => error.stderr(),
            Self::BadSite { stderr, .. } | Self::NothingResolved { stderr, .. } => Some(stderr),
        }
    }
}

impl fmt::Display for PythonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stderr = match self {
            Self::Scip(error) => return write!(f, "scip-python: {error}"),
            Self::Sites(error) => return write!(f, "python name sites: {error}"),
            Self::BadSite {
                line,
                message,
                stderr,
            } => {
                write!(f, "python name sites: line {line} is not a site: {message}")?;
                stderr
            }
            Self::NothingResolved {
                calls,
                example,
                stderr,
            } => {
                write!(
                    f,
                    "scip-python resolved none of {calls} call sites (e.g. in {example})"
                )?;
                stderr
            }
        };
        write!(f, "\n--- provider stderr ---\n{}", stderr.trim_end())
    }
}

impl std::error::Error for PythonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Scip(error) | Self::Sites(error) => Some(error),
            _ => None,
        }
    }
}

/// Runs `scip-python` and the site script.
#[derive(Debug, Clone)]
pub struct PythonProvider {
    scip_python: OsString,
    python: OsString,
    envs: Vec<(OsString, OsString)>,
}

impl Default for PythonProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl PythonProvider {
    /// The `scip-python` and Python interpreter found on `PATH`: npm's
    /// `scip-python.cmd` shim and `python` on Windows, `scip-python` and
    /// `python3` elsewhere (the names scip-python itself tries first).
    pub fn new() -> Self {
        if cfg!(windows) {
            Self::with_programs("scip-python.cmd", "python")
        } else {
            Self::with_programs("scip-python", "python3")
        }
    }

    /// Specific executables.
    pub fn with_programs(scip_python: impl Into<OsString>, python: impl Into<OsString>) -> Self {
        Self {
            scip_python: scip_python.into(),
            python: python.into(),
            envs: Vec::new(),
        }
    }

    /// Sets an environment variable for both programs.
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.envs
            .push((key.as_ref().to_os_string(), value.as_ref().to_os_string()));
        self
    }

    fn command(&self, program: &OsStr) -> Command {
        let mut command = Command::new(program);
        command.envs(self.envs.iter().map(|(k, v)| (k, v)));
        command
    }

    fn spawn_error(program: &OsStr) -> impl FnOnce(std::io::Error) -> ProviderError {
        let program = program.to_string_lossy().into_owned();
        move |source| ProviderError::Spawn { program, source }
    }

    /// `scip-python --version`, e.g. `0.6.6`.
    pub fn scip_python_version(&self) -> Result<String, ProviderError> {
        let output = self
            .command(&self.scip_python)
            .arg("--version")
            .stdin(Stdio::null())
            .output()
            .map_err(Self::spawn_error(&self.scip_python))?;
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() {
            return Err(ProviderError::Exit {
                status: output.status,
                stderr,
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Indexes the Python project at `root` into `out_dir`, creating it if
    /// needed and replacing earlier output there, and lists its unresolved
    /// call and attribute references (see the module docs for what fails).
    /// scip-python takes the project's name and version from its
    /// `pyproject.toml`, or the version from git.
    pub fn index(&self, root: &Path, out_dir: &Path) -> Result<PythonRun, PythonError> {
        let index_path = out_dir.join(INDEX_FILE);
        let sites_path = out_dir.join(SITES_FILE);
        let script_path = out_dir.join(SITES_SCRIPT_FILE);
        prepare(out_dir, &[&index_path, &sites_path]).map_err(PythonError::Scip)?;
        // scip-python drops every file when --cwd is relative (V92) or
        // reaches the project through a symbolic link (V104), and writes
        // --output relative to --cwd.
        let out_error = |source| ProviderError::OutputDir {
            path: out_dir.to_path_buf(),
            source,
        };
        let root = project_root(root)
            .map_err(out_error)
            .map_err(PythonError::Scip)?;
        let index_path = std::path::absolute(&index_path)
            .map_err(out_error)
            .map_err(PythonError::Scip)?;

        let output = self
            .command(&self.scip_python)
            .arg("index")
            .arg("--cwd")
            .arg(&root)
            .arg("--output")
            .arg(&index_path)
            .arg("--quiet")
            .stdin(Stdio::null())
            .output()
            .map_err(Self::spawn_error(&self.scip_python))
            .map_err(PythonError::Scip)?;
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() {
            return Err(PythonError::Scip(ProviderError::Exit {
                status: output.status,
                stderr,
            }));
        }
        let scip = check_output(&root, &index_path, stderr, declares_function)
            .map_err(PythonError::Scip)?;

        std::fs::write(&script_path, SITES_SCRIPT)
            .map_err(out_error)
            .map_err(PythonError::Sites)?;
        let paths: String = scip
            .index
            .documents
            .iter()
            .map(|document| format!("{}\n", document.relative_path))
            .collect();
        let (status, sites_stderr) = self
            .run_script(&script_path, &root, &sites_path, &paths)
            .map_err(PythonError::Sites)?;
        if !status.success() {
            return Err(PythonError::Sites(ProviderError::Exit {
                status,
                stderr: sites_stderr,
            }));
        }
        let Ok(text) = std::fs::read_to_string(&sites_path) else {
            return Err(PythonError::Sites(ProviderError::MissingOutput {
                path: sites_path,
                stderr: sites_stderr,
            }));
        };
        let sites = parse_sites(&text, &sites_stderr)?;
        let unresolved = unresolved(&scip.index, &sites);
        check_resolved(&sites, &unresolved, &scip.stderr)?;
        Ok(PythonRun {
            scip,
            sites_path,
            sites,
            unresolved,
            sites_stderr,
        })
    }

    /// Runs the site script with `paths` on stdin.
    fn run_script(
        &self,
        script: &Path,
        root: &Path,
        sites_path: &Path,
        paths: &str,
    ) -> Result<(std::process::ExitStatus, String), ProviderError> {
        let mut child = self
            .command(&self.python)
            .arg(script)
            .arg(root)
            .arg(sites_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(Self::spawn_error(&self.python))?;
        let written = child
            .stdin
            .take()
            .map(|mut stdin| stdin.write_all(paths.as_bytes()));
        let output = child
            .wait_with_output()
            .map_err(Self::spawn_error(&self.python))?;
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        // A script that stopped reading early reports why on stderr; its
        // exit status says more than the broken pipe.
        if output.status.success()
            && let Some(Err(source)) = written
        {
            return Err(ProviderError::Spawn {
                program: self.python.to_string_lossy().into_owned(),
                source,
            });
        }
        Ok((output.status, stderr))
    }
}

/// The root as scip-python must be given it: absolute, with symbolic links
/// resolved. scip-python keeps a file only if its real path starts with the
/// root as given (V92), so a root reached through a link (macOS's temporary
/// directories, under `/var` → `/private/var`) gives an index with no
/// documents (V104). A root that cannot be resolved, e.g. one that does not
/// exist, is only made absolute, and scip-python reports it.
fn project_root(root: &Path) -> std::io::Result<PathBuf> {
    match std::fs::canonicalize(root) {
        Ok(real) => Ok(without_verbatim_prefix(real)),
        Err(_) => std::path::absolute(root),
    }
}

/// `path` without the `\\?\` prefix Windows' `canonicalize` adds, for a
/// drive or UNC path: Node and Pyright do not expect verbatim paths. Any
/// other path is returned unchanged.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// Creates `out_dir` and removes earlier output files in it: a stale file
/// must never pass for this run's output.
fn prepare(out_dir: &Path, files: &[&Path]) -> Result<(), ProviderError> {
    let out_error = |source| ProviderError::OutputDir {
        path: out_dir.to_path_buf(),
        source,
    };
    std::fs::create_dir_all(out_dir).map_err(out_error)?;
    for file in files {
        match std::fs::remove_file(file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(out_error(error)),
        }
    }
    Ok(())
}

/// Parses the site script's JSON lines, sorted by file and position. Blank
/// lines are skipped; any other line that is not a site is an error.
pub fn parse_sites(text: &str, stderr: &str) -> Result<Vec<NameSite>, PythonError> {
    let mut sites = text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(at, line)| {
            serde_json::from_str::<RawSite>(line)
                .map(NameSite::from)
                .map_err(|error| PythonError::BadSite {
                    line: at + 1,
                    message: error.to_string(),
                    stderr: stderr.to_string(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    sites.sort();
    Ok(sites)
}

/// The sites `index` does not resolve (see the module docs), in the order
/// of `sites`. A site in a file the index has no document for has no
/// occurrence.
pub fn unresolved(index: &ScipIndex, sites: &[NameSite]) -> Vec<UnresolvedReference> {
    sites
        .iter()
        .filter_map(|site| {
            let document = index
                .documents
                .iter()
                .find(|document| document.relative_path == site.file);
            resolution(document, &site.range).map(|reason| UnresolvedReference {
                site: site.clone(),
                reason,
            })
        })
        .collect()
}

/// Why the name at `range` is unresolved in `document`, or `None` when an
/// occurrence there names a symbol.
fn resolution(document: Option<&Document>, range: &Range) -> Option<Unresolved> {
    let Some(document) = document else {
        return Some(Unresolved::NoOccurrence);
    };
    let at_name: Vec<_> = document
        .occurrences
        .iter()
        .filter(|occurrence| {
            (
                occurrence.range.start_line,
                occurrence.range.start_character,
            ) == (range.start_line, range.start_character)
        })
        .collect();
    if at_name.is_empty() {
        return Some(Unresolved::NoOccurrence);
    }
    let named = at_name.iter().any(|occurrence| {
        !occurrence.is_local()
            || document
                .occurrences
                .iter()
                .any(|other| other.is_definition() && other.symbol == occurrence.symbol)
    });
    if named {
        None
    } else {
        Some(Unresolved::UnnamedLocal)
    }
}

/// Fails when there are call sites but the index resolves none of them
/// (brief §4.4: a provider that silently loses its edges must fail).
pub fn check_resolved(
    sites: &[NameSite],
    unresolved: &[UnresolvedReference],
    stderr: &str,
) -> Result<(), PythonError> {
    let calls: Vec<&NameSite> = sites.iter().filter(|site| site.called).collect();
    let unresolved_calls = unresolved.iter().filter(|u| u.site.called).count();
    match calls.first() {
        Some(example) if unresolved_calls == calls.len() => Err(PythonError::NothingResolved {
            calls: calls.len(),
            example: example.file.clone(),
            stderr: stderr.to_string(),
        }),
        _ => Ok(()),
    }
}

/// Whether Python `source` declares a function or method: the keyword `def`
/// followed by whitespace and a name. Lexical, so a commented-out
/// declaration counts too; it only decides whether an index with no
/// occurrences at all is suspicious.
pub fn declares_function(source: &str) -> bool {
    let is_ident = |c: char| c == '_' || c.is_alphanumeric();
    source.match_indices("def").any(|(at, _)| {
        let before = source[..at].chars().next_back();
        let after = &source[at + 3..];
        let name = after.trim_start();
        before.is_none_or(|c| !is_ident(c))
            && after.len() > name.len()
            && name
                .chars()
                .next()
                .is_some_and(|c| c == '_' || c.is_alphabetic())
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::super::scip::Occurrence;
    use super::*;

    fn range(line: u32, start: u32, end: u32) -> Range {
        Range {
            start_line: line,
            start_character: start,
            end_line: line,
            end_character: end,
        }
    }

    fn occurrence(symbol: &str, at: Range, roles: i32) -> Occurrence {
        Occurrence {
            symbol: symbol.into(),
            range: at,
            enclosing_range: None,
            roles,
        }
    }

    fn site(line: u32, start: u32, name: &str, called: bool) -> NameSite {
        NameSite {
            file: "a.py".into(),
            range: range(line, start, start + name.len() as u32),
            name: name.into(),
            called,
        }
    }

    fn index_with(occurrences: Vec<Occurrence>) -> ScipIndex {
        ScipIndex {
            documents: vec![Document {
                relative_path: "a.py".into(),
                occurrences,
                symbols: vec![],
            }],
            ..ScipIndex::default()
        }
    }

    const SETTLE: &str = "scip-python python shop 0.1.0 `shop.pay.methods`/settle().";

    #[test]
    fn sites_parse_and_sort() {
        let text = concat!(
            r#"{"file": "b.py", "line": 1, "start": 4, "end": 8, "name": "send", "called": true}"#,
            "\n\n",
            r#"{"file": "a.py", "line": 3, "start": 0, "end": 2, "name": "fn", "called": false}"#,
            "\n"
        );
        let sites = parse_sites(text, "").unwrap();
        assert_eq!(
            sites,
            [
                NameSite {
                    file: "a.py".into(),
                    range: range(3, 0, 2),
                    name: "fn".into(),
                    called: false,
                },
                NameSite {
                    file: "b.py".into(),
                    range: range(1, 4, 8),
                    name: "send".into(),
                    called: true,
                },
            ]
        );
    }

    #[test]
    fn a_line_that_is_not_a_site_is_an_error_with_its_number() {
        let text = "{\"file\": \"a.py\", \"line\": 0, \"start\": 0, \"end\": 1, \"name\": \"f\", \"called\": true}\n{\"file\": \"a.py\"}\n";
        let error = parse_sites(text, "the stderr").unwrap_err();
        assert!(
            matches!(error, PythonError::BadSite { line: 2, .. }),
            "{error}"
        );
        assert!(error.to_string().contains("the stderr"), "{error}");
    }

    #[test]
    fn a_site_without_an_occurrence_is_unresolved() {
        let index = index_with(vec![occurrence(SETTLE, range(0, 0, 6), 8)]);
        let sites = [site(0, 0, "settle", true), site(2, 9, "send", true)];
        let found = unresolved(&index, &sites);
        assert_eq!(
            found,
            [UnresolvedReference {
                site: sites[1].clone(),
                reason: Unresolved::NoOccurrence,
            }]
        );
    }

    #[test]
    fn an_undefined_local_is_unnamed_and_a_defined_one_resolves() {
        let index = index_with(vec![
            // `local 0` is referenced and never defined: a stdlib symbol
            // scip-python could not name.
            occurrence("local 0", range(1, 4, 10), 8),
            // `local 1` is defined in the document: a real local.
            occurrence("local 1", range(2, 4, 9), 1),
            occurrence("local 1", range(3, 4, 9), 8),
        ]);
        let sites = [site(1, 4, "submit", true), site(3, 4, "inner", true)];
        let found = unresolved(&index, &sites);
        assert_eq!(
            found,
            [UnresolvedReference {
                site: sites[0].clone(),
                reason: Unresolved::UnnamedLocal,
            }]
        );
    }

    #[test]
    fn a_site_in_a_file_without_a_document_has_no_occurrence() {
        let mut other = site(0, 0, "f", true);
        other.file = "b.py".into();
        let found = unresolved(&index_with(vec![]), std::slice::from_ref(&other));
        assert_eq!(found[0].reason, Unresolved::NoOccurrence);
    }

    #[test]
    fn no_resolved_call_is_an_error() {
        let sites = [site(0, 0, "send", true), site(1, 0, "fee", false)];
        let found = unresolved(&index_with(vec![]), &sites);
        let error = check_resolved(&sites, &found, "the stderr").unwrap_err();
        assert!(
            matches!(error, PythonError::NothingResolved { calls: 1, .. }),
            "{error}"
        );
        assert_eq!(error.stderr(), Some("the stderr"));
    }

    #[test]
    fn one_resolved_call_or_no_calls_passes() {
        let sites = [site(0, 0, "settle", true), site(1, 0, "send", true)];
        let index = index_with(vec![occurrence(SETTLE, range(0, 0, 6), 8)]);
        assert!(check_resolved(&sites, &unresolved(&index, &sites), "").is_ok());
        let attributes = [site(1, 0, "fee", false)];
        let found = unresolved(&index_with(vec![]), &attributes);
        assert!(check_resolved(&attributes, &found, "").is_ok());
    }

    #[test]
    fn verbatim_prefixes_are_dropped_and_other_paths_kept() {
        let plain = |text: &str| without_verbatim_prefix(PathBuf::from(text));
        assert_eq!(plain(r"\\?\D:\a\project"), PathBuf::from(r"D:\a\project"));
        assert_eq!(
            plain(r"\\?\UNC\server\share\project"),
            PathBuf::from(r"\\server\share\project")
        );
        // A verbatim path that is not a drive or UNC path has no plain form.
        assert_eq!(
            plain(r"\\?\Volume{1}\project"),
            PathBuf::from(r"\\?\Volume{1}\project")
        );
        assert_eq!(
            plain("/private/var/project"),
            PathBuf::from("/private/var/project")
        );
    }

    #[test]
    fn a_root_reached_through_a_symlink_resolves_to_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        #[cfg(unix)]
        let root = {
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            link
        };
        // A link needs a privilege on Windows: check the plain form only.
        #[cfg(not(unix))]
        let root = real.clone();
        let expected = without_verbatim_prefix(std::fs::canonicalize(&real).unwrap());
        assert_eq!(project_root(&root).unwrap(), expected);
        // A root that does not exist stays as given, made absolute, for
        // scip-python to report.
        let missing = dir.path().join("missing");
        assert_eq!(
            project_root(&missing).unwrap(),
            std::path::absolute(&missing).unwrap()
        );
    }

    #[test]
    fn function_declarations_are_found() {
        assert!(declares_function("def main():\n    pass"));
        assert!(declares_function("class A:\n    async def\trun(self): ..."));
        assert!(declares_function("def _private(): ..."));
    }

    #[test]
    fn non_declarations_are_not() {
        assert!(!declares_function("default = 1"));
        assert!(!declares_function("undef(x)"));
        assert!(!declares_function("# nothing"));
        assert!(!declares_function("def"));
    }

    #[test]
    fn a_missing_program_fails_to_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let provider = PythonProvider::with_programs(
            dir.path().join("no-such-scip-python"),
            dir.path().join("no-such-python"),
        );
        let error = provider
            .index(dir.path(), &dir.path().join("out"))
            .unwrap_err();
        assert!(
            matches!(error, PythonError::Scip(ProviderError::Spawn { .. })),
            "{error}"
        );
        assert!(matches!(
            provider.scip_python_version().unwrap_err(),
            ProviderError::Spawn { .. }
        ));
    }

    /// The quoted value of `key = "..."` under `[section]` in `text`.
    fn toml_string(text: &str, section: &str, key: &str) -> String {
        let mut in_section = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_section = line == section;
            } else if in_section
                && let Some((k, v)) = line.split_once('=')
                && k.trim() == key
            {
                return v.trim().trim_matches('"').to_string();
            }
        }
        panic!("no {key} under {section}")
    }

    #[test]
    fn providers_toml_pins_scip_python_as_an_npm_package() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let providers = std::fs::read_to_string(repo.join("providers.toml")).unwrap();
        let spec = toml_string(&providers, "[scip-python]", "npm");
        let version = spec
            .strip_prefix("@sourcegraph/scip-python@")
            .unwrap_or_else(|| panic!("not a scip-python npm spec: {spec}"));
        assert!(
            version.split('.').count() == 3 && version.split('.').all(|p| p.parse::<u32>().is_ok()),
            "an exact version, not a range: {version}"
        );
    }
}
