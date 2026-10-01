//! codetags CLI and codetagsd.

mod doctor;
mod index;

use std::path::PathBuf;
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
    /// Index the project into a new generation under `.codetags/index/`.
    Index {
        /// The project root.
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Doctor => doctor::run(),
        Command::Index { root } => index::run(&root),
    }
}
