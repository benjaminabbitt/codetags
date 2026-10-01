# codetags task runner: the only entry point for agents and CI (PLAN.md §0.5).
# Recipe names are a frozen contract (PLAN.md Appendix A). A recipe whose phase
# is not built yet FAILS with the phase name, so a stub never passes for a real
# check (PLAN.md §0.11).
#
# Recipes run under `sh` on every OS (Git's sh on Windows), so keep them POSIX.

default: check

# --- setup ---------------------------------------------------------------

# Toolchain from rust-toolchain.toml, plus the cargo tools `check` needs.
setup:
    rustup toolchain install
    @command -v cargo-deny >/dev/null 2>&1 || echo "setup: cargo-deny missing; install with: cargo install cargo-deny --locked"
    @echo "setup: ready"

# Pinned SCIP providers from providers.toml (P1).
# Idempotent. Rust: the rust-analyzer component of the toolchain that
# providers.toml pins (the one in rust-toolchain.toml).
setup-providers:
    rustup toolchain install
    rustup component add rust-analyzer --toolchain "$(sed -n '/^\[rust-analyzer\]/,/^\[/s/^toolchain *= *"\(.*\)".*/\1/p' providers.toml)"
    rust-analyzer --version
    @echo "setup-providers: ready"

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
    cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
    cargo clippy --workspace --all-targets --target aarch64-apple-darwin -- -D warnings

# Licence boundaries (PLAN.md §0.12, D9); policy in deny.toml.
license-check:
    cargo deny check licenses bans

test:
    cargo test --workspace

# Gherkin features (PLAN.md §0.2) that need no mount, provider, or privilege.
# Jobs with more capabilities set CODETAGS_BDD_CAPABILITIES (codetags-bdd).
# Every cargo invocation here selects --workspace: a narrower selection unifies
# features differently and rebuilds dependencies.
bdd:
    cargo test --workspace --test bdd

# Validates this repo's .codetags/tags (P6.2). Vacuously green while no tags
# file exists; fails once one exists until the validator is built.
tags-check:
    @if [ -e .codetags/tags ]; then echo "tags-check: .codetags/tags exists but the validator is not built yet (P6.2)"; exit 1; else echo "tags-check: no .codetags/tags"; fi

# --- capability-gated suites ---------------------------------------------

# Mount features against this OS's live backend (P0b, P5): the bdd suite with
# the `mount` capability enabled, plus `winfsp` on Windows, whose backend needs
# WinFsp installed. Needs /dev/fuse and a setuid fusermount3 on Linux;
# `codetags doctor` explains what is missing.
test-mount:
    CODETAGS_BDD_CAPABILITIES={{ if os() == "windows" { "mount,winfsp" } else { "mount" } }} cargo test --workspace --test bdd

# Index features against the fixtures (P1): the bdd suite with the
# `providers` capability enabled. Run `setup-providers` first.
test-providers:
    CODETAGS_BDD_CAPABILITIES=providers cargo test --workspace --test bdd

# Edge-count regression against tests/baselines (P1.8).
baseline-check:
    @echo "baseline-check: not built yet (P1.8)"
    @exit 1

# Privileged-helper features; CI only (P4.4).
test-privileged:
    @echo "test-privileged: not built yet (P4.4)"
    @exit 1

bench:
    cargo bench --workspace

# Evaluation condition A, B, or C (P2.7, P5.5).
eval CONDITION:
    @echo "eval {{CONDITION}}: not built yet (P2.7)"
    @exit 1

# Environment report: mount backend, helper, providers. Builds with
# --workspace (not `cargo run -p`, which unifies features differently), then
# runs the binary; it links no DuckDB, so it runs outside cargo.
doctor:
    cargo build --workspace --bins --quiet
    "${CARGO_TARGET_DIR:-target}/debug/codetags" doctor

# --- development ---------------------------------------------------------

# Point tagma-core at a local checkout (e.g. ../kvtag) with a marked [patch]
# block in the committed .cargo/config.toml. With no PATH, remove the block.
# `override-check` fails while the block is present.
dev-tagma PATH="":
    @sed '/^# >>> dev-tagma/,/^# <<< dev-tagma/d' .cargo/config.toml > .cargo/config.toml.new && mv .cargo/config.toml.new .cargo/config.toml
    @if [ -z "{{PATH}}" ]; then echo "dev-tagma: using the pinned git revision"; else printf '# >>> dev-tagma: local override; remove with "just dev-tagma" before committing\n[patch."https://github.com/benjaminabbitt/tagma"]\ntagma-core = { path = "%s/crates/tagma-core" }\n# <<< dev-tagma\n' "{{PATH}}" >> .cargo/config.toml && echo "dev-tagma: using {{PATH}}"; fi
