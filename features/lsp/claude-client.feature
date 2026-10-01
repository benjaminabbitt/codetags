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

  # M3 stages 1 and 2 (PLAN.md D14-D22): the same client through the agent
  # shim and lspmux. The rule's setup swaps in the codetags-lsp plugin, gives
  # lspmux an isolated config (XDG_CONFIG_HOME, so Linux only: on macOS
  # lspmux's config lives under $HOME, which Claude Code itself needs), and
  # records both sides: the shim's log is what Claude Code sent and got, and
  # a recorder between lspmux and rust-analyzer logs what the server got.
  @linux
  Rule: Through the shim and lspmux

    Background:
      Given the scratch project uses the codetags-lsp plugin through lspmux
      When Claude Code asks the LSP for the definition of "Ledger" in "src/lib.rs"
      And Claude Code asks the LSP for references to "record" in "src/ledger.rs"
      And Claude Code appends "// shim: edit tool" to "src/ledger.rs" with the Edit tool, then hovers on "Ledger"

    Scenario: Headless Claude Code loads the codetags-lsp plugin from --plugin-dir
      Then Claude Code loaded the plugin "codetags-lsp"
      And Claude Code did not load the plugin "rust-analyzer-lsp"

    Scenario: Definitions and references still answer through the shim
      Then a "textDocument/definition" request from the client got a non-empty result
      And a "textDocument/references" request from the client got a non-empty result

    Scenario: The client sends document messages, and the server receives none of them
      Then after the "definition" the client's document notifications were "didOpen src/lib.rs"
      And after the "Edit" the client's document notifications were "didChange src/ledger.rs, didSave src/ledger.rs"
      And the language server received no document notifications
      And one language server process served the session

  # M3, before stage 3: is rust-analyzer stale after changes made outside
  # Claude Code? Direct, it is (V110). Through the shim, the server is told
  # nothing about any change (D16: the agent's document messages are dropped,
  # and the client never sends didChangeWatchedFiles), and the shim takes
  # didChangeWatchedFiles out of initialize, so rust-analyzer watches the
  # files itself (V122). These rules ask whether that is enough (V126).
  #
  # Each rule first searches the workspace until rust-analyzer has indexed
  # the project, so every change below lands on a running, loaded server.
  # Each change runs through Bash, then `sleep 2` (the watcher is
  # asynchronous), then one LSP lookup; the `Then` steps read that lookup's
  # answer in the shim's log. The markers are constants, and the new file is
  # made with cp and sed: in dontAsk mode Claude Code 2.1.286 denied a sed
  # that writes a function, and a `cat > … <<<` with an `echo … >>`, even
  # with exact allow rules (V127).
  @linux
  Rule: Unopened files changed outside Claude Code, through the shim

    Background:
      Given the scratch project uses the codetags-lsp plugin through lspmux
      When Claude Code searches the workspace for "Ledger"
      And Claude Code runs "sed -i 's|^pub struct Ledger|pub const STAGE_SED_MARKER: u32 = 7; &|' src/ledger.rs" through Bash, then searches the workspace for "STAGE_SED_MARKER"
      And Claude Code runs "git checkout -- src/ledger.rs" through Bash, then searches the workspace for "STAGE_SED_MARKER"
      And Claude Code runs "cp src/macros.rs src/stage_new.rs && sed -i 's|^macro_rules|pub const STAGE_NEW_MARKER: u32 = 9; &|' src/stage_new.rs && sed -i 's|^pub mod pipeline;|pub mod pipeline; pub mod stage_new;|' src/lib.rs" through Bash, then searches the workspace for "STAGE_NEW_MARKER"

    Scenario: The server has indexed the project before anything changes
      Then after the "workspace search" a workspace symbol search for "Ledger" found it

    Scenario: A constant sed adds to a file the agent never opened is found
      Then after the "sed" a workspace symbol search for "STAGE_SED_MARKER" found it

    Scenario: After git checkout restores the file, the constant is gone
      Then after the "git checkout" a workspace symbol search for "STAGE_SED_MARKER" found nothing

    Scenario: A new file made through Bash, never opened, is indexed
      Then after the "cp" a workspace symbol search for "STAGE_NEW_MARKER" found it

    Scenario: Nobody told the server about the changes
      Then after the "sed" the client's document notifications were "none"
      And after the "git checkout" the client's document notifications were "none"
      And after the "cp" the client's document notifications were "none"
      And the client sent no "workspace/didChangeWatchedFiles"
      And the language server received no document notifications

  # The D16 case: Claude Code has opened the document (a didOpen, which the
  # shim drops), and believes the server holds its old text.
  @linux
  Rule: An opened file changed outside Claude Code, through the shim

    Background:
      Given the scratch project uses the codetags-lsp plugin through lspmux
      When Claude Code searches the workspace for "Ledger"
      And Claude Code reads "src/charge.rs" with the Read tool
      And Claude Code asks the LSP to hover on "Apply" in "src/charge.rs"
      And Claude Code runs "sed -i 's|^pub struct Fee|pub const STAGE_OPENED_MARKER: u32 = 8; &|' src/charge.rs" through Bash, then lists the symbols of "src/charge.rs"
      And Claude Code runs "git checkout -- src/charge.rs" through Bash, then lists the symbols of "src/charge.rs"

    Scenario: The agent opened the document before it changed, and the server got nothing
      Then a "textDocument/hover" request from the client got a non-empty result
      And after the "hover" the client's document notifications were "didOpen src/charge.rs"
      And after the "sed" the client's document notifications were "none"
      And the language server received no document notifications

    Scenario: The opened document's symbols show the constant sed added
      Then after the "sed" the symbols of "src/charge.rs" include "STAGE_OPENED_MARKER"

    Scenario: After git checkout restores it, the opened document's symbols do not
      Then after the "git checkout" the symbols of "src/charge.rs" do not include "STAGE_OPENED_MARKER"
