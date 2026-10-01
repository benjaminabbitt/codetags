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
setup-providers:
    @echo "setup-providers: not built yet (P1)"
    @exit 1

# --- the green light -----------------------------------------------------

# Universal check: must exit 0 on Linux, macOS, and Windows (PLAN.md §0.6).
check: fmt-check lint test tags-check
    @echo "check: green"

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

lint:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

# Validates this repo's .codetags/tags (P6.2). Vacuously green while no tags
# file exists; fails once one exists until the validator is built.
tags-check:
    @if [ -e .codetags/tags ]; then echo "tags-check: .codetags/tags exists but the validator is not built yet (P6.2)"; exit 1; else echo "tags-check: no .codetags/tags"; fi

# --- capability-gated suites ---------------------------------------------

# View, plugin, and mount-tagging features against this OS's live backend (P0b, P5).
test-mount:
    @echo "test-mount: not built yet (P0b/P5)"
    @exit 1

# Index features against the fixtures (P1).
test-providers:
    @echo "test-providers: not built yet (P1)"
    @exit 1

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

# Environment report: mount backend, helper, providers (P0b S1).
doctor:
    @echo "doctor: not built yet (P0b)"
    @exit 1

# --- development ---------------------------------------------------------

# Point tagma-core at a local checkout (e.g. ../kvtag), via an ignored
# .cargo/config.toml. With no PATH, remove the override.
dev-tagma PATH="":
    @if [ -z "{{PATH}}" ]; then rm -f .cargo/config.toml; echo "dev-tagma: using the pinned git revision"; else mkdir -p .cargo && printf '[patch."https://github.com/benjaminabbitt/tagma"]\ntagma-core = { path = "%s/crates/tagma-core" }\n' "{{PATH}}" > .cargo/config.toml && echo "dev-tagma: using {{PATH}}"; fi
