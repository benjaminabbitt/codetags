Feature: codetags-lsp serve shares one language server between sessions through lspmux
  M3 stages 1 and 2 (PLAN.md D14-D22, docs/proxy-zero-change.md §3.5, §7).
  Editors and agents start `codetags-lsp serve` (or a wrapper that runs it)
  instead of the language server. It runs upstream `lspmux client`, starting
  `lspmux server` first if nothing answers, and relays the session between
  the editor and it. An agent session never sends document contents (D16):
  the server reads files from disk. A multi-root `initialize` gets an LSP
  error, the root is normalized to the project root in the client's own
  spelling, and a client's own `workspace/didChangeWatchedFiles` is dropped.

  The language server is a fake one, which logs every message it receives
  and answers any request; each scenario has its own home directory, lspmux
  config and daemon. Scenarios tagged `@lspmux` need the pinned lspmux
  (`just setup-lspmux`).

  Scenario: --version goes straight to the real server and starts no daemon
    Given lspmux is set up in an isolated home
    And a fake language server
    When codetags-lsp serve runs the fake server as the "editor" with the arguments "--version"
    Then it exits with status 0
    And stdout matches "^fake-lsp 9\.9\.9\n$"
    And no lspmux daemon is answering

  Scenario: A multi-root initialize gets an LSP error, and no daemon starts
    Given lspmux is set up in an isolated home
    And a fake language server
    When an "editor" session "vscode" sends an initialize with 2 workspace folders
    Then session "vscode" got the error -32602 for its initialize
    And no lspmux daemon is answering

  @lspmux
  Scenario: The daemon starts on demand
    Given lspmux is set up in an isolated home
    And a fake language server
    And no lspmux daemon is answering
    When an "editor" session "vscode" starts through codetags-lsp serve
    Then an lspmux daemon is answering
    And the fake server was started 1 time

  @lspmux
  Scenario: An editor session and an agent session share one server process
    Given lspmux is set up in an isolated home
    And a fake language server
    When an "editor" session "vscode" starts through codetags-lsp serve
    And an "agent" session "claude" starts through codetags-lsp serve
    And session "vscode" asks "textDocument/hover" for "src/lib.rs"
    And session "claude" asks "textDocument/hover" for "src/lib.rs"
    Then the fake server was started 1 time
    And the fake server received "textDocument/hover" 2 times

  @lspmux
  Scenario: Two agent sessions started at once start one daemon and share one server
    Given lspmux is set up in an isolated home
    And a fake language server
    When the "agent" sessions "claude-1" and "claude-2" start at once through codetags-lsp serve
    Then the fake server was started 1 time
    And the lspmux daemon was started 1 time

  @lspmux
  Scenario: The agent's document messages never reach the server
    Given lspmux is set up in an isolated home
    And a fake language server
    When an "agent" session "claude" starts through codetags-lsp serve
    And session "claude" sends "textDocument/didOpen" for "src/lib.rs"
    And session "claude" sends "textDocument/didChange" for "src/lib.rs"
    And session "claude" sends "textDocument/didSave" for "src/lib.rs"
    And session "claude" sends "textDocument/didClose" for "src/lib.rs"
    And session "claude" asks "textDocument/hover" for "src/lib.rs"
    Then the fake server received "textDocument/hover" 1 time
    And the fake server received no document notifications

  @lspmux
  Scenario: The editor's document messages reach the server
    Given lspmux is set up in an isolated home
    And a fake language server
    When an "editor" session "vscode" starts through codetags-lsp serve
    And session "vscode" sends "textDocument/didOpen" for "src/lib.rs"
    And session "vscode" sends "textDocument/didChange" for "src/lib.rs"
    And session "vscode" sends "textDocument/didSave" for "src/lib.rs"
    And session "vscode" sends "textDocument/didClose" for "src/lib.rs"
    And session "vscode" asks "textDocument/hover" for "src/lib.rs"
    Then the fake server received "textDocument/didOpen" 1 time
    And the fake server received "textDocument/didChange" 1 time
    And the fake server received "textDocument/didSave" 1 time
    And the fake server received "textDocument/didClose" 1 time

  @lspmux
  Scenario: A client's own didChangeWatchedFiles never reaches the server
    Given lspmux is set up in an isolated home
    And a fake language server
    When an "editor" session "vscode" starts through codetags-lsp serve
    And session "vscode" sends "workspace/didChangeWatchedFiles" for "src/lib.rs"
    And session "vscode" asks "textDocument/hover" for "src/lib.rs"
    Then the fake server received "textDocument/hover" 1 time
    And the fake server received no "workspace/didChangeWatchedFiles"

  @lspmux
  Scenario: The server is initialized with the Cargo workspace root, as the client spells it
    Given lspmux is set up in an isolated home
    And a fake language server
    And the project is a Cargo workspace with the member crate "crates/member"
    When an "agent" session "claude" starts through codetags-lsp serve in "crates/member"
    Then the fake server's initialize named the project root
    And the fake server's initialize did not advertise "workspace.didChangeWatchedFiles"

  @lspmux
  Scenario: workspace/configuration reaching an agent session is answered by its shim
    Given lspmux is set up in an isolated home
    And a fake language server
    When an "agent" session "claude" starts through codetags-lsp serve
    And session "claude" asks "fake/askConfiguration" for "src/lib.rs"
    Then session "claude"'s answer to "fake/askConfiguration" was '[null]'

  @lspmux
  Scenario: Killing an agent session after shutdown, without exit, leaves the server serving the editor
    Given lspmux is set up in an isolated home
    And a fake language server
    When an "editor" session "vscode" starts through codetags-lsp serve
    And an "agent" session "claude" starts through codetags-lsp serve
    And session "claude" shuts down and is killed without exit
    And session "vscode" asks "textDocument/hover" for "src/lib.rs"
    Then the fake server was started 1 time
    And the fake server received no "shutdown"
    And the fake server received "textDocument/hover" 1 time

  @lspmux
  Scenario: A scripted VS Code-style session through the editor wrapper
    Given lspmux is set up in an isolated home
    And a fake language server
    And a wrapper "rust-analyzer" that runs the fake server as the "editor"
    When the wrapper "rust-analyzer" runs with the arguments "--version"
    Then it exits with status 0
    And stdout matches "^fake-lsp 9\.9\.9\n$"
    When a VS Code-style session "vscode" runs through the wrapper "rust-analyzer"
    Then session "vscode" exited with status 0
    And the fake server received "textDocument/didOpen" 1 time
    And the fake server received "textDocument/didChange" 1 time
    And the fake server received "textDocument/hover" 1 time
    And the fake server received no "workspace/didChangeWatchedFiles"
    And the fake server received no "shutdown"
    And the fake server's initialize did not advertise "workspace.didChangeWatchedFiles"

  @lspmux
  Scenario: A recorded Claude Code session replayed through the shim sends the server no document messages
    Given lspmux is set up in an isolated home
    And a fake language server
    When the recorded Claude Code session "claude-code-2.1.286/document-sync" replays through codetags-lsp serve as the "agent"
    Then the fake server received "textDocument/definition" 4 times
    And the fake server received "textDocument/references" 1 time
    And the fake server received no document notifications
    And the fake server was started 1 time
