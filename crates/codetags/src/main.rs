//! codetags CLI and codetagsd.

mod doctor;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Views of a software project as files, backed by a derived code index.
#[derive(Debug, Parser)]
#[command(name = "codetags", version, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Report what this machine can do (mount backend, helpers) and what to fix.
    Doctor,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Doctor => doctor::run(),
    }
}
