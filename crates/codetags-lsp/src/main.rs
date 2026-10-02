//! `codetags-lsp`: the LSP shim between clients and lspmux (`serve`), its
//! wrappers, and the stage-0 recorder: record a language-server session, or summarize a
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
        /// Step marks, one JSON object per line: `{"ts_ms": <Unix ms>,
        /// "step": "<name>"}`. Adds what the client sent during each step.
        #[arg(long)]
        marks: Option<PathBuf>,
    },
    /// Stand in for a language server and share it through lspmux: start
    /// `lspmux server` if nothing answers, then relay the session through
    /// `lspmux client`. Non-LSP invocations (`--version`, subcommands) run
    /// the server directly.
    Serve {
        /// Who the client is: `editor` sends document contents; `agent`
        /// never does, and the server reads files from disk.
        #[arg(long)]
        role: codetags_lsp::policy::Role,
        /// The real language server binary (absolute, or found on PATH).
        #[arg(long)]
        server: PathBuf,
        /// The lspmux binary; by default $CODETAGS_LSPMUX,
        /// .codetags/local/bin/lspmux, or lspmux on PATH.
        #[arg(long)]
        lspmux: Option<PathBuf>,
        /// Log the session as `record` does (dropped and locally answered
        /// messages are marked). `{ts}` and `{pid}` are expanded.
        #[arg(long)]
        log: Option<PathBuf>,
        /// A subcommand that still starts an LSP session. Repeatable.
        #[arg(long = "lsp-subcommand", value_name = "NAME")]
        lsp_subcommands: Vec<String>,
        /// How to find the project root: `cargo` (the default for
        /// rust-analyzer) or `client` (the root the client sent).
        #[arg(long, value_parser = parse_root_rule)]
        root: Option<codetags_lsp::root::RootRule>,
        /// How long an agent's requests may wait for the server to finish
        /// loading, in seconds (0: never wait); by default
        /// $CODETAGS_LSP_READY_TIMEOUT, else 300.
        #[arg(long, value_name = "SECONDS")]
        ready_timeout: Option<u64>,
        /// How to find the session's toolchain, passed to the server:
        /// `rust` (the default for rust-analyzer) or `none`.
        #[arg(long, value_parser = parse_toolchain_rule)]
        toolchain: Option<codetags_lsp::toolchain::ToolchainRule>,
        /// Arguments for the server, after `--`.
        #[arg(last = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Install a wrapper: a copy of this binary at DEST that runs `serve`
    /// with these settings whatever its name, for clients that can start
    /// only an executable (Windows).
    InstallWrapper {
        /// The session role.
        #[arg(long)]
        role: codetags_lsp::policy::Role,
        /// The real language server binary.
        #[arg(long)]
        server: PathBuf,
        /// The lspmux binary, if not the default.
        #[arg(long)]
        lspmux: Option<PathBuf>,
        /// A session log template.
        #[arg(long)]
        log: Option<PathBuf>,
        /// Where to put the wrapper (`.exe` is added on Windows).
        dest: PathBuf,
    },
}

fn parse_root_rule(text: &str) -> Result<codetags_lsp::root::RootRule, String> {
    match text {
        "cargo" => Ok(codetags_lsp::root::RootRule::Cargo),
        "client" => Ok(codetags_lsp::root::RootRule::Client),
        other => Err(format!(
            "unknown root rule {other:?}: expected cargo or client"
        )),
    }
}

fn parse_toolchain_rule(text: &str) -> Result<codetags_lsp::toolchain::ToolchainRule, String> {
    match text {
        "rust" => Ok(codetags_lsp::toolchain::ToolchainRule::Rust),
        "none" => Ok(codetags_lsp::toolchain::ToolchainRule::None),
        other => Err(format!(
            "unknown toolchain rule {other:?}: expected rust or none"
        )),
    }
}

/// Ends the process the way a `serve` run ended.
fn serve_exit(result: Result<codetags_lsp::serve::Outcome, String>) -> ExitCode {
    match result {
        Ok(codetags_lsp::serve::Outcome::Status(status)) => exit_like(status),
        Ok(codetags_lsp::serve::Outcome::Code(code)) => ExitCode::from(code),
        Err(error) => {
            eprintln!("codetags-lsp: {error}");
            ExitCode::from(FAILURE)
        }
    }
}

/// Reads `path` as text, reporting a failure on stderr.
fn read_text(path: &std::path::Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(error) => {
            eprintln!("codetags-lsp: cannot read {}: {error}", path.display());
            None
        }
    }
}

fn main() -> ExitCode {
    // Started under another name: a wrapper (codetags_lsp::wrapper).
    if let Some(exe) = codetags_lsp::wrapper::invoked_as_wrapper() {
        let args = std::env::args_os().skip(1).collect();
        return serve_exit(codetags_lsp::wrapper::run(&exe, args));
    }
    match Cli::parse().command {
        Command::Serve {
            role,
            server,
            lspmux,
            log,
            lsp_subcommands,
            root,
            ready_timeout,
            toolchain,
            args,
        } => serve_exit(codetags_lsp::serve::run(&codetags_lsp::serve::Options {
            toolchain_rule: toolchain,
            role,
            server,
            args,
            lspmux,
            log,
            lsp_subcommands,
            root_rule: root,
            ready_timeout,
        })),
        Command::InstallWrapper {
            role,
            server,
            lspmux,
            log,
            dest,
        } => {
            let settings = codetags_lsp::wrapper::Settings {
                role,
                server,
                lspmux,
                log,
            };
            match codetags_lsp::wrapper::install(&dest, &settings) {
                Ok(exe) => {
                    println!("codetags-lsp: installed the wrapper {}", exe.display());
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("codetags-lsp: {error}");
                    ExitCode::from(FAILURE)
                }
            }
        }
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
        Command::Analyze { log, marks } => {
            let Some(text) = read_text(&log) else {
                return ExitCode::from(FAILURE);
            };
            let marks = match marks.as_deref().map(read_text) {
                None => None,
                Some(Some(marks)) => Some(marks),
                Some(None) => return ExitCode::from(FAILURE),
            };
            match codetags_lsp::analyze::analyze(&text, marks.as_deref()) {
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
