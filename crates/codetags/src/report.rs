//! `codetags report`: the resolution report of a generation, and its
//! edge-count regression checks (brief §4.4; PLAN.md §9 P1.8).
//!
//! Exit status 0 means every check passed; 1 means a check failed (a drop
//! since the previous generation, or a difference from `--baseline`) or the
//! report could not be made (no generation, an unreadable baseline).

use std::path::PathBuf;
use std::process::ExitCode;

use codetags_ingest::project::{DATA_DIR, INDEX_DIR};
use codetags_ingest::report::{ReportOptions, baseline_json, report};

/// What `codetags report` was asked for.
#[derive(Debug)]
pub struct Args {
    /// The project root.
    pub root: PathBuf,
    /// The generation; by default the newest complete one.
    pub generation: Option<u64>,
    /// Print JSON instead of text.
    pub json: bool,
    /// A baseline to compare the edge counts with.
    pub baseline: Option<PathBuf>,
    /// Where to write the generation's edge counts as a baseline.
    pub write_baseline: Option<PathBuf>,
}

pub fn run(args: Args) -> ExitCode {
    let index = args.root.join(DATA_DIR).join(INDEX_DIR);
    let options = ReportOptions {
        generation: args.generation,
        baseline: args.baseline,
    };
    let report = match report(&args.root, &index, &options) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("codetags report: {error}");
            return ExitCode::from(1);
        }
    };
    if args.json {
        println!("{}", report.to_json());
    } else {
        print!("{}", report.to_text());
    }
    if let Some(path) = &args.write_baseline {
        if let Err(error) = std::fs::write(path, baseline_json(&report.edge_counts)) {
            eprintln!("codetags report: writing {}: {error}", path.display());
            return ExitCode::from(1);
        }
        eprintln!(
            "codetags report: wrote the edge counts to {}",
            path.display()
        );
    }
    if report.passed() {
        ExitCode::SUCCESS
    } else {
        eprintln!("codetags report: an edge-count check failed; see above");
        ExitCode::from(1)
    }
}
