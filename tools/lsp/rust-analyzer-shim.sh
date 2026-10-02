#!/bin/sh
# rust-analyzer through the codetags shim and lspmux (M3 stages 1 and 2,
# PLAN.md D14-D22; docs/lsp-shim.md), for Linux and macOS. Usage:
#
#     rust-analyzer-shim.sh editor|agent [rust-analyzer arguments...]
#
# The per-client wrappers run it: tools/vscode/rust-analyzer (VS Code, role
# editor) and the Claude Code plugin's scripts/rust-analyzer (role agent).
# On Windows the clients start a codetags-lsp wrapper executable instead
# (`just lsp-setup` installs it; V119).
#
# Setup lives in a `.codetags/local` directory: bin/codetags-lsp (the shim)
# and rust-analyzer.path (the real rust-analyzer's absolute path), which
# `just lsp-setup` writes, and bin/lspmux, which `just setup-lspmux`
# installs. A Claude Code project's own setup ($CLAUDE_PROJECT_DIR) wins over
# this checkout's. lspmux's config is `codetags lsp setup`'s.
#
# Optional:
# - CODETAGS_LSP_LOG_DIR: log each session there, as `codetags-lsp record`
#   does: the client's side (lsp-<time>-<pid>.jsonl, from the shim) and the
#   server's (server-<time>-<pid>.jsonl, from a recorder between lspmux and
#   rust-analyzer). The live test (features/lsp/claude-client.feature) sets it.
# - CODETAGS_LSPMUX_XDG_CONFIG_HOME: XDG_CONFIG_HOME for the shim and lspmux,
#   so they use a separate lspmux config (Linux only; the live test sets it).
#
# Without setup, this runs rust-analyzer from PATH directly, as the official
# rust-analyzer-lsp plugin does, so the client still has a language server.
# With a shim that cannot serve (`codetags-lsp serve --help` fails, as in a
# stage-0 build), or no lspmux, it runs the recorded rust-analyzer directly.
# Each fallback says why on stderr; `codetags doctor` reports the same
# ("lsp wiring").

role="${1:?usage: rust-analyzer-shim.sh editor|agent [args...]}"
shift
repo=$(cd "$(dirname "$0")/../.." && pwd) || exit 1
for local_dir in ${CLAUDE_PROJECT_DIR:+"$CLAUDE_PROJECT_DIR/.codetags/local"} "$repo/.codetags/local"; do
    shim="$local_dir/bin/codetags-lsp"
    if ! [ -x "$shim" ] || ! [ -s "$local_dir/rust-analyzer.path" ]; then
        continue
    fi
    server=$(cat "$local_dir/rust-analyzer.path")
    # A stage-0 build (`just lsp-record-setup`) has no `serve` (D25).
    if ! "$shim" serve --help >/dev/null 2>&1; then
        echo "rust-analyzer ($role): $shim cannot serve (run: just lsp-setup); running rust-analyzer without the shim" >&2
        exec "$server" "$@"
    fi
    lspmux=""
    for candidate in "$local_dir/bin/lspmux" "$repo/.codetags/local/bin/lspmux"; do
        if [ -x "$candidate" ]; then
            lspmux="$candidate"
            break
        fi
    done
    if [ -z "$lspmux" ]; then
        lspmux=$(command -v lspmux) || {
            echo "rust-analyzer ($role): lspmux not found (run: just setup-lspmux); running rust-analyzer without the shim" >&2
            exec "$server" "$@"
        }
    fi
    if [ -n "${CODETAGS_LSPMUX_XDG_CONFIG_HOME:-}" ]; then
        XDG_CONFIG_HOME="$CODETAGS_LSPMUX_XDG_CONFIG_HOME"
        export XDG_CONFIG_HOME
    fi
    if [ -n "${CODETAGS_LSP_LOG_DIR:-}" ]; then
        exec "$shim" serve --role "$role" --lspmux "$lspmux" --root cargo --toolchain rust \
            --log "$CODETAGS_LSP_LOG_DIR/lsp-{ts}-{pid}.jsonl" \
            --server "$shim" --lsp-subcommand record \
            -- record --server "$server" --log "$CODETAGS_LSP_LOG_DIR/server-{ts}-{pid}.jsonl" -- "$@"
    fi
    exec "$shim" serve --role "$role" --lspmux "$lspmux" --server "$server" -- "$@"
done
echo "rust-analyzer ($role): codetags-lsp is not set up (run: just lsp-setup); running rust-analyzer without the shim" >&2
exec rust-analyzer "$@"
