//! codetags CLI and codetagsd.

use clap::Parser;

/// Views of a software project as files, backed by a derived code index.
#[derive(Debug, Parser)]
#[command(name = "codetags", version, arg_required_else_help = true)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
