//! `codetags index`: index the project into a new generation under
//! `.codetags/index/` (PLAN.md §2.2, §2.3).
//!
//! Prints what each provider's ingest wrote and skipped, and every
//! canonical-name collision. It ends with one resolution line per language
//! and the edge-count regression check against the previous complete
//! generation, the same check `codetags report` runs (PLAN.md D33; brief
//! §4.4, "Validation output"). Per-module detail stays in `codetags report`.
//!
//! Exit status 1 means either nothing was indexed (no supported project, a
//! provider failure, a store error), or the new generation is complete but
//! a file's edges dropped suddenly since the previous one, which is treated
//! as a provider failure, not a code change (brief §4.4). The next run
//! compares against the new generation.

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use codetags_ingest::project::{IndexOptions, index_project};
use codetags_ingest::provider::rust::IMPL_PROVIDER;
use codetags_ingest::report::{ReportOptions, report};

/// Overrides how many seconds the Rust implementation pass waits for
/// rust-analyzer to load the workspace (PLAN.md D31).
pub const LOAD_TIMEOUT_ENV: &str = "CODETAGS_RUST_ANALYZER_LOAD_TIMEOUT";

/// `n` and the noun for it, e.g. "1 call site" or "3 call sites".
fn plural(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

/// The options, with [`LOAD_TIMEOUT_ENV`] applied; an error names a value
/// that is not a whole number of seconds.
fn options() -> Result<IndexOptions, String> {
    let mut options = IndexOptions::default();
    if let Some(value) = std::env::var_os(LOAD_TIMEOUT_ENV) {
        let text = value.to_string_lossy();
        let seconds: u64 = text
            .trim()
            .parse()
            .map_err(|_| format!("{LOAD_TIMEOUT_ENV}={text:?} is not a whole number of seconds"))?;
        options.rust_analyzer_load_timeout = Duration::from_secs(seconds);
    }
    Ok(options)
}

pub fn run(root: &Path) -> ExitCode {
    let options = match options() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("codetags index: {error}");
            return ExitCode::from(1);
        }
    };
    let summary = match index_project(root, &options) {
        Ok(summary) => summary,
        Err(error) => {
            eprintln!("codetags index: {error}");
            return ExitCode::from(1);
        }
    };
    println!(
        "generation {} in {}",
        summary.generation,
        summary.index.display()
    );
    for run in &summary.runs {
        let report = &run.report;
        let skipped = &report.skipped;
        println!(
            "{}: {} files, {} symbols, {} call sites ({})",
            run.version, report.files, report.symbols, report.call_sites, report.source
        );
        println!(
            "  skipped: {} operator references, {} non-callable references, \
             {} references outside a definition, {} local occurrences",
            skipped.operator, skipped.non_callable, skipped.outside_definition, skipped.local
        );
        println!("  canonical-name collisions: {}", report.collisions.len());
        for collision in &report.collisions {
            println!("    {}: {}", collision.name, collision.symbols.join(", "));
        }
        match &run.implementations {
            Some(expansion) => {
                println!(
                    "{IMPL_PROVIDER}: {}, {}, {} on {}",
                    plural(expansion.trait_methods, "called trait method"),
                    plural(expansion.implementations, "implementation"),
                    plural(expansion.targets, "call target"),
                    plural(expansion.sites, "call site"),
                );
                println!(
                    "  rust-analyzer loaded the workspace in {:.1} s and answered in {:.1} s; \
                     answers outside the project: {}; answers matching no definition: {}",
                    expansion.load_time.as_secs_f64(),
                    expansion.answer_time.as_secs_f64(),
                    expansion.outside_root,
                    expansion.unmapped.len()
                );
            }
            None if run.provider == codetags_ingest::project::Provider::Rust => {
                println!("{IMPL_PROVIDER}: no call of a trait method of the project; not run");
            }
            None => {}
        }
    }
    if !summary.gc.removed.is_empty() {
        let removed: Vec<String> = summary.gc.removed.iter().map(u64::to_string).collect();
        println!("removed old generations: {}", removed.join(" "));
    }
    for (path, error) in &summary.gc.deferred {
        println!(
            "could not remove {} yet ({error}); the next run retries",
            path.display()
        );
    }

    let options = ReportOptions {
        generation: Some(summary.generation),
        baseline: None,
    };
    let checked = match report(root, &summary.index, &options) {
        Ok(checked) => checked,
        Err(error) => {
            eprintln!(
                "codetags index: generation {} is complete, but its report failed: {error}",
                summary.generation
            );
            return ExitCode::from(1);
        }
    };
    print!("{}", checked.resolution_lines());
    print!("{}", checked.regression_text());
    if checked.passed() {
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "codetags index: an edge-count check failed (see above); generation {} is \
             complete, and the next run compares against it",
            summary.generation
        );
        ExitCode::from(1)
    }
}
