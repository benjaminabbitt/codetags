//! Whether a server invocation is an LSP session (brief §4.1).
//!
//! Editors also run the server binary for other things: VS Code's
//! rust-analyzer extension runs `<server> --version` and fails if that
//! fails. Such invocations go straight to the real binary.

use std::ffi::OsString;

/// Flags that make any invocation a non-LSP one.
const NON_LSP_FLAGS: [&str; 4] = ["--version", "-V", "--help", "-h"];

/// Whether running the server with `args` starts an LSP session.
///
/// It does not when any argument is one of `--version`, `-V`, `--help` or
/// `-h`, or when the first argument that does not start with `-` (a
/// subcommand, such as `version` or rust-analyzer's `analysis-stats`) is not
/// in `lsp_subcommands` (such as gopls's `serve`). A flag that takes a
/// separate value therefore looks like a subcommand; listing the value in
/// `lsp_subcommands` overrides that. Getting it wrong is safe: a
/// non-LSP invocation is passed straight through, unrecorded.
pub fn is_lsp_session(args: &[OsString], lsp_subcommands: &[String]) -> bool {
    let args: Vec<String> = args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    if args.iter().any(|arg| NON_LSP_FLAGS.contains(&arg.as_str())) {
        return false;
    }
    match args.iter().find(|arg| !arg.starts_with('-')) {
        Some(subcommand) => lsp_subcommands.contains(subcommand),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::is_lsp_session;

    fn lsp(args: &[&str], subcommands: &[&str]) -> bool {
        let args: Vec<_> = args.iter().map(Into::into).collect();
        let subcommands: Vec<String> = subcommands.iter().map(|s| s.to_string()).collect();
        is_lsp_session(&args, &subcommands)
    }

    #[test]
    fn no_arguments_is_a_session() {
        assert!(lsp(&[], &[]));
    }

    #[test]
    fn flags_alone_are_a_session() {
        assert!(lsp(&["--stdio"], &[]));
    }

    #[test]
    fn version_and_help_are_not() {
        for flag in ["--version", "-V", "--help", "-h"] {
            assert!(!lsp(&[flag], &[]), "{flag}");
        }
        assert!(!lsp(&["serve", "--version"], &["serve"]));
    }

    #[test]
    fn subcommands_are_not_unless_listed() {
        assert!(!lsp(&["version"], &[]));
        assert!(!lsp(&["analysis-stats", "."], &[]));
        assert!(lsp(&["serve"], &["serve"]));
    }
}
