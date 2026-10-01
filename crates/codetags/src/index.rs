//! `codetags index`: index the project into a new generation under
//! `.codetags/index/` (PLAN.md §2.2, §2.3).
//!
//! Prints what each provider's ingest wrote and skipped, and every
//! canonical-name collision. Exit status 1 means nothing was indexed: no
//! supported project, a provider failure, or a store error.

use std::path::Path;
use std::process::ExitCode;

use codetags_ingest::project::{IndexOptions, index_project};

pub fn run(root: &Path) -> ExitCode {
    let summary = match index_project(root, &IndexOptions::default()) {
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
    ExitCode::SUCCESS
}
