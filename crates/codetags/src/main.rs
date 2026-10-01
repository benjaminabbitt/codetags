//! codetags CLI and codetagsd.

mod doctor;
mod index;
mod lsp;
mod report;

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
    /// Report a generation's resolved and unresolved call sites per language
    /// and module, and check its edge counts against the previous generation
    /// and an optional baseline. Exits 1 if a check fails.
    Report {
        /// The project root.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// The generation to report; by default the newest complete one.
        #[arg(long)]
        generation: Option<u64>,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
        /// A JSON file of per-file edge counts per provider to compare with;
        /// any difference fails.
        #[arg(long, value_name = "FILE")]
        baseline: Option<PathBuf>,
        /// Write the generation's per-file edge counts to FILE, as a baseline.
        #[arg(long, value_name = "FILE")]
        write_baseline: Option<PathBuf>,
    },
    /// The LSP proxy: language servers shared through lspmux (M3).
    Lsp {
        #[command(subcommand)]
        command: LspCommand,
    },
}

#[derive(Debug, Subcommand)]
enum LspCommand {
    /// Write lspmux's config file: a listen address only you can reach (a
    /// socket in a private directory; loopback TCP on Windows), the
    /// environment allowlist that keys a server, and the idle timeout. Shows
    /// the difference and asks first; backs up a different file.
    Setup {
        /// Write without asking.
        #[arg(long)]
        yes: bool,
        /// Listen on this address instead: IP:PORT (loopback only) or an
        /// absolute socket path.
        #[arg(long, value_parser = lsp::parse_listen)]
        listen: Option<codetags_lsp::lspmux::Address>,
    },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Doctor => doctor::run(),
        Command::Index { root } => index::run(&root),
        Command::Report {
            root,
            generation,
            json,
            baseline,
            write_baseline,
        } => report::run(report::Args {
            root,
            generation,
            json,
            baseline,
            write_baseline,
        }),
        Command::Lsp {
            command: LspCommand::Setup { yes, listen },
        } => lsp::setup(listen, yes),
    }
}
