//! The resolution report and the edge-count regression checks (brief §4.4,
//! "Validation output"; PLAN.md §2.2, §9 P1.8): `codetags report`.
//!
//! [`report`] reads one complete generation and never writes to it.
//!
//! # Resolution
//!
//! Every `call_site` row is counted once, under the language of its file
//! (`file.language`) and its caller's package and innermost module
//! (`symbol.package`, `symbol.module`; the module is `None` at a package
//! root). The package keeps same-named modules of different crates apart.
//!
//! - A call site is **resolved** when it has at least one `call_target` row
//!   whose method is not `name-match`: a declared target, or one a dispatch
//!   analysis found (`cha`, `rta`, `vta`, `jelly`, `di-binding`).
//! - It is **unresolved** otherwise: it has no `call_target` at all, or only
//!   `name-match` candidates, which are guesses by name for what the
//!   provider could not resolve (brief §4.4, Python row).
//! - The **unresolved rate** is unresolved over call sites, and undefined
//!   with no call sites.
//!
//! What this counts today: only Rust is ingested into generations, and
//! ingest writes every Rust call site with its declared target, so Rust
//! reports nothing unresolved. References the provider could not resolve
//! at all never become call sites in Rust (calls inside macro bodies,
//! through closures; V65). Python's unresolved name sites (V94) are still
//! provider output, not generation rows; they count here once Python ingest
//! writes them as call sites without a declared target, with or without
//! name-match candidates (V99).
//!
//! # Edge counts
//!
//! A generation's edge counts are its `run_file` rows, summed per provider
//! and path ([`EdgeCounts`]). Two checks read them:
//!
//! - **Against the previous complete generation** (PLAN.md §2.2 keeps it):
//!   a file that loses more than [`DROP_THRESHOLD_PERCENT`] of its edges,
//!   or drops to zero from non-zero, fails ([`regressions`]). A sudden drop
//!   is a provider failure, not a code change (brief §4.4).
//! - **Against a baseline**, a JSON file of [`EdgeCounts`] committed for each
//!   provider fixture under `tests/baselines/`: any difference fails
//!   ([`diff_baseline`]). Pin the providers; after any indexer or binding
//!   upgrade the baselines catch a provider that silently reads less (brief
//!   §4.4).

mod check;
mod query;
mod render;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use codetags_model::{GenerationStore, StoreError};
use serde::Serialize;

pub use self::check::{
    BaselineDifference, DROP_THRESHOLD_PERCENT, EdgeDrop, diff_baseline, is_drop, regressions,
};

/// Per-file edge counts: provider, then project-relative path, then count.
/// This is also the baseline file's format.
pub type EdgeCounts = BTreeMap<String, BTreeMap<String, i64>>;

/// Call sites, and how many of them resolved (see the module docs).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    /// Call sites.
    pub call_sites: u64,
    /// Resolved call sites.
    pub resolved: u64,
    /// Unresolved call sites.
    pub unresolved: u64,
}

impl Counts {
    /// `call_sites` sites, `resolved` of them resolved.
    pub fn new(call_sites: u64, resolved: u64) -> Self {
        Self {
            call_sites,
            resolved,
            unresolved: call_sites.saturating_sub(resolved),
        }
    }

    /// Unresolved over call sites, in `0.0..=1.0`; `None` with no call
    /// sites.
    #[allow(clippy::cast_precision_loss)]
    pub fn unresolved_rate(&self) -> Option<f64> {
        (self.call_sites > 0).then(|| self.unresolved as f64 / self.call_sites as f64)
    }

    fn add(&mut self, other: &Self) {
        self.call_sites += other.call_sites;
        self.resolved += other.resolved;
        self.unresolved += other.unresolved;
    }
}

/// A counts row as JSON: its labels, its counts, and its unresolved rate.
struct WithRate<'a> {
    labels: &'a [(&'static str, &'a Option<String>)],
    counts: &'a Counts,
}

impl Serialize for WithRate<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        for (key, value) in self.labels {
            map.serialize_entry(key, value)?;
        }
        map.serialize_entry("call_sites", &self.counts.call_sites)?;
        map.serialize_entry("resolved", &self.counts.resolved)?;
        map.serialize_entry("unresolved", &self.counts.unresolved)?;
        map.serialize_entry("unresolved_rate", &self.counts.unresolved_rate())?;
        map.end()
    }
}

/// The counts of one module of one package and language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleCounts {
    /// The language of the call sites' files; `None` if unknown.
    pub language: Option<String>,
    /// The callers' SCIP package (`symbol.package`), e.g. a crate; `None` if
    /// unknown.
    pub package: Option<String>,
    /// The callers' innermost dotted module; `None` at a package root.
    pub module: Option<String>,
    /// The counts.
    pub counts: Counts,
}

impl Serialize for ModuleCounts {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        WithRate {
            labels: &[
                ("language", &self.language),
                ("package", &self.package),
                ("module", &self.module),
            ],
            counts: &self.counts,
        }
        .serialize(serializer)
    }
}

/// The counts of one language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageCounts {
    /// The language; `None` if unknown.
    pub language: Option<String>,
    /// The counts.
    pub counts: Counts,
}

impl Serialize for LanguageCounts {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        WithRate {
            labels: &[("language", &self.language)],
            counts: &self.counts,
        }
        .serialize(serializer)
    }
}

/// The total counts, serialized with their rate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Total(pub Counts);

impl Serialize for Total {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        WithRate {
            labels: &[],
            counts: &self.0,
        }
        .serialize(serializer)
    }
}

/// The previous-generation check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Regression {
    /// The generation compared with; `None` when there is no earlier
    /// complete generation, and nothing was checked.
    pub previous: Option<u64>,
    /// [`DROP_THRESHOLD_PERCENT`].
    pub threshold_percent: u32,
    /// The files that dropped. Any fails the report.
    pub drops: Vec<EdgeDrop>,
}

/// The baseline check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaselineCheck {
    /// The baseline file, as given.
    pub path: PathBuf,
    /// Every difference. Any fails the report.
    pub differences: Vec<BaselineDifference>,
}

/// A generation's resolution report and its edge-count checks.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// The generation reported.
    pub generation: u64,
    /// Per language, package and module, ordered by language, package, then
    /// module (package roots first).
    pub modules: Vec<ModuleCounts>,
    /// Per language.
    pub languages: Vec<LanguageCounts>,
    /// Every call site.
    pub total: Total,
    /// The previous-generation check.
    pub regression: Regression,
    /// The baseline check, when a baseline was given.
    pub baseline: Option<BaselineCheck>,
    /// The generation's edge counts.
    pub edge_counts: EdgeCounts,
}

impl Report {
    /// Whether every check passed: no drop since the previous generation,
    /// and no difference from the baseline.
    pub fn passed(&self) -> bool {
        self.regression.drops.is_empty()
            && self
                .baseline
                .as_ref()
                .is_none_or(|baseline| baseline.differences.is_empty())
    }

    /// The report as JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

/// What to report.
#[derive(Debug, Clone, Default)]
pub struct ReportOptions {
    /// The generation; by default the newest complete one.
    pub generation: Option<u64>,
    /// A baseline file to compare the edge counts with.
    pub baseline: Option<PathBuf>,
}

/// Why [`report`] failed. A failed check is not an error: see
/// [`Report::passed`].
#[derive(Debug)]
pub enum ReportError {
    /// The generation store failed, or holds no such generation.
    Store(StoreError),
    /// The baseline file could not be read.
    BaselineRead {
        /// The file.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// The baseline file is not a JSON object of per-file edge counts per
    /// provider.
    BaselineFormat {
        /// The file.
        path: PathBuf,
        /// Why.
        source: serde_json::Error,
    },
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{error}"),
            Self::BaselineRead { path, source } => {
                write!(f, "reading baseline {}: {source}", path.display())
            }
            Self::BaselineFormat { path, source } => write!(
                f,
                "baseline {}: not a JSON object of edge counts per provider and file: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ReportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            Self::BaselineRead { source, .. } => Some(source),
            Self::BaselineFormat { source, .. } => Some(source),
        }
    }
}

impl From<StoreError> for ReportError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<codetags_model::Error> for ReportError {
    fn from(error: codetags_model::Error) -> Self {
        Self::Store(StoreError::Duckdb(error))
    }
}

/// Parses a baseline file's contents.
pub fn parse_baseline(text: &str) -> Result<EdgeCounts, serde_json::Error> {
    serde_json::from_str(text)
}

/// A baseline file's contents for `counts`: pretty JSON, sorted, with a
/// final newline.
pub fn baseline_json(counts: &EdgeCounts) -> String {
    let mut text = serde_json::to_string_pretty(counts).unwrap_or_default();
    text.push('\n');
    text
}

/// Reads and parses the baseline at `path`.
pub fn read_baseline(path: &Path) -> Result<EdgeCounts, ReportError> {
    let text = std::fs::read_to_string(path).map_err(|source| ReportError::BaselineRead {
        path: path.to_path_buf(),
        source,
    })?;
    parse_baseline(&text).map_err(|source| ReportError::BaselineFormat {
        path: path.to_path_buf(),
        source,
    })
}

/// Reports a generation of the index at `index`, for the project at `root`
/// (which tells a deleted file from one the provider lost).
pub fn report(root: &Path, index: &Path, options: &ReportOptions) -> Result<Report, ReportError> {
    let store = GenerationStore::new(index);
    let generation = match options.generation {
        Some(number) => store.open(number)?,
        None => store.open_newest()?,
    };
    let number = generation.number();
    let db = generation.connect()?;
    let (modules, languages, total) = query::resolution(&db)?;
    let edge_counts = query::edge_counts(&db)?;

    let previous = store
        .complete_generations()?
        .into_iter()
        .rev()
        .find(|&n| n < number);
    let drops = match previous {
        Some(previous) => {
            let before = query::edge_counts(&store.open(previous)?.connect()?)?;
            regressions(&before, &edge_counts, |path| root.join(path).exists())
        }
        None => Vec::new(),
    };

    let baseline = match &options.baseline {
        Some(path) => Some(BaselineCheck {
            path: path.clone(),
            differences: diff_baseline(&read_baseline(path)?, &edge_counts),
        }),
        None => None,
    };

    Ok(Report {
        generation: number,
        modules,
        languages,
        total: Total(total),
        regression: Regression {
            previous,
            threshold_percent: DROP_THRESHOLD_PERCENT,
            drops,
        },
        baseline,
        edge_counts,
    })
}

#[cfg(test)]
mod tests;
