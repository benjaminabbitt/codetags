Feature: Recorded Claude Code sessions stay readable
  The live feature (claude-client.feature, `@claude`) saved its sanitized
  recordings under tests/fixtures/lsp/claude-code-<version>/, where the
  shim's tests (P3.4) will replay them as a fake client. These scenarios run
  `codetags-lsp analyze` on them in every job, so a fixture that stops
  parsing, or an analyzer that stops reading it the same way, fails here
  without a live session.

  Scenario: The recorded handshake reads as it was recorded
    When codetags-lsp analyzes the recorded session "claude-code-2.1.286/the-handshake"
    Then the client's initialize rootUri is the project root
    And the client's initialize workspaceFolders are only the project root
    And the client capability "workspace.didChangeWatchedFiles" is absent
    And the client capability "general.positionEncodings" is '["utf-16"]'
    And the client capability "workspace.configuration" is 'false'
    And the server sent no "client/registerCapability" request
    And the client answered "workspace/diagnostic/refresh" with error -32601
    And the client's last request was "shutdown"
    And the recording has no end-of-stream or exit record

  Scenario: The recorded stage-0 script reads as it was recorded
    When codetags-lsp analyzes the recorded session "claude-code-2.1.286/document-sync"
    Then after the "Read" the client's first request was "none"
    And after the "definition" the client's first request was "initialize"
    And after the "definition" the client's document notifications were "didOpen src/lib.rs"
    And after the "Edit" the client's document notifications were "didChange src/ledger.rs, didSave src/ledger.rs"
    And every didChange the client sent carried the full text
    And after the "Write" the client's document notifications were "didOpen src/stage0_scratch.rs"
    And after the "sed" the client's document notifications were "none"
    And after the "git checkout" the client's document notifications were "none"
    And the client sent no "workspace/didChangeWatchedFiles"

  Scenario: The recorded shim session saw external changes to a document it had opened
    When codetags-lsp analyzes the recorded session "claude-code-2.1.286/an-opened-file-changed-outside-claude-code"
    Then after the "workspace search" a workspace symbol search for "Ledger" found it
    And after the "hover" the client's document notifications were "didOpen src/charge.rs"
    And after the "sed" the client's document notifications were "none"
    And after the "sed" the symbols of "src/charge.rs" include "STAGE_OPENED_MARKER"
    And after the "git checkout" the symbols of "src/charge.rs" do not include "STAGE_OPENED_MARKER"
    And the client sent no "workspace/didChangeWatchedFiles"
