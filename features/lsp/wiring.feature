@lspmux @providers
Feature: After lsp-setup, Claude Code and VS Code share one rust-analyzer
  M3 stages 1 and 2 (PLAN.md D16-D22, docs/lsp-shim.md §1). This drives what
  that runbook asks a person to check after `just setup-lspmux` and
  `just lsp-setup`, rather than trusting that someone did. Today nothing
  runs the recipe itself: the live test re-creates its effect in Rust.

  Each scenario copies the Rust fixture into a scratch git checkout, with this
  repo's LSP wiring beside it: tools/lsp/, the Claude Code plugin
  tools/claude-plugins/codetags-lsp/, tools/vscode/ and .vscode/settings.json.
  "lspmux is installed" copies the pinned lspmux to where
  `just setup-lspmux` puts it. "lsp-setup has run" runs the script that
  `just lsp-setup` runs, in an isolated home, answering yes. Sessions start
  the committed wrappers the way each client does. They talk to this
  toolchain's real rust-analyzer through the real lspmux. Nothing here
  touches this checkout's `.codetags/local` or the user's lspmux config.
  Every scenario needs the pinned lspmux (`just setup-lspmux`), this
  toolchain's rust-analyzer, and Node: CI's providers job has them all.

  Claude Code's plugin wrapper is started the way Claude Code starts a
  plugin's language server: by its path, in the project, with
  CLAUDE_PROJECT_DIR and CLAUDE_PLUGIN_ROOT set (V107, V114). Claude Code is
  a Bun binary, and how Bun resolves the path on Windows is not checked
  (V119). VS Code's wrapper is started the way the rust-analyzer extension
  starts it (V118). `rust-analyzer.server.path` is read from
  .vscode/settings.json with ${workspaceFolder} substituted. The path is run
  with --version and then with no arguments, through Node's
  child_process.spawn with no shell. On Windows, Node finds the
  rust-analyzer.exe beside the sh wrapper, as libuv does (V119).

  Background:
    Given an isolated home for lspmux
    And a scratch checkout of the Rust fixture with this repo's LSP wiring

  # The wrappers degrade on purpose: a client with no setup still gets a
  # language server. These say that the degradation is visible.
  @linux @macos
  Rule: Without a working setup, the wrappers run rust-analyzer directly and say why

    Scenario: Before lsp-setup, Claude Code's plugin runs rust-analyzer directly
      When Claude Code's plugin wrapper runs with the arguments "--version"
      Then it exits with status 0
      And stdout matches "^rust-analyzer "
      And stderr matches "codetags-lsp is not set up \(run: just lsp-setup\)"

    Scenario: Before lsp-setup, doctor says the clients run rust-analyzer without the shim
      When codetags doctor runs in the scratch checkout
      Then stdout matches "(?m)^lsp wiring: .*without the shim.*just lsp-setup"

    # D25. The wrapper used to run `codetags-lsp serve` on any
    # .codetags/local/bin/codetags-lsp it found. A stage-0 build, from
    # `just lsp-record-setup`, has no `serve`, so with lspmux installed
    # Claude Code's language server failed to start. The wrapper now checks
    # `codetags-lsp serve --help` first.
    Scenario: A shim from before stage 1 is not used
      Given lspmux is installed in the scratch checkout
      And the scratch checkout's shim is a build with no serve command
      When Claude Code's plugin wrapper runs with the arguments "--version"
      Then it exits with status 0
      And stdout matches "^rust-analyzer "
      And stderr matches "cannot serve \(run: just lsp-setup\)"

  Rule: lsp-setup wires both clients to one rust-analyzer

    Background:
      Given lspmux is installed in the scratch checkout
      And lsp-setup has run in the scratch checkout

    Scenario: lsp-setup installs a shim that serves, and records this toolchain's rust-analyzer
      Then the scratch checkout's shim can serve
      And the scratch checkout's rust-analyzer path names this toolchain's rust-analyzer
      And lspmux's config sets "pass_environment" to '["CODETAGS_KEY_*"]'

    # D25: doctor's "lsp wiring" line. Before it, doctor checked only
    # lspmux's binary and config, so it was green on a machine where neither
    # client used the shim.
    Scenario: After lsp-setup, doctor says the clients run through the shim
      When codetags doctor runs in the scratch checkout
      Then stdout matches "(?m)^lsp wiring: .*through the shim"
      And stdout matches "(?m)^lspmux config: .*as codetags lsp setup writes it"

    Scenario: VS Code's --version check reaches the real rust-analyzer and starts no daemon
      When VS Code checks its rust-analyzer's version
      Then it exits with status 0
      And stdout matches "^rust-analyzer "
      And no lspmux daemon is answering

    Scenario: Claude Code's plugin and VS Code share one rust-analyzer
      When Claude Code's plugin starts its language server as session "claude"
      And VS Code starts its rust-analyzer as session "vscode"
      And session "claude" searches the workspace for "Ledger" until it is found
      And session "vscode" hovers on "Ledger" in "src/lib.rs"
      Then session "vscode"'s hover is not empty
      And lspmux status shows 1 instance with 2 clients
      And the lspmux daemon was started 1 time

  # V126 found this with a scripted probe on Linux only. These make it a
  # scenario on all three OSes. The shim drops the agent's document messages
  # (D16) and takes watched files out of initialize, so rust-analyzer watches
  # the disk itself (V122). Each edit below is made by the step, not by a
  # client, so no client sends any notification about it.
  Rule: Changes on disk reach rust-analyzer with no client telling it

    Background:
      Given lspmux is installed in the scratch checkout
      And lsp-setup has run in the scratch checkout
      When Claude Code's plugin starts its language server as session "claude"
      And session "claude" searches the workspace for "Ledger" until it is found

    Scenario: A constant added to a file the agent never opened is found
      When "src/ledger.rs" gets the line "pub const STAGE_SED_MARKER: u32 = 7;" before the line starting "pub struct Ledger"
      Then within 10 s session "claude"'s workspace search for "STAGE_SED_MARKER" finds it

    Scenario: After git restores the file, the constant is gone
      When "src/ledger.rs" gets the line "pub const STAGE_SED_MARKER: u32 = 7;" before the line starting "pub struct Ledger"
      Then within 10 s session "claude"'s workspace search for "STAGE_SED_MARKER" finds it
      When git restores "src/ledger.rs"
      Then within 10 s session "claude"'s workspace search for "STAGE_SED_MARKER" finds nothing

    Scenario: A new file, never opened, is indexed
      When the new file "src/stage_new.rs" holds "pub const STAGE_NEW_MARKER: u32 = 9;"
      And "src/lib.rs" gets the line "pub mod stage_new;" before the line starting "pub mod pipeline;"
      Then within 10 s session "claude"'s workspace search for "STAGE_NEW_MARKER" finds it

    Scenario: A document the agent opened shows a change made on disk
      When session "claude" opens "src/charge.rs"
      And "src/charge.rs" gets the line "pub const STAGE_OPENED_MARKER: u32 = 8;" before the line starting "pub struct Fee"
      Then within 10 s session "claude"'s symbols of "src/charge.rs" include "STAGE_OPENED_MARKER"

    # The control: this shows the scenario above can fail. An editor's open
    # document is the editor's text, so rust-analyzer rightly ignores the
    # disk for it. Without D16, an agent's document would behave the same
    # way, and go stale.
    Scenario: A document an editor opened keeps the editor's text when the disk changes
      When VS Code starts its rust-analyzer as session "vscode"
      And session "vscode" opens "src/charge.rs"
      And "src/charge.rs" gets the line "pub const STAGE_OPENED_MARKER: u32 = 8;" before the line starting "pub struct Fee"
      Then for 3 s session "vscode"'s symbols of "src/charge.rs" do not include "STAGE_OPENED_MARKER"

  # D26 (V132). Both clients reach the checkout through a symlink, as a
  # checkout under a symlinked directory is. On macOS, rust-analyzer's
  # watcher reports canonical paths, so with the root as the client spells
  # it nothing on disk ever reached it. The shim gives the server the
  # canonical root and rewrites URIs both ways.
  @linux @macos
  Rule: Through a symlinked path, changes on disk still reach rust-analyzer

    Background:
      Given lspmux is installed in the scratch checkout
      And lsp-setup has run in the scratch checkout
      And the sessions reach the scratch checkout through a symlink
      When Claude Code's plugin starts its language server as session "claude"
      And session "claude" searches the workspace for "Ledger" until it is found

    Scenario: Through a symlink, a constant added to a file the agent never opened is found
      When "src/ledger.rs" gets the line "pub const STAGE_SED_MARKER: u32 = 7;" before the line starting "pub struct Ledger"
      Then within 10 s session "claude"'s workspace search for "STAGE_SED_MARKER" finds it

    Scenario: Through a symlink, a document the agent opened shows a change made on disk
      When session "claude" opens "src/charge.rs"
      And "src/charge.rs" gets the line "pub const STAGE_OPENED_MARKER: u32 = 8;" before the line starting "pub struct Fee"
      Then within 10 s session "claude"'s symbols of "src/charge.rs" include "STAGE_OPENED_MARKER"

    Scenario: Through a symlink, VS Code finds a constant added to a file it never opened
      When VS Code starts its rust-analyzer as session "vscode"
      And "src/ledger.rs" gets the line "pub const STAGE_SED_MARKER: u32 = 7;" before the line starting "pub struct Ledger"
      Then within 10 s session "vscode"'s workspace search for "STAGE_SED_MARKER" finds it
