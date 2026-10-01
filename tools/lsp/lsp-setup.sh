#!/bin/sh
# The body of `just lsp-setup` (M3 stages 1 and 2, PLAN.md D14-D22;
# docs/lsp-shim.md §1), which builds the binaries and then runs this. Usage:
#
#     lsp-setup.sh CODETAGS_LSP CODETAGS [codetags lsp setup options...]
#
# CODETAGS_LSP and CODETAGS are the built `codetags-lsp` and `codetags`
# binaries. It wires the checkout it lives in, whatever the current
# directory: this checkout's Claude Code plugin
# (tools/claude-plugins/codetags-lsp) and VS Code (.vscode/settings.json), to
# one rust-analyzer through the codetags shim and lspmux. It copies
# CODETAGS_LSP to .codetags/local/bin (by rename, so running shims keep
# theirs) and writes this toolchain's rust-analyzer path to
# .codetags/local/rust-analyzer.path, which the sh wrappers read. On Windows,
# where clients start only executables (V118, V119), it installs wrapper
# executables beside the sh wrappers instead. Last, `codetags lsp setup`, with
# the options given here (e.g. --yes), shows its change to lspmux's config and
# asks before writing (D20). Needs `just setup-lspmux` first.
#
# The wiring scenarios (features/lsp/wiring.feature) run it in a scratch
# checkout, so a person's setup and the tested one are the same.

if [ $# -lt 2 ]; then
    echo "usage: lsp-setup.sh CODETAGS_LSP CODETAGS [codetags lsp setup options...]" >&2
    exit 2
fi
case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) windows=yes exe=.exe ;;
    *) windows="" exe="" ;;
esac

# An absolute POSIX path (Git's sh on Windows: cygpath), made before this
# script leaves the caller's directory.
absolute() {
    if command -v cygpath >/dev/null 2>&1; then
        cygpath -ua "$1"
    else
        case "$1" in
            /*) printf '%s\n' "$1" ;;
            *) printf '%s\n' "$PWD/$1" ;;
        esac
    fi
}
lsp=$(absolute "$1") && codetags=$(absolute "$2") || exit 1
shift 2
cd "$(dirname "$0")/../.." || exit 1

[ -x .codetags/local/bin/lspmux ] || { echo "lsp-setup: lspmux is not installed; run: just setup-lspmux"; exit 1; }
mkdir -p .codetags/local/bin || exit 1
cp "$lsp" ".codetags/local/bin/codetags-lsp$exe.new" && mv -f ".codetags/local/bin/codetags-lsp$exe.new" ".codetags/local/bin/codetags-lsp$exe" || exit 1
ra="$(rustup which rust-analyzer)" || { echo "lsp-setup: no rust-analyzer for this toolchain; add it with: rustup component add rust-analyzer"; exit 1; }
printf '%s\n' "$ra" > .codetags/local/rust-analyzer.path || exit 1
echo "lsp-setup: shim .codetags/local/bin/codetags-lsp, real server $ra ($("$ra" --version))"
if [ -n "$windows" ]; then
    lspmux="$(cygpath -w "$PWD/.codetags/local/bin/lspmux.exe")"
    .codetags/local/bin/codetags-lsp.exe install-wrapper --role agent --server "$ra" --lspmux "$lspmux" tools/claude-plugins/codetags-lsp/scripts/rust-analyzer &&
        .codetags/local/bin/codetags-lsp.exe install-wrapper --role editor --server "$ra" --lspmux "$lspmux" tools/vscode/rust-analyzer || exit 1
fi
# codetags links the prebuilt libduckdb, which the build copies into deps
# beside it: Linux and macOS find it by the binary's run path, Windows on PATH.
PATH="$(dirname "$codetags")/deps:$PATH" "$codetags" lsp setup "$@" || exit
echo ""
echo "Next (docs/lsp-shim.md has the details):"
echo "  1. Claude Code: restart it in this checkout; /plugin lists codetags-lsp@codetags-local, enabled."
echo "  2. VS Code: reload the window; .vscode/settings.json points rust-analyzer.server.path at tools/vscode/rust-analyzer."
echo "  3. Check: codetags doctor (just doctor), and .codetags/local/bin/lspmux status"
