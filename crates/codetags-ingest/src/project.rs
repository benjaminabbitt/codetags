//! Indexing a project into a new generation: `codetags index` (PLAN.md
//! §2.2, §2.3).
//!
//! [`index_project`] creates `<root>/.codetags/` with its `.gitignore` if
//! missing, takes the index's write lock by beginning the next generation,
//! runs every provider the project calls for, ingests each run, completes
//! the generation, and garbage-collects old ones. Any provider failure
//! fails the whole run, and no generation is completed (brief §4.4: failures
//! are loud). The incomplete generation is never read, and the next GC
//! deletes it.
//!
//! Providers are detected from the root: `Cargo.toml` means Rust. Each
//! provider writes its index and stderr under `.codetags/index/providers/`,
//! which is ignored and never mistaken for a generation.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use codetags_model::{GcReport, GenerationStore, StoreError};

use crate::ingest::{self, IngestError, IngestReport, RunInfo};
use crate::provider::ProviderError;
use crate::provider::rust::{self, RustAnalyzer};

/// The project's data directory, relative to the root.
pub const DATA_DIR: &str = ".codetags";

/// The index directory, relative to the data directory.
pub const INDEX_DIR: &str = "index";

/// The `.gitignore` written into a new data directory (PLAN.md §2.3).
pub const GITIGNORE: &str =
    "# codetags: derived and local data, never committed.\nindex/\n*.lock\nlocal/\n";

/// A provider [`index_project`] can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// `rust-analyzer scip`, for a root holding `Cargo.toml`.
    Rust,
}

impl Provider {
    /// The provider's name, as in the `run` table.
    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => rust::PROVIDER,
        }
    }

    fn language(self) -> &'static str {
        match self {
            Self::Rust => "rust",
        }
    }

    fn directory(self) -> &'static str {
        match self {
            Self::Rust => "rust-analyzer",
        }
    }
}

/// The providers a project at `root` calls for.
pub fn detect(root: &Path) -> Vec<Provider> {
    let mut providers = Vec::new();
    if root.join("Cargo.toml").is_file() {
        providers.push(Provider::Rust);
    }
    providers
}

/// How to run the providers.
#[derive(Debug, Clone, Default)]
pub struct IndexOptions {
    /// The Rust provider; by default the `rust-analyzer` on `PATH`.
    pub rust_analyzer: RustAnalyzer,
}

/// One provider's part of an indexing run.
#[derive(Debug)]
pub struct ProviderRun {
    /// Which provider.
    pub provider: Provider,
    /// Its full version.
    pub version: String,
    /// What its ingest wrote and skipped.
    pub report: IngestReport,
}

/// What [`index_project`] did.
#[derive(Debug)]
pub struct IndexSummary {
    /// The index directory.
    pub index: PathBuf,
    /// The generation completed.
    pub generation: u64,
    /// Each provider's run, in order.
    pub runs: Vec<ProviderRun>,
    /// What the GC after completing did.
    pub gc: GcReport,
}

/// Why [`index_project`] failed.
#[derive(Debug)]
pub enum IndexError {
    /// The root holds no project any provider indexes.
    NoProviders {
        /// The root.
        root: PathBuf,
    },
    /// Creating `.codetags/` or its `.gitignore` failed.
    DataDir {
        /// The path.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// A provider failed.
    Provider {
        /// Which.
        provider: Provider,
        /// Why.
        source: ProviderError,
    },
    /// Ingesting a provider's index failed.
    Ingest {
        /// Which provider's.
        provider: Provider,
        /// Why.
        source: IngestError,
    },
    /// The generation store failed.
    Store(StoreError),
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoProviders { root } => write!(
                f,
                "{}: no project to index (supported: a Cargo.toml for Rust)",
                root.display()
            ),
            Self::DataDir { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Provider { provider, source } => write!(f, "{}: {source}", provider.name()),
            Self::Ingest { provider, source } => {
                write!(f, "ingesting {}: {source}", provider.name())
            }
            Self::Store(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for IndexError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoProviders { .. } => None,
            Self::DataDir { source, .. } => Some(source),
            Self::Provider { source, .. } => Some(source),
            Self::Ingest { source, .. } => Some(source),
            Self::Store(error) => Some(error),
        }
    }
}

impl From<StoreError> for IndexError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// Creates `<root>/.codetags/` and its `.gitignore` if missing. An existing
/// `.gitignore` is left as it is. Returns the data directory.
pub fn ensure_data_dir(root: &Path) -> Result<PathBuf, IndexError> {
    let data = root.join(DATA_DIR);
    let fail = |path: &Path| {
        let path = path.to_path_buf();
        move |source| IndexError::DataDir { path, source }
    };
    std::fs::create_dir_all(&data).map_err(fail(&data))?;
    let gitignore = data.join(".gitignore");
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&gitignore)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(GITIGNORE.as_bytes())
                .map_err(fail(&gitignore))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(fail(&gitignore)(error)),
    }
    Ok(data)
}

/// Indexes the project at `root` into a new generation of
/// `<root>/.codetags/index/`.
pub fn index_project(root: &Path, options: &IndexOptions) -> Result<IndexSummary, IndexError> {
    let providers = detect(root);
    if providers.is_empty() {
        return Err(IndexError::NoProviders {
            root: root.to_path_buf(),
        });
    }
    let index = ensure_data_dir(root)?.join(INDEX_DIR);
    let store = GenerationStore::new(&index);
    let writer = store.begin()?;
    let mut runs = Vec::new();
    for provider in providers {
        runs.push(run_provider(provider, root, &index, options, &writer)?);
    }
    let generation = writer.complete()?;
    let gc = store.gc()?;
    Ok(IndexSummary {
        index,
        generation,
        runs,
        gc,
    })
}

fn run_provider(
    provider: Provider,
    root: &Path,
    index: &Path,
    options: &IndexOptions,
    writer: &codetags_model::GenerationWriter,
) -> Result<ProviderRun, IndexError> {
    let out = index.join("providers").join(provider.directory());
    let provider_error = |source| IndexError::Provider { provider, source };
    let started_at = SystemTime::now();
    let (version, scip) = match provider {
        Provider::Rust => {
            let version = options.rust_analyzer.version().map_err(provider_error)?;
            let result = options.rust_analyzer.index(root, &out);
            // Keep the provider's stderr next to its output, pass or fail.
            if let Some(stderr) = match &result {
                Ok(run) => Some(run.stderr.as_str()),
                Err(error) => error.stderr(),
            } {
                let _ = std::fs::write(out.join("stderr.log"), stderr);
            }
            (version, result.map_err(provider_error)?)
        }
    };
    let finished_at = SystemTime::now();
    let report = ingest::ingest(
        writer.connection(),
        &ingest::Input {
            root,
            index: &scip.index,
            language: provider.language(),
            run: RunInfo {
                provider: provider.name().to_string(),
                provider_version: version.clone(),
                args: vec![
                    "scip".to_string(),
                    root.display().to_string(),
                    "--output".to_string(),
                    scip.index_path.display().to_string(),
                ],
                started_at,
                finished_at,
            },
        },
    )
    .map_err(|source| IndexError::Ingest { provider, source })?;
    Ok(ProviderRun {
        provider,
        version,
        report,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn cargo_toml_means_rust() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect(dir.path()), []);
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        assert_eq!(detect(dir.path()), [Provider::Rust]);
    }

    #[test]
    fn no_project_is_an_error_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let error = index_project(dir.path(), &IndexOptions::default()).unwrap_err();
        assert!(matches!(error, IndexError::NoProviders { .. }), "{error}");
        assert!(!dir.path().join(DATA_DIR).exists());
    }

    #[test]
    fn the_data_dir_gets_a_gitignore_once() {
        let dir = tempfile::tempdir().unwrap();
        let data = ensure_data_dir(dir.path()).unwrap();
        let gitignore = data.join(".gitignore");
        assert_eq!(std::fs::read_to_string(&gitignore).unwrap(), GITIGNORE);
        std::fs::write(&gitignore, "custom\n").unwrap();
        ensure_data_dir(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&gitignore).unwrap(), "custom\n");
    }

    #[test]
    fn a_failing_provider_completes_no_generation() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        let options = IndexOptions {
            rust_analyzer: RustAnalyzer::with_program(dir.path().join("no-such-rust-analyzer")),
        };
        let error = index_project(dir.path(), &options).unwrap_err();
        assert!(
            matches!(
                error,
                IndexError::Provider {
                    provider: Provider::Rust,
                    ..
                }
            ),
            "{error}"
        );
        let store = GenerationStore::new(dir.path().join(DATA_DIR).join(INDEX_DIR));
        assert_eq!(store.complete_generations().unwrap(), Vec::<u64>::new());
    }
}
