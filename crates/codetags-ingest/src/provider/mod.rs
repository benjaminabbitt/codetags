//! SCIP provider runners (brief §4.4, PLAN.md §9).
//!
//! A runner executes one provider over a project root into a scratch output
//! directory and returns the index it wrote. Every failure is loud (brief
//! §4.4: a silent provider failure once cut a call graph by 81%): a non-zero
//! exit, a missing or empty index, an index with no documents, and an index
//! with no occurrences for sources that declare functions are all errors, and
//! each carries the provider's stderr.

pub mod go;
pub mod rust;
pub mod scip;
pub mod ts;

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use self::scip::{ReadError, ScipIndex};

/// A provider run that passed every check.
#[derive(Debug)]
pub struct ScipRun {
    /// The `index.scip` the provider wrote, inside the output directory.
    pub index_path: PathBuf,
    /// The decoded index.
    pub index: ScipIndex,
    /// The provider's stderr. A run can pass and still log problems here
    /// (rust-analyzer logs derive definitions it could not place, V65).
    pub stderr: String,
}

/// Why a provider run failed. Every variant that ran the provider carries
/// its stderr.
#[derive(Debug)]
pub enum ProviderError {
    /// The provider could not be started (not installed, not on PATH).
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
    /// The provider exited non-zero or was killed.
    Exit {
        /// Its exit status.
        status: ExitStatus,
        /// Its stderr.
        stderr: String,
    },
    /// The provider exited 0 but wrote no index.
    MissingOutput {
        /// Where the index should be.
        path: PathBuf,
        /// Its stderr.
        stderr: String,
    },
    /// The provider wrote an empty index file.
    EmptyOutput {
        /// The index file.
        path: PathBuf,
        /// Its stderr.
        stderr: String,
    },
    /// The index could not be read or decoded.
    Unreadable {
        /// Why.
        source: ReadError,
        /// Its stderr.
        stderr: String,
    },
    /// The index holds no documents.
    NoDocuments {
        /// Its stderr.
        stderr: String,
    },
    /// The index holds no occurrences although the indexed sources declare
    /// functions.
    NoOccurrences {
        /// A document whose source declares a function.
        example: String,
        /// Its stderr.
        stderr: String,
    },
}

impl ProviderError {
    /// The provider's stderr, when it ran.
    pub fn stderr(&self) -> Option<&str> {
        match self {
            Self::Spawn { .. } | Self::OutputDir { .. } => None,
            Self::Exit { stderr, .. }
            | Self::MissingOutput { stderr, .. }
            | Self::EmptyOutput { stderr, .. }
            | Self::Unreadable { stderr, .. }
            | Self::NoDocuments { stderr }
            | Self::NoOccurrences { stderr, .. } => Some(stderr),
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { program, source } => {
                return write!(f, "provider {program} could not start: {source}");
            }
            Self::OutputDir { path, source } => {
                return write!(f, "output directory {}: {source}", path.display());
            }
            Self::Exit { status, .. } => write!(f, "provider failed ({status})")?,
            Self::MissingOutput { path, .. } => {
                write!(f, "provider exited 0 but wrote no {}", path.display())?
            }
            Self::EmptyOutput { path, .. } => {
                write!(f, "provider wrote an empty {}", path.display())?
            }
            Self::Unreadable { source, .. } => write!(f, "provider output unreadable: {source}")?,
            Self::NoDocuments { .. } => write!(f, "provider index has no documents")?,
            Self::NoOccurrences { example, .. } => write!(
                f,
                "provider index has no occurrences, but {example} declares functions"
            )?,
        }
        if let Some(stderr) = self.stderr() {
            write!(f, "\n--- provider stderr ---\n{}", stderr.trim_end())?;
        }
        Ok(())
    }
}

impl std::error::Error for ProviderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } | Self::OutputDir { source, .. } => Some(source),
            Self::Unreadable { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Checks the index a provider wrote to `index_path` after exiting 0, and
/// decodes it. `declares_functions` says whether a document's source (read
/// from `root`) declares a function, the language-specific part of the
/// zero-occurrence check.
pub(crate) fn check_output(
    root: &Path,
    index_path: &Path,
    stderr: String,
    declares_functions: impl Fn(&str) -> bool,
) -> Result<ScipRun, ProviderError> {
    let size = match std::fs::metadata(index_path) {
        Ok(metadata) => metadata.len(),
        Err(_) => {
            return Err(ProviderError::MissingOutput {
                path: index_path.to_path_buf(),
                stderr,
            });
        }
    };
    if size == 0 {
        return Err(ProviderError::EmptyOutput {
            path: index_path.to_path_buf(),
            stderr,
        });
    }
    let index = match scip::read_index(index_path) {
        Ok(index) => index,
        Err(source) => return Err(ProviderError::Unreadable { source, stderr }),
    };
    if index.documents.is_empty() {
        return Err(ProviderError::NoDocuments { stderr });
    }
    let occurrences: usize = index.documents.iter().map(|d| d.occurrences.len()).sum();
    if occurrences == 0 {
        let example = index.documents.iter().find(|document| {
            std::fs::read_to_string(root.join(&document.relative_path))
                .is_ok_and(|source| declares_functions(&source))
        });
        if let Some(document) = example {
            return Err(ProviderError::NoOccurrences {
                example: document.relative_path.clone(),
                stderr,
            });
        }
    }
    Ok(ScipRun {
        index_path: index_path.to_path_buf(),
        index,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::scip::tests::{index_bytes, raw_occurrence};
    use super::*;

    fn always(_: &str) -> bool {
        true
    }

    fn check(dir: &Path, bytes: Option<&[u8]>) -> Result<ScipRun, ProviderError> {
        let path = dir.join("index.scip");
        if let Some(bytes) = bytes {
            std::fs::write(&path, bytes).unwrap();
        }
        check_output(dir, &path, "the stderr".to_string(), always)
    }

    #[test]
    fn missing_output_is_an_error_with_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let error = check(dir.path(), None).unwrap_err();
        assert!(
            matches!(error, ProviderError::MissingOutput { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("the stderr"), "{error}");
    }

    #[test]
    fn empty_output_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let error = check(dir.path(), Some(b"")).unwrap_err();
        assert!(
            matches!(error, ProviderError::EmptyOutput { .. }),
            "{error}"
        );
        assert_eq!(error.stderr(), Some("the stderr"));
    }

    #[test]
    fn undecodable_output_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let error = check(dir.path(), Some(b"\xff\xff\xff")).unwrap_err();
        assert!(matches!(error, ProviderError::Unreadable { .. }), "{error}");
    }

    #[test]
    fn an_index_without_documents_is_an_error() {
        use protobuf::Message;
        let mut index = ::scip::types::Index::new();
        index.metadata.mut_or_insert_default().project_root = "file:///x".into();
        let bytes = index.write_to_bytes().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let error = check(dir.path(), Some(&bytes)).unwrap_err();
        assert!(
            matches!(error, ProviderError::NoDocuments { .. }),
            "{error}"
        );
    }

    #[test]
    fn no_occurrences_where_functions_are_declared_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "fn f() {}").unwrap();
        let error = check(dir.path(), Some(&index_bytes("src/a.rs", vec![]))).unwrap_err();
        assert!(
            matches!(error, ProviderError::NoOccurrences { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("src/a.rs"), "{error}");
    }

    #[test]
    fn no_occurrences_without_functions_passes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "// nothing").unwrap();
        let path = dir.path().join("index.scip");
        std::fs::write(&path, index_bytes("src/a.rs", vec![])).unwrap();
        let run = check_output(dir.path(), &path, String::new(), |_| false).unwrap();
        assert_eq!(run.index.documents.len(), 1);
    }

    #[test]
    fn a_good_index_passes() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = index_bytes("src/a.rs", vec![raw_occurrence("s", &[0, 0, 1], &[], 1)]);
        let run = check(dir.path(), Some(&bytes)).unwrap();
        assert_eq!(run.index_path, dir.path().join("index.scip"));
        assert_eq!(run.index.documents[0].occurrences.len(), 1);
        assert_eq!(run.stderr, "the stderr");
    }
}
