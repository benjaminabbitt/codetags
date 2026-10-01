//! `codetags lsp setup`: writes lspmux's config file (PLAN.md D20). It is
//! the only code that does, and only when run: it prints the difference,
//! asks, keeps a backup, then writes.

use std::io::{BufRead, Write};
use std::process::ExitCode;

use codetags_lsp::lspmux::{self, Address};
use codetags_lsp::setup::Plan;

/// `--listen`: loopback TCP or an absolute socket path, nothing other
/// machines could reach.
pub fn parse_listen(text: &str) -> Result<Address, String> {
    let address = Address::parse(text)?;
    lspmux::validate_listen(&address)?;
    Ok(address)
}

/// Runs `codetags lsp setup`. Exit status 0 when the file is written or
/// already current, 1 when not confirmed or on failure.
pub fn setup(listen: Option<Address>, yes: bool) -> ExitCode {
    let plan = match Plan::new(listen, lspmux::DEFAULT_INSTANCE_TIMEOUT) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("codetags lsp setup: {error}");
            return ExitCode::from(1);
        }
    };
    println!("lspmux config: {}", plan.path.display());
    println!("listen: {}", plan.listen);
    if let Some(note) = plan.listen.exposure_note() {
        println!("note: {note}");
    }
    if plan.is_current() {
        println!("up to date: nothing to write");
        return ExitCode::SUCCESS;
    }
    print!("{}", plan.diff());
    if !yes && !confirm(&plan) {
        println!("not written (confirm with y, or pass --yes)");
        return ExitCode::from(1);
    }
    match plan.apply() {
        Ok(backup) => {
            if let Some(backup) = backup {
                println!("backed up the old file to {}", backup.display());
            }
            println!("wrote {}", plan.path.display());
            println!(
                "a running lspmux server keeps its old settings until it restarts \
                 (codetags-lsp starts one when none answers)"
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("codetags lsp setup: {error}");
            ExitCode::from(1)
        }
    }
}

/// Asks on stdin; only `y` or `yes` confirms. End of input declines.
fn confirm(plan: &Plan) -> bool {
    print!("Write {}? [y/N] ", plan.path.display());
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    println!();
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}
