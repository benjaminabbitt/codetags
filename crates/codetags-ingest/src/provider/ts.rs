//! The TypeScript provider (brief §4.4 TypeScript row): `scip-typescript`
//! for symbols and references, and [Jelly](jelly) for call edges. Ingest
//! unions the two (P1.3).
//!
//! Both are npm packages pinned in `providers.toml` (V81, V83) and started
//! by name from `PATH`. npm installs a `.cmd` shim for each on Windows, so
//! the default program names end in `.cmd` there.
//!
//! Both tools log their errors to **stdout**, and Jelly exits 0 even when it
//! aborts (V83). So the runners read the log as well as the exit status, and
//! the "stderr" they attach to errors and runs is the tool's whole log:
//! stdout, then stderr.

pub mod jelly;

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::Path;
use std::process::{Command, Stdio};

use super::{ProviderError, ScipRun, check_output};

/// The file name the runner asks scip-typescript to write in the output
/// directory.
pub const INDEX_FILE: &str = "index.scip";

/// The runner's `--max-file-byte-size`. scip-typescript skips files over
/// 1 MB by default (brief §4.4, V81); the runner raises the limit, and a
/// file still skipped is an error ([`TsError::SkippedFiles`]).
pub const DEFAULT_MAX_FILE_BYTE_SIZE: &str = "64mb";

/// The default program for an npm-installed tool: its `.cmd` shim on
/// Windows, its name elsewhere.
pub(crate) fn npm_program(name: &str) -> OsString {
    if cfg!(windows) {
        OsString::from(format!("{name}.cmd"))
    } else {
        OsString::from(name)
    }
}

/// A finished process: its exit status and its log (stdout, then stderr).
pub(crate) struct Finished {
    pub(crate) status: std::process::ExitStatus,
    pub(crate) log: String,
}

/// Runs `command` with stdin closed and collects its log, or returns the
/// spawn error.
pub(crate) fn finish(command: &mut Command) -> std::io::Result<Finished> {
    let output = command.stdin(Stdio::null()).output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let log = match (stdout.trim_end(), stderr.trim_end()) {
        ("", stderr) => stderr.to_string(),
        (stdout, "") => stdout.to_string(),
        (stdout, stderr) => format!("{stdout}\n{stderr}"),
    };
    Ok(Finished {
        status: output.status,
        log,
    })
}

/// Why a scip-typescript run failed.
#[derive(Debug)]
pub enum TsError {
    /// A failure every SCIP provider can have (see [`super`]).
    Provider(ProviderError),
    /// scip-typescript skipped files over its size limit, so the index is
    /// missing them although it exited 0.
    SkippedFiles {
        /// The skipped files, as scip-typescript names them.
        files: Vec<String>,
        /// Its log.
        stderr: String,
    },
}

impl TsError {
    /// The provider's log, when it ran.
    pub fn stderr(&self) -> Option<&str> {
        match self {
            Self::Provider(error) => error.stderr(),
            Self::SkippedFiles { stderr, .. } => Some(stderr),
        }
    }
}

impl From<ProviderError> for TsError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(error)
    }
}

impl fmt::Display for TsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Provider(error) => error.fmt(f),
            Self::SkippedFiles { files, stderr } => write!(
                f,
                "scip-typescript skipped {} file(s) over its size limit: {}\n--- provider stderr ---\n{}",
                files.len(),
                files.join(", "),
                stderr.trim_end()
            ),
        }
    }
}

impl std::error::Error for TsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Provider(error) => Some(error),
            Self::SkippedFiles { .. } => None,
        }
    }
}

/// Runs `scip-typescript index`.
#[derive(Debug, Clone)]
pub struct ScipTypescript {
    program: OsString,
    envs: Vec<(OsString, OsString)>,
    max_file_byte_size: String,
}

impl Default for ScipTypescript {
    fn default() -> Self {
        Self::new()
    }
}

impl ScipTypescript {
    /// The `scip-typescript` found on `PATH`.
    pub fn new() -> Self {
        Self::with_program(npm_program("scip-typescript"))
    }

    /// A specific scip-typescript executable.
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            envs: Vec::new(),
            max_file_byte_size: DEFAULT_MAX_FILE_BYTE_SIZE.to_string(),
        }
    }

    /// Sets `--max-file-byte-size` (e.g. `"1mb"`) in place of
    /// [`DEFAULT_MAX_FILE_BYTE_SIZE`].
    pub fn max_file_byte_size(mut self, value: impl Into<String>) -> Self {
        self.max_file_byte_size = value.into();
        self
    }

    /// Sets an environment variable for the provider.
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

    fn spawn_error(&self, source: std::io::Error) -> ProviderError {
        ProviderError::Spawn {
            program: self.program.to_string_lossy().into_owned(),
            source,
        }
    }

    /// `scip-typescript --version`, e.g. `0.4.0`.
    pub fn version(&self) -> Result<String, ProviderError> {
        let run = finish(self.command().arg("--version")).map_err(|e| self.spawn_error(e))?;
        if !run.status.success() {
            return Err(ProviderError::Exit {
                status: run.status,
                stderr: run.log,
            });
        }
        Ok(run.log.trim().to_string())
    }

    /// Indexes the TypeScript project at `root` (which needs a
    /// `tsconfig.json`) into `out_dir/index.scip`, creating `out_dir` if
    /// needed and replacing a previous index there, and checks the result:
    /// see [`super`], plus [`TsError::SkippedFiles`].
    pub fn index(&self, root: &Path, out_dir: &Path) -> Result<ScipRun, TsError> {
        let index_path = out_dir.join(INDEX_FILE);
        let out_error = |source| ProviderError::OutputDir {
            path: out_dir.to_path_buf(),
            source,
        };
        std::fs::create_dir_all(out_dir).map_err(out_error)?;
        // A stale index must never pass for this run's output.
        match std::fs::remove_file(&index_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(out_error(error).into()),
        }
        let run = finish(
            self.command()
                .arg("index")
                .arg("--cwd")
                .arg(root)
                .arg("--output")
                .arg(&index_path)
                .arg("--no-progress-bar")
                .arg("--max-file-byte-size")
                .arg(&self.max_file_byte_size),
        )
        .map_err(|e| self.spawn_error(e))?;
        if !run.status.success() {
            return Err(ProviderError::Exit {
                status: run.status,
                stderr: run.log,
            }
            .into());
        }
        let files = skipped_files(&run.log);
        if !files.is_empty() {
            return Err(TsError::SkippedFiles {
                files,
                stderr: run.log,
            });
        }
        Ok(check_output(root, &index_path, run.log, declares_function)?)
    }
}

/// The files scip-typescript's log says it skipped for their size
/// (`info: skipping file '<path>' because it has byte size ...`, V81).
pub fn skipped_files(log: &str) -> Vec<String> {
    log.lines()
        .filter_map(|line| line.split_once("skipping file '"))
        .filter_map(|(_, rest)| rest.split_once("' because"))
        .map(|(path, _)| path.to_string())
        .collect()
}

/// Whether TypeScript or JavaScript `source` declares a function: the
/// keyword `function` or `class`, or an arrow `=>`. Lexical, like
/// [`super::rust::declares_function`]; it only decides whether an index
/// with no occurrences at all is suspicious.
pub fn declares_function(source: &str) -> bool {
    let is_ident = |c: char| c == '_' || c == '$' || c.is_alphanumeric();
    let keyword = |word: &str| {
        source.match_indices(word).any(|(at, _)| {
            let before = source[..at].chars().next_back();
            let after = source[at + word.len()..].chars().next();
            before.is_none_or(|c| !is_ident(c)) && after.is_none_or(|c| !is_ident(c))
        })
    };
    source.contains("=>") || keyword("function") || keyword("class")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn function_declarations_are_found() {
        assert!(declares_function("export function f() {}"));
        assert!(declares_function("const f = (x) => x;"));
        assert!(declares_function("export class Card {}"));
        assert!(declares_function("handlers.card = function (p) {};"));
    }

    #[test]
    fn non_declarations_are_not() {
        assert!(!declares_function("export const x = 1;"));
        assert!(!declares_function(
            "const functional = 1; const classy = 2;"
        ));
        assert!(!declares_function("// nothing"));
    }

    #[test]
    fn skipped_files_are_read_from_the_log() {
        let log = "info: skipping file '/p/src/huge.ts' because it has byte size 2.010036mb that exceeds the maximum threshold 1000kb. If you intended ...\n+ /p (16ms)\ndone /out/index.scip";
        assert_eq!(skipped_files(log), vec!["/p/src/huge.ts".to_string()]);
        assert!(skipped_files("+ /p (16ms)\ndone /out/index.scip").is_empty());
    }

    #[test]
    fn skipped_files_display_the_log() {
        let error = TsError::SkippedFiles {
            files: vec!["/p/src/huge.ts".into()],
            stderr: "info: skipping file '/p/src/huge.ts' because".into(),
        };
        let text = error.to_string();
        assert!(text.contains("skipped 1 file(s)"), "{text}");
        assert!(
            text.contains("--- provider stderr ---\ninfo: skipping"),
            "{text}"
        );
        assert_eq!(
            error.stderr(),
            Some("info: skipping file '/p/src/huge.ts' because")
        );
    }

    #[test]
    fn a_missing_program_fails_to_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let provider = ScipTypescript::with_program(dir.path().join("no-such-scip-typescript"));
        let error = provider
            .index(dir.path(), &dir.path().join("out"))
            .unwrap_err();
        assert!(
            matches!(error, TsError::Provider(ProviderError::Spawn { .. })),
            "{error}"
        );
        assert!(matches!(
            provider.version().unwrap_err(),
            ProviderError::Spawn { .. }
        ));
    }

    #[test]
    fn npm_programs_are_cmd_shims_on_windows() {
        let program = npm_program("jelly");
        if cfg!(windows) {
            assert_eq!(program, "jelly.cmd");
        } else {
            assert_eq!(program, "jelly");
        }
    }

    /// The quoted value of `key = "..."` under `[section]` in providers.toml.
    pub(crate) fn pin(section: &str, key: &str) -> String {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let text = std::fs::read_to_string(repo.join("providers.toml")).unwrap();
        let header = format!("[{section}]");
        let mut in_section = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_section = line == header;
            } else if in_section
                && let Some((k, v)) = line.split_once('=')
                && k.trim() == key
            {
                return v.trim().trim_matches('"').to_string();
            }
        }
        panic!("providers.toml has no {key} under {header}")
    }

    #[test]
    fn providers_toml_pins_exact_npm_versions() {
        for (section, package) in [
            ("scip-typescript", "@sourcegraph/scip-typescript@"),
            ("jelly", "@cs-au-dk/jelly@"),
        ] {
            let spec = pin(section, "npm");
            let version = spec
                .strip_prefix(package)
                .unwrap_or_else(|| panic!("{section}: {spec:?} is not {package}<version>"));
            assert!(
                version.split('.').count() == 3
                    && version.split('.').all(|part| part.parse::<u32>().is_ok()),
                "{section}: {spec:?} is not pinned to an exact version"
            );
        }
        let node = pin("node", "version");
        assert_eq!(node.split('.').count(), 3, "node {node:?} is not exact");
    }
}
