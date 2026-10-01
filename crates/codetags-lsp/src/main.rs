//! `codetags-lsp`: record a language-server session, or summarize a
//! recording (M3 stage 0, PLAN.md D18).

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{ExitCode, ExitStatus};

use clap::{Parser, Subcommand};

/// Exit status for the recorder's own failures (bad arguments aside, which
/// clap reports with 2).
const FAILURE: u8 = 1;

#[derive(Parser)]
#[command(name = "codetags-lsp", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a language server, pass its stdio through unchanged, and log
    /// every message as one JSON line. Exits with the server's status.
    /// Non-LSP invocations (`--version`, subcommands) run the server
    /// directly and log nothing.
    Record {
        /// The real language server binary.
        #[arg(long)]
        server: PathBuf,
        /// The log file, appended to. `{ts}` becomes the UTC start time and
        /// `{pid}` the recorder's process id. Its directory is created.
        #[arg(long)]
        log: PathBuf,
        /// A subcommand that still starts an LSP session (e.g. gopls's
        /// `serve`). Repeatable.
        #[arg(long = "lsp-subcommand", value_name = "NAME")]
        lsp_subcommands: Vec<String>,
        /// Arguments for the server, after `--`.
        #[arg(last = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Summarize a recording: initialize params, document sync, server
    /// requests and the client's answers, and request latencies.
    Analyze {
        /// A log written by `record`.
        log: PathBuf,
    },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Record {
            server,
            log,
            lsp_subcommands,
            args,
        } => {
            let options = codetags_lsp::record::Options {
                server,
                args,
                log,
                lsp_subcommands,
            };
            match codetags_lsp::record::run(&options) {
                Ok(status) => exit_like(status),
                Err(error) => {
                    eprintln!("codetags-lsp: {error}");
                    ExitCode::from(FAILURE)
                }
            }
        }
        Command::Analyze { log } => {
            let text = match std::fs::read_to_string(&log) {
                Ok(text) => text,
                Err(error) => {
                    eprintln!("codetags-lsp: cannot read {}: {error}", log.display());
                    return ExitCode::from(FAILURE);
                }
            };
            match codetags_lsp::analyze::analyze(&text) {
                Ok(report) => {
                    print!("{report}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("codetags-lsp: {}: {error}", log.display());
                    ExitCode::from(FAILURE)
                }
            }
        }
    }
}

/// Ends this process the way the server's ended: with its exit code, or on
/// Unix by the same signal (falling back to `128 + signal`).
fn exit_like(status: ExitStatus) -> ExitCode {
    if let Some(code) = status.code() {
        // Exit codes outside 0..=255 exist only on Windows; std::process::exit
        // passes them on unchanged.
        if let Ok(code) = u8::try_from(code) {
            return ExitCode::from(code);
        }
        std::process::exit(code);
    }
    #[cfg(unix)]
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
        // Every signal keeps its default action in a Rust program except
        // SIGPIPE, which std ignores; raising it then returns, and the
        // fallback below reports it.
        if let Ok(signal) = nix::sys::signal::Signal::try_from(signal) {
            let _ = nix::sys::signal::raise(signal);
        }
        return ExitCode::from(128u8.saturating_add(u8::try_from(signal).unwrap_or(127)));
    }
    ExitCode::from(FAILURE)
}
