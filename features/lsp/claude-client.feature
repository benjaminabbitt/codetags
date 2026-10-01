@claude @linux @macos @serial
Feature: Claude Code's LSP client, observed headless
  M3 stage 0 (PLAN.md D16-D18, docs/lsp-stage0.md). Claude Code runs headless
  (`claude -p`, Haiku) in a scratch copy of the Rust fixture, with the
  recorder plugin loaded through `--plugin-dir`, so rust-analyzer runs
  through `codetags-lsp record`. Each `When` step is one scripted action,
  sent as its own user turn of one session. The `Then` steps read
  `codetags-lsp analyze` of the recording and state what Claude Code's LSP
  client was observed to do, so a change in Claude Code fails a scenario.
  docs/verification.md V110 records the facts with the Claude Code version.

  Within one test run, every scenario of a rule shares one live session:
  the first scenario runs it, and the others read its recording.

  Background:
    Given a scratch Rust project from the fixture "rust" with the LSP recorder plugin

  Rule: The handshake, from a session with one hover

    Background:
      When Claude Code asks the LSP to hover on "Ledger" in "src/ledger.rs"

    Scenario: Headless Claude Code loads the recorder plugin from --plugin-dir
      Then Claude Code loaded the plugin "codetags-lsp-recorder"
      And Claude Code did not load the plugin "rust-analyzer-lsp"
      And the recorder wrote 1 recording

    Scenario: initialize names the project directory as rootUri
      Then the client's initialize rootUri is the project root

    Scenario: initialize names the project directory as the only workspace folder
      Then the client's initialize workspaceFolders are only the project root

    Scenario: The client does not advertise didChangeWatchedFiles
      Then the client capability "workspace.didChangeWatchedFiles" is absent

    Scenario: The client offers only UTF-16 positions
      Then the client capability "general.positionEncodings" is '["utf-16"]'

    Scenario: The client does not advertise workDoneProgress
      Then the client capability "window.workDoneProgress" is absent

    Scenario: The client says it does not answer workspace/configuration
      Then the client capability "workspace.configuration" is 'false'

    Scenario: The client says it does not support workspace folder changes
      Then the client capability "workspace.workspaceFolders" is 'false'

    Scenario: The client says it sends didSave but not willSave
      Then the client capability "textDocument.synchronization" is '{"didSave":true,"dynamicRegistration":false,"willSave":false,"willSaveWaitUntil":false}'

    Scenario: So rust-analyzer sends none of the requests reported as refused
      Then the server sent no "client/registerCapability" request
      And the server sent no "window/workDoneProgress/create" request
      And the server sent no "workspace/configuration" request

    Scenario: The client refuses workspace/diagnostic/refresh
      Then the client answered "workspace/diagnostic/refresh" with error -32601

    Scenario: The session ends with shutdown, and no exit
      Then the client's last request was "shutdown"
      And the client sent no "exit"
      And the recording has no end-of-stream or exit record

  Rule: Document sync, from the stage-0 script

    Background:
      When Claude Code reads "src/ledger.rs" with the Read tool
      And Claude Code asks the LSP for the definition of "Ledger" in "src/lib.rs"
      And Claude Code asks the LSP for references to "record" in "src/ledger.rs"
      And Claude Code asks the LSP to hover on "Ledger" in "src/ledger.rs"
      And Claude Code asks the LSP for implementations of "Apply" in "src/charge.rs"
      And Claude Code asks the LSP for the call hierarchy of "settle" in "src/lib.rs"
      And Claude Code appends "// stage0: edit tool" to "src/ledger.rs" with the Edit tool, then hovers on "Ledger"
      And Claude Code creates "src/stage0_scratch.rs" holding "pub fn stage0() -> u32 { 1 }" with the Write tool, then lists its symbols
      And Claude Code runs "sed -i 's|// stage0: edit tool|// stage0: sed|' src/ledger.rs" through Bash, then hovers on "Ledger" in "src/ledger.rs"
      And Claude Code runs "git checkout -- src/ledger.rs && rm src/stage0_scratch.rs" through Bash, then hovers on "Ledger" in "src/ledger.rs"

    Scenario: The Read tool neither starts the server nor opens the document
      Then after the "Read" the client's document notifications were "none"
      And after the "Read" the client's first request was "none"

    Scenario: The server starts, and the document opens, at the first LSP use
      Then after the "definition" the client's first request was "initialize"
      And after the "definition" the client's document notifications were "didOpen src/lib.rs"

    Scenario: Each document opens at its first LSP use, once
      Then after the "references" the client's document notifications were "didOpen src/ledger.rs"
      And after the "hover" the client's document notifications were "none"
      And after the "implementations" the client's document notifications were "didOpen src/charge.rs"

    Scenario: An Edit is followed by didChange and didSave
      Then after the "Edit" the client's document notifications were "didChange src/ledger.rs, didSave src/ledger.rs"

    Scenario: didChange carries the full text
      Then every didChange the client sent carried the full text

    Scenario: A new file from Write opens when the LSP is first used on it
      Then after the "Write" the client's document notifications were "didOpen src/stage0_scratch.rs"

    Scenario: A change made by sed through Bash reaches the server not at all
      Then after the "sed" the client's document notifications were "none"

    Scenario: A git checkout through Bash reaches the server not at all
      Then after the "git checkout" the client's document notifications were "none"

    Scenario: The client never sends didChangeWatchedFiles or didClose
      Then the client sent no "workspace/didChangeWatchedFiles"
      And the client sent no "textDocument/didClose"
