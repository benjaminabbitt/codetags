# codetags task runner: the only entry point for agents and CI (PLAN.md §0.5).
# Recipe names are a frozen contract (PLAN.md Appendix A). A recipe whose phase
# is not built yet FAILS with the phase name, so a stub never passes for a real
# check (PLAN.md §0.11).
#
# Recipes run under `sh` on every OS (Git's sh on Windows), so keep them POSIX.

default: check

# The executable suffix of this OS.
exe := if os() == "windows" { ".exe" } else { "" }

# --- setup ---------------------------------------------------------------

# Toolchain from rust-toolchain.toml, plus the cargo tools `check` needs.
setup:
    rustup toolchain install
    @command -v cargo-deny >/dev/null 2>&1 || echo "setup: cargo-deny missing; install with: cargo install cargo-deny --locked"
    @echo "setup: ready"

# Pinned SCIP providers from providers.toml (P1).
# Idempotent. Rust: the rust-analyzer component of the toolchain that
# providers.toml pins (the one in rust-toolchain.toml). Go: needs the pinned
# Go minor version on PATH (CI: actions/setup-go), then `go install`s scip-go
# and tools/gocallgraph into `go env GOPATH`/bin, which must be on PATH.
# TypeScript: scip-typescript and Jelly, `npm install -g` into npm's global
# prefix, which must be per-user (nvm, actions/setup-node, or `npm config set
# prefix ~/.local`): providers never install or run as root (PLAN.md §0.10).
# Needs Node 22 or later on PATH; providers.toml pins the version CI uses.
# Python: scip-python, `npm install -g` the same way; it needs python3 (or
# python) and pip on PATH (CI: actions/setup-python). The installed bundle is
# patched so it starts on Windows (tools/patch-scip-python.js, V104).
setup-providers:
    rustup toolchain install
    rustup component add rust-analyzer --toolchain "$(sed -n '/^\[rust-analyzer\]/,/^\[/s/^toolchain *= *"\(.*\)".*/\1/p' providers.toml)"
    rust-analyzer --version
    @want="$(sed -n '/^\[go\]/,/^\[/s/^version *= *"\([0-9]*\.[0-9]*\).*/\1/p' providers.toml)"; if go version | grep -q "go$want[. ]"; then go version; else echo "setup-providers: providers.toml pins Go $want; found: $(go version 2>&1)"; exit 1; fi
    go install "$(sed -n '/^\[scip-go\]/,/^\[/s/^module *= *"\(.*\)".*/\1/p' providers.toml)/cmd/scip-go@$(sed -n '/^\[scip-go\]/,/^\[/s/^version *= *"\(.*\)".*/\1/p' providers.toml)"
    go install -C tools/gocallgraph .
    @command -v scip-go >/dev/null && command -v gocallgraph >/dev/null || { echo "setup-providers: add $(go env GOPATH)/bin to PATH"; exit 1; }
    scip-go --version
    @node --version || { echo "setup-providers: node is not on PATH; install Node $(sed -n '/^\[node\]/,/^\[/s/^version *= *"\(.*\)".*/\1/p' providers.toml) per-user (e.g. nvm)"; exit 1; }
    npm install -g --no-fund --no-audit "$(sed -n '/^\[scip-typescript\]/,/^\[/s/^npm *= *"\(.*\)".*/\1/p' providers.toml)" "$(sed -n '/^\[jelly\]/,/^\[/s/^npm *= *"\(.*\)".*/\1/p' providers.toml)"
    scip-typescript --version
    jelly --version
    @if command -v python3 >/dev/null; then python3 --version; elif command -v python >/dev/null; then python --version; else echo "setup-providers: scip-python needs python3 or python on PATH (providers.toml [python])"; exit 1; fi
    @command -v pip3 >/dev/null || command -v pip >/dev/null || { echo "setup-providers: scip-python needs pip3 or pip on PATH"; exit 1; }
    npm install -g --no-fund --no-audit "$(sed -n '/^\[scip-python\]/,/^\[/s/^npm *= *"\(.*\)".*/\1/p' providers.toml)"
    node tools/patch-scip-python.js "$(npm root -g)"
    @want="$(sed -n '/^\[scip-python\]/,/^\[/s/^npm *= *".*@\(.*\)".*/\1/p' providers.toml)"; got="$(scip-python --version 2>&1)"; if [ "$got" = "$want" ]; then echo "scip-python $got"; else echo "setup-providers: providers.toml pins scip-python $want; found: $got"; exit 1; fi
    @echo "setup-providers: ready"

# The pinned lspmux (PLAN.md D19; providers.toml [lspmux]) that the LSP shim
# runs (M3), built from source into .codetags/local/bin, where codetags-lsp,
# the wrappers and the tests find it. Idempotent: no build, and no network,
# when .codetags/local/.crates.toml already records the pinned revision.
# Kept out of `setup`: it builds for about a minute and fetches from
# codeberg.org, which only the jobs that test the proxy (`@lspmux`) need.
setup-lspmux:
    @rev="$(sed -n '/^\[lspmux\]/,/^\[/s/^rev *= *"\(.*\)".*/\1/p' providers.toml)"; git="$(sed -n '/^\[lspmux\]/,/^\[/s/^git *= *"\(.*\)".*/\1/p' providers.toml)"; \
    if grep -q "#$rev" .codetags/local/.crates.toml 2>/dev/null && [ -x .codetags/local/bin/lspmux ]; then echo "setup-lspmux: lspmux $rev is already installed"; \
    else cargo install --locked --git "$git" --rev "$rev" --root .codetags/local lspmux; fi
    .codetags/local/bin/lspmux --version

# --- the green light -----------------------------------------------------

# Universal check: must exit 0 on Linux, macOS, and Windows (PLAN.md §0.6).
check: override-check fmt-check lint duckdb-verify license-check test bdd tags-check
    @echo "check: green"

# Fails while .cargo/config.toml holds a local override (`just dev-tagma PATH`
# or a hand-written [patch]), so one cannot be committed.
override-check:
    @if grep -q -e '^# >>> dev-tagma' -e '^\[patch' .cargo/config.toml; then echo "override-check: .cargo/config.toml holds a local override; remove it with: just dev-tagma"; exit 1; else echo "override-check: no local overrides"; fi

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Checks the prebuilt libduckdb archive that the build downloaded (V29) against
# its pinned SHA-256 in duckdb.sha256 (PLAN.md R8). Runs after `lint` has built
# libduckdb-sys and before `test` loads the library. With TARGET, checks the
# cache of a `--target TARGET` build.
duckdb-verify TARGET="":
    sh ci/duckdb-verify.sh {{TARGET}}

# Clippy for the Windows and macOS targets, from any host, to catch cfg-gated
# code before CI does. Not part of `check`: CI lints natively on those OSes.
lint-cross:
    @for target in x86_64-pc-windows-msvc aarch64-apple-darwin; do rustup target list --installed | grep -qx "$target" || { echo "lint-cross: target $target is not installed; add it with: rustup target add $target"; exit 1; }; done
    # winfsp-sys runs bindgen over WinFsp's headers, which need windows.h; its
    # docsrs feature uses the bindings it ships instead (V49).
    cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc --features winfsp-sys/docsrs -- -D warnings
    cargo clippy --workspace --all-targets --target aarch64-apple-darwin -- -D warnings

# Licence boundaries (PLAN.md §0.12, D9); policy in deny.toml.
license-check:
    cargo deny check licenses bans

test:
    cargo test --workspace

# Gherkin features (PLAN.md §0.2) that need no mount, provider, or privilege.
# Jobs with more capabilities set CODETAGS_BDD_CAPABILITIES (codetags-bdd).
# Every cargo invocation here selects --workspace: a narrower selection unifies
# features differently and rebuilds dependencies. Builds every workspace
# binary first: features/lsp runs codetags-lsp, which the bdd test (in the
# codetags package) does not build.
bdd:
    cargo build --workspace --bins --quiet
    cargo test --workspace --test bdd

# Validates this repo's .codetags/tags (P6.2). Vacuously green while no tags
# file exists; fails once one exists until the validator is built.
tags-check:
    @if [ -e .codetags/tags ]; then echo "tags-check: .codetags/tags exists but the validator is not built yet (P6.2)"; exit 1; else echo "tags-check: no .codetags/tags"; fi

# --- capability-gated suites ---------------------------------------------

# Mount features against this OS's live backend (P0b, P5): the bdd suite with
# the `mount` capability enabled, plus `winfsp` on Windows, whose backend needs
# WinFsp installed. Needs /dev/fuse and a setuid fusermount3 on Linux;
# `codetags doctor` explains what is missing. On Windows it also runs S4's
# name probe (crates/codetags-mount-winfsp/tests/name_probe.rs).
test-mount:
    cargo build --workspace --bins --quiet
    CODETAGS_BDD_CAPABILITIES={{ if os() == "windows" { "mount,winfsp" } else { "mount" } }} cargo test --workspace --test bdd
    if [ "{{ os() }}" = windows ]; then cargo test --workspace --test name_probe -- --ignored --nocapture; fi

# Index features against the fixtures (P1): the bdd suite with the
# `providers` capability enabled. Run `setup-providers` first. Builds every
# workspace binary first, as `bdd` does.
test-providers:
    cargo build --workspace --bins --quiet
    CODETAGS_BDD_CAPABILITIES=providers cargo test --workspace --test bdd

# The provider fixtures whose edge counts are baselined in tests/baselines:
# those `codetags index` ingests. Add go, ts and python once ingest supports
# them (P1.8).
baseline_fixtures := "rust"

# Edge-count regression against tests/baselines (P1.8): indexes a copy of
# each fixture in baseline_fixtures and fails, printing a diff, if any
# per-file edge count differs from its baseline. Run `setup-providers` first.
baseline-check:
    cargo build --workspace --bins --quiet
    sh ci/baseline.sh check {{baseline_fixtures}}

# Rewrites tests/baselines from fresh fixture indexes. Deliberate only: after
# a provider upgrade, review the diff `baseline-check` printed first.
baseline-update:
    cargo build --workspace --bins --quiet
    sh ci/baseline.sh update {{baseline_fixtures}}

# Privileged-helper features (P4.4): the bdd suite with the `privileged`
# capability enabled. CI only (the privileged-linux job): the scenarios start
# codetags-privhelper with `sudo -n` themselves, so sudo must need no
# password, while the scenarios and their watchers run as the calling,
# unprivileged user. Builds the helper binary first: the bdd test does not.
test-privileged:
    cargo build --workspace --bins
    CODETAGS_BDD_CAPABILITIES=privileged cargo test --workspace --test bdd

# Claude Code's LSP client, observed live (M3 stage 0, D16-D18;
# features/lsp/claude-client.feature, docs/lsp-stage0.md): only the @claude
# scenarios, with the `claude` capability. Cucumber's --tags filter replaces
# the capability and platform filter, so this recipe checks the platform
# itself. Needs `claude` on PATH, logged in (`claude auth login`) or with
# ANTHROPIC_API_KEY set, git, and this toolchain's rust-analyzer. Each run
# drives two short Haiku sessions, which cost real money. With
# CODETAGS_BDD_CLAUDE_SAVE=DIR, saves the sanitized recordings under DIR (how
# tests/fixtures/lsp is made).
test-claude:
    @if [ "{{ os() }}" = windows ]; then echo "test-claude: Linux and macOS only (the recorder plugin's wrapper is a sh script)"; exit 1; fi
    @command -v claude >/dev/null 2>&1 || { echo "test-claude: claude is not on PATH; install Claude Code (https://code.claude.com/docs/en/setup)"; exit 1; }
    @if [ -z "${ANTHROPIC_API_KEY:-}" ] && ! claude auth status 2>/dev/null | grep -q '"loggedIn": *true'; then echo "test-claude: Claude Code is not logged in; run: claude auth login (or set ANTHROPIC_API_KEY)"; exit 1; fi
    @rustup which rust-analyzer >/dev/null 2>&1 || { echo "test-claude: no rust-analyzer for this toolchain; add it with: rustup component add rust-analyzer"; exit 1; }
    @echo "test-claude: $(claude --version)"
    cargo build --workspace --bins --quiet
    CODETAGS_BDD_CAPABILITIES=claude cargo test --workspace --test bdd -- --tags @claude

bench:
    cargo bench --workspace

# Evaluation condition A, B, or C (P2.7, P5.5).
eval CONDITION:
    @echo "eval {{CONDITION}}: not built yet (P2.7)"
    @exit 1

# Environment report: mount backend, helper, providers. Builds with
# --workspace (not `cargo run -p`, which unifies features differently), then
# runs the binary outside cargo. It links the prebuilt libduckdb, which
# libduckdb-sys copies into target/debug/deps: on Linux and macOS the binary's
# run path finds it there (crates/codetags/build.rs, V72); Windows has no run
# path, so deps goes first on PATH (as an absolute POSIX path for Git's sh).
doctor:
    cargo build --workspace --bins --quiet
    bin="${CARGO_TARGET_DIR:-target}/debug"; if command -v cygpath >/dev/null 2>&1; then bin="$(cygpath -ua "$bin")"; else case "$bin" in /*) ;; *) bin="$PWD/$bin" ;; esac; fi; PATH="$bin/deps:$PATH" "$bin/codetags" doctor

# Indexes this repo with `codetags index --root .` (M1, D10) and prints the
# summary. Fails if the index fails (any provider failure) or reports any
# canonical-name collision. Builds and finds libduckdb as `doctor` does.
# Views are not refreshed yet: the materializer is P2.5.
dogfood:
    cargo build --workspace --bins --quiet
    bin="${CARGO_TARGET_DIR:-target}/debug"; if command -v cygpath >/dev/null 2>&1; then bin="$(cygpath -ua "$bin")"; else case "$bin" in /*) ;; *) bin="$PWD/$bin" ;; esac; fi; \
    out="$(PATH="$bin/deps:$PATH" "$bin/codetags" index --root .)"; status=$?; printf '%s\n' "$out"; \
    if [ "$status" -ne 0 ]; then echo "dogfood: codetags index failed (exit $status)"; exit "$status"; fi; \
    if printf '%s\n' "$out" | grep -q 'canonical-name collisions: [1-9]'; then echo "dogfood: canonical-name collisions (listed above)"; exit 1; fi; \
    echo "dogfood: indexed, no canonical-name collisions"

# M3 stage 0 (PLAN.md D18, docs/lsp-stage0.md): sets up the recorder that this
# repo's Claude Code plugin (tools/claude-plugins, enabled in .claude/settings.json)
# runs rust-analyzer through. Copies codetags-lsp to .codetags/local/bin (by
# rename, so a running recorder keeps its binary), writes the real
# rust-analyzer's absolute path (this toolchain's) to
# .codetags/local/rust-analyzer.path, and prints the steps. Unix only: the
# plugin's wrapper is a sh script.
lsp-record-setup:
    @if [ "{{ os() }}" = windows ]; then echo "lsp-record-setup: Unix only (the plugin's wrapper is a sh script)"; exit 1; fi
    cargo build --workspace --bins --quiet
    @mkdir -p .codetags/local/bin
    @cp "${CARGO_TARGET_DIR:-target}/debug/codetags-lsp" .codetags/local/bin/codetags-lsp.new && mv -f .codetags/local/bin/codetags-lsp.new .codetags/local/bin/codetags-lsp
    @ra="$(rustup which rust-analyzer)" || { echo "lsp-record-setup: no rust-analyzer for this toolchain; add it with: rustup component add rust-analyzer"; exit 1; }; \
    printf '%s\n' "$ra" > .codetags/local/rust-analyzer.path; \
    echo "lsp-record-setup: recorder .codetags/local/bin/codetags-lsp"; \
    echo "lsp-record-setup: real server $ra ($("$ra" --version))"
    @echo ""
    @echo "Next (docs/lsp-stage0.md has the full runbook):"
    @echo "  1. Quit any Claude Code session in this repo. In the main checkout ($PWD), run: claude"
    @echo "     Accept the trust dialog if asked. The project enables the codetags-lsp shim plugin instead (M3); in /plugin enable codetags-lsp-recorder@codetags-local and disable codetags-lsp (docs/lsp-stage0.md, appendix)."
    @echo "  2. Do the scripted actions in docs/lsp-stage0.md, appendix A.3, then /exit."
    @echo "  3. Logs: .codetags/local/lsp-<UTC time>-<pid>.jsonl, one per server process. Summarize with:"
    @echo "     .codetags/local/bin/codetags-lsp analyze .codetags/local/lsp-<...>.jsonl"

# M3 stages 1 and 2 (PLAN.md D14-D22, docs/lsp-shim.md): wires this
# checkout's Claude Code plugin (tools/claude-plugins/codetags-lsp) and VS
# Code (.vscode/settings.json) to one rust-analyzer through the codetags
# shim and lspmux. Copies codetags-lsp to .codetags/local/bin (by rename, so
# running shims keep theirs) and writes this toolchain's rust-analyzer path to
# .codetags/local/rust-analyzer.path, which the sh wrappers read. On Windows,
# where clients start only executables (V118, V119), it installs wrapper
# executables beside the sh wrappers instead. Last, `codetags lsp setup`
# shows its change to lspmux's config and asks before writing (D20). Run
# `setup-lspmux` first.
lsp-setup:
    @[ -x .codetags/local/bin/lspmux ] || { echo "lsp-setup: lspmux is not installed; run: just setup-lspmux"; exit 1; }
    cargo build --workspace --bins --quiet
    @mkdir -p .codetags/local/bin
    @cp "${CARGO_TARGET_DIR:-target}/debug/codetags-lsp{{ exe }}" ".codetags/local/bin/codetags-lsp{{ exe }}.new" && mv -f ".codetags/local/bin/codetags-lsp{{ exe }}.new" ".codetags/local/bin/codetags-lsp{{ exe }}"
    @ra="$(rustup which rust-analyzer)" || { echo "lsp-setup: no rust-analyzer for this toolchain; add it with: rustup component add rust-analyzer"; exit 1; }; \
    printf '%s\n' "$ra" > .codetags/local/rust-analyzer.path; \
    echo "lsp-setup: shim .codetags/local/bin/codetags-lsp, real server $ra ($("$ra" --version))"; \
    if [ "{{ os() }}" = windows ]; then \
        lspmux="$(cygpath -w "$PWD/.codetags/local/bin/lspmux.exe")"; \
        .codetags/local/bin/codetags-lsp.exe install-wrapper --role agent --server "$ra" --lspmux "$lspmux" tools/claude-plugins/codetags-lsp/scripts/rust-analyzer && \
        .codetags/local/bin/codetags-lsp.exe install-wrapper --role editor --server "$ra" --lspmux "$lspmux" tools/vscode/rust-analyzer || exit 1; \
    fi
    bin="${CARGO_TARGET_DIR:-target}/debug"; if command -v cygpath >/dev/null 2>&1; then bin="$(cygpath -ua "$bin")"; else case "$bin" in /*) ;; *) bin="$PWD/$bin" ;; esac; fi; PATH="$bin/deps:$PATH" "$bin/codetags" lsp setup
    @echo ""
    @echo "Next (docs/lsp-shim.md has the details):"
    @echo "  1. Claude Code: restart it in this checkout; /plugin lists codetags-lsp@codetags-local, enabled."
    @echo "  2. VS Code: reload the window; .vscode/settings.json points rust-analyzer.server.path at tools/vscode/rust-analyzer."
    @echo "  3. Check: codetags doctor (just doctor), and .codetags/local/bin/lspmux status"

# --- development ---------------------------------------------------------

# Point tagma-core at a local checkout (e.g. ../kvtag) with a marked [patch]
# block in the committed .cargo/config.toml. With no PATH, remove the block.
# `override-check` fails while the block is present.
dev-tagma PATH="":
    @sed '/^# >>> dev-tagma/,/^# <<< dev-tagma/d' .cargo/config.toml > .cargo/config.toml.new && mv .cargo/config.toml.new .cargo/config.toml
    @if [ -z "{{PATH}}" ]; then echo "dev-tagma: using the pinned git revision"; else printf '# >>> dev-tagma: local override; remove with "just dev-tagma" before committing\n[patch."https://github.com/benjaminabbitt/tagma"]\ntagma-core = { path = "%s/crates/tagma-core" }\n# <<< dev-tagma\n' "{{PATH}}" >> .cargo/config.toml && echo "dev-tagma: using {{PATH}}"; fi
