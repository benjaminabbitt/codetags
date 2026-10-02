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

use codetags_ingest::project::{IndexOptions, index_project};
use codetags_ingest::report::{ReportOptions, report};

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
