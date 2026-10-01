//! The Rust provider: `rust-analyzer scip` (brief §4.4 Rust row).
//!
//! The pinned rust-analyzer is the `rust-analyzer` component of the toolchain
//! in `rust-toolchain.toml` (`providers.toml`, V62). The runner starts the
//! program by name, so on a rustup install the proxy picks the toolchain from
//! the current directory, as cargo does.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, Stdio};

use super::{ProviderError, ScipRun, check_output};

/// The file name the runner asks rust-analyzer to write in the output
/// directory.
pub const INDEX_FILE: &str = "index.scip";

/// Runs `rust-analyzer scip`.
#[derive(Debug, Clone)]
pub struct RustAnalyzer {
    program: OsString,
    envs: Vec<(OsString, OsString)>,
}

impl Default for RustAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl RustAnalyzer {
    /// The `rust-analyzer` found on `PATH`.
    pub fn new() -> Self {
        Self::with_program("rust-analyzer")
    }

    /// A specific rust-analyzer executable.
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            envs: Vec::new(),
        }
    }

    /// Sets an environment variable for the provider, e.g.
    /// `CARGO_TARGET_DIR` to keep its `cargo check` out of the project.
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

    fn program(&self) -> String {
        self.program.to_string_lossy().into_owned()
    }

    /// `rust-analyzer --version`, e.g. `rust-analyzer 1.96.1 (31fca3a 2026-06-26)`.
    pub fn version(&self) -> Result<String, ProviderError> {
        let output = self
            .command()
            .arg("--version")
            .stdin(Stdio::null())
            .output()
            .map_err(|source| ProviderError::Spawn {
                program: self.program(),
                source,
            })?;
        if !output.status.success() {
            return Err(ProviderError::Exit {
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Indexes the Cargo project at `root` into `out_dir/index.scip`, creating
    /// `out_dir` if needed and replacing a previous index there, and checks
    /// the result (see [`super`] for what fails).
    pub fn index(&self, root: &Path, out_dir: &Path) -> Result<ScipRun, ProviderError> {
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
            Err(error) => return Err(out_error(error)),
        }
        let output = self
            .command()
            .arg("scip")
            .arg(root)
            .arg("--output")
            .arg(&index_path)
            .stdin(Stdio::null())
            .output()
            .map_err(|source| ProviderError::Spawn {
                program: self.program(),
                source,
            })?;
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.status.success() {
            return Err(ProviderError::Exit {
                status: output.status,
                stderr,
            });
        }
        check_output(root, &index_path, stderr, declares_function)
    }
}

/// Whether Rust `source` declares a function: the keyword `fn` followed by
/// a name. Lexical, so a commented-out `fn` counts too; it only decides
/// whether an index with no occurrences at all is suspicious.
pub fn declares_function(source: &str) -> bool {
    let is_ident = |c: char| c == '_' || c.is_alphanumeric();
    source.match_indices("fn").any(|(at, _)| {
        let before = source[..at].chars().next_back();
        let after = &source[at + 2..];
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

    use super::*;

    #[test]
    fn function_declarations_are_found() {
        assert!(declares_function("fn main() {}"));
        assert!(declares_function("pub(crate) async fn go()"));
        assert!(declares_function("impl X {\n    fn\tr#type(&self) {}\n}"));
    }

    #[test]
    fn non_declarations_are_not() {
        assert!(!declares_function("struct Fn;"));
        assert!(!declares_function("let f: fn(i64) -> i64 = g;"));
        assert!(!declares_function("let define = 1;"));
        assert!(!declares_function("// nothing"));
        assert!(!declares_function("fn"));
    }

    /// The quoted value of `key = "..."` in `text`, after the line `section`.
    fn toml_string(text: &str, section: &str, key: &str) -> String {
        let mut in_section = section.is_empty();
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
        panic!("no {key} under {section:?}")
    }

    #[test]
    fn providers_toml_pins_the_toolchain_of_rust_toolchain_toml() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let providers = std::fs::read_to_string(repo.join("providers.toml")).unwrap();
        let toolchain = std::fs::read_to_string(repo.join("rust-toolchain.toml")).unwrap();
        assert_eq!(
            toml_string(&providers, "[rust-analyzer]", "toolchain"),
            toml_string(&toolchain, "[toolchain]", "channel")
        );
    }

    #[test]
    fn a_missing_program_fails_to_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let provider = RustAnalyzer::with_program(dir.path().join("no-such-rust-analyzer"));
        let error = provider
            .index(dir.path(), &dir.path().join("out"))
            .unwrap_err();
        assert!(matches!(error, ProviderError::Spawn { .. }), "{error}");
        assert!(matches!(
            provider.version().unwrap_err(),
            ProviderError::Spawn { .. }
        ));
    }
}
