//! The report as text for people.

use std::fmt::Write;

use super::{Counts, Report};

/// A package root's module, in text.
const ROOT: &str = "(root)";
/// A language's own row, in the package column.
const ALL: &str = "(all)";
/// An unknown language.
const UNKNOWN: &str = "(unknown)";

/// `n` and the noun for it, e.g. "1 file" or "3 files".
fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn rate(counts: &Counts) -> String {
    counts
        .unresolved_rate()
        .map_or_else(|| "-".to_string(), |rate| format!("{:.1}%", rate * 100.0))
}

/// Left-aligned columns, two spaces apart, with no trailing spaces.
fn table(rows: &[Vec<String>]) -> String {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|i| {
            rows.iter()
                .filter_map(|row| row.get(i))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for row in rows {
        let mut line = String::new();
        for (cell, width) in row.iter().zip(&widths) {
            let _ = write!(line, "{cell:<width$}  ");
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn row(language: &str, package: &str, module: &str, counts: &Counts) -> Vec<String> {
    vec![
        language.to_string(),
        package.to_string(),
        module.to_string(),
        counts.call_sites.to_string(),
        counts.resolved.to_string(),
        counts.unresolved.to_string(),
        rate(counts),
    ]
}

impl Report {
    /// The report as text: the counts table (per module, per language and in
    /// total), then the result of each edge-count check.
    pub fn to_text(&self) -> String {
        let mut out = format!("generation {}\n", self.generation);
        let mut rows = vec![
            [
                "language",
                "package",
                "module",
                "call sites",
                "resolved",
                "unresolved",
                "unresolved rate",
            ]
            .map(String::from)
            .to_vec(),
        ];
        for language in &self.languages {
            let name = language.language.as_deref().unwrap_or(UNKNOWN);
            for module in self
                .modules
                .iter()
                .filter(|module| module.language == language.language)
            {
                let package = module.package.as_deref().unwrap_or(UNKNOWN);
                let module_name = module.module.as_deref().unwrap_or(ROOT);
                rows.push(row(name, package, module_name, &module.counts));
            }
            rows.push(row(name, ALL, "", &language.counts));
        }
        rows.push(row("total", "", "", &self.total.0));
        out.push_str(&table(&rows));
        out.push_str(&self.regression_text());

        if let Some(baseline) = &self.baseline {
            let path = baseline.path.display();
            if baseline.differences.is_empty() {
                let _ = writeln!(out, "baseline {path}: matches");
            } else {
                let _ = writeln!(
                    out,
                    "baseline {path}: {} (- baseline, + this generation):",
                    plural(baseline.differences.len(), "difference", "differences")
                );
                for difference in &baseline.differences {
                    for (sign, count) in [("-", difference.expected), ("+", difference.found)] {
                        if let Some(count) = count {
                            let _ = writeln!(
                                out,
                                "{sign} {} {} {count}",
                                difference.provider, difference.path
                            );
                        }
                    }
                }
            }
        }
        out
    }

    /// One resolution line per language, for the end of `codetags index`
    /// (PLAN.md D33): its call sites, how many resolved and did not, and the
    /// unresolved rate. Per-module detail is in [`Self::to_text`].
    pub fn resolution_lines(&self) -> String {
        let mut out = String::new();
        for language in &self.languages {
            let counts = &language.counts;
            let _ = writeln!(
                out,
                "resolution {}: {} call sites, {} resolved, {} unresolved (unresolved rate {})",
                language.language.as_deref().unwrap_or(UNKNOWN),
                counts.call_sites,
                counts.resolved,
                counts.unresolved,
                rate(counts)
            );
        }
        out
    }

    /// The previous-generation check's result: one line, then one line per
    /// file that dropped.
    pub fn regression_text(&self) -> String {
        let mut out = String::new();
        let regression = &self.regression;
        match regression.previous {
            None => out.push_str("edge counts: no previous generation to compare with\n"),
            Some(previous) if regression.drops.is_empty() => {
                let _ = writeln!(
                    out,
                    "edge counts: no regression against generation {previous}"
                );
            }
            Some(previous) => {
                let _ = writeln!(
                    out,
                    "edge counts: {} lost more than {}% of their edges, or all of them, \
                     against generation {previous}; likely a provider failure, not a code change:",
                    plural(regression.drops.len(), "file", "files"),
                    regression.threshold_percent
                );
                for drop in &regression.drops {
                    let now = drop.current.map_or_else(
                        || "none (the file still exists)".to_string(),
                        |n| n.to_string(),
                    );
                    let _ = writeln!(
                        out,
                        "  {} {}: {} -> {now} edges",
                        drop.provider, drop.path, drop.previous
                    );
                }
            }
        }
        out
    }
}
