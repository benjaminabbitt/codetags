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
//!
//! After the Rust provider's SCIP ingest, the implementation pass expands
//! calls of the project's trait methods to their implementations through
//! rust-analyzer's `textDocument/implementation` (PLAN.md D31;
//! [`crate::ingest::implementations`]). It is its own run in the
//! generation, and it fails the run as loudly as a provider does.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use codetags_model::{GcReport, GenerationStore, GenerationWriter, StoreError};

use crate::ingest::implementations::{self, Expansion};
use crate::ingest::{self, IngestError, IngestReport, RunInfo};
use crate::provider::ProviderError;
use crate::provider::rust::implementations::ImplError;
use crate::provider::rust::{self, IMPL_PROVIDER, RustAnalyzer};
use crate::provider::scip::ScipIndex;

/// How long the implementation pass waits for rust-analyzer to load the
/// workspace by default. Generous: loading this repository took about 1 s
/// with a warm target directory, 15 s with a cold one, and over a minute
/// while cargo held the target directory's lock (V141, V160).
pub const DEFAULT_LOAD_TIMEOUT: Duration = Duration::from_secs(600);

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
#[derive(Debug, Clone)]
pub struct IndexOptions {
    /// The Rust provider; by default the `rust-analyzer` on `PATH`.
    pub rust_analyzer: RustAnalyzer,
    /// How long the implementation pass waits for rust-analyzer to load the
    /// workspace; [`DEFAULT_LOAD_TIMEOUT`] by default.
    pub rust_analyzer_load_timeout: Duration,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            rust_analyzer: RustAnalyzer::default(),
            rust_analyzer_load_timeout: DEFAULT_LOAD_TIMEOUT,
        }
    }
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
    /// What the implementation pass added (Rust); `None` when there was no
    /// call of a project trait method to expand, so it did not run.
    pub implementations: Option<Expansion>,
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
    /// The implementation pass (D31) failed to get its answers.
    Implementations(ImplError),
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
            Self::Implementations(source) => write!(f, "{IMPL_PROVIDER}: {source}"),
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
            Self::Implementations(source) => Some(source),
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
    let implementations = match provider {
        Provider::Rust => expand_implementations(
            root,
            &out,
            options,
            writer,
            &scip.index,
            report.run_id,
            &version,
        )?,
    };
    Ok(ProviderRun {
        provider,
        version,
        report,
        implementations,
    })
}

/// The implementation pass over the Rust SCIP run `scip_run` (D31): asks
/// rust-analyzer about every called trait method of the project, and writes
/// what it answers. `None` when nothing is called, so nothing is asked.
fn expand_implementations(
    root: &Path,
    out: &Path,
    options: &IndexOptions,
    writer: &GenerationWriter,
    index: &ScipIndex,
    scip_run: i64,
    version: &str,
) -> Result<Option<Expansion>, IndexError> {
    let provider = Provider::Rust;
    let ingest_error = |source| IndexError::Ingest { provider, source };
    let questions =
        implementations::questions(writer.connection(), index, scip_run).map_err(ingest_error)?;
    if questions.is_empty() {
        return Ok(None);
    }
    // The server speaks URIs, so it gets an absolute root; the canonical
    // one, so its answers' paths compare with it.
    let absolute = std::fs::canonicalize(root)
        .or_else(|_| std::path::absolute(root))
        .map_err(|source| IndexError::DataDir {
            path: root.to_path_buf(),
            source,
        })?;
    let started_at = SystemTime::now();
    let answers = options
        .rust_analyzer
        .implementations(
            &absolute,
            &questions.positions,
            options.rust_analyzer_load_timeout,
            &out.join("lsp-stderr.log"),
        )
        .map_err(IndexError::Implementations)?;
    let run = RunInfo {
        provider: IMPL_PROVIDER.to_string(),
        provider_version: version.to_string(),
        args: vec![
            "lsp".to_string(),
            "textDocument/implementation".to_string(),
            absolute.display().to_string(),
        ],
        started_at,
        finished_at: SystemTime::now(),
    };
    implementations::write(writer.connection(), &questions, &answers, &run)
        .map(Some)
        .map_err(ingest_error)
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
            ..IndexOptions::default()
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
