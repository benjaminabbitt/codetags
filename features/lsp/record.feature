Feature: The LSP recorder passes a session through and logs it
  `codetags-lsp record` (M3 stage 0, PLAN.md D18) sits between an editor and
  a real language server. It passes every byte through unchanged in both
  directions, appends one JSON line per message to its log, and exits with
  the server's status, so the editor cannot tell it is there. Invocations
  that are not LSP sessions, such as `--version`, go straight to the real
  server (brief §4.1). The real server here is a fake one, so the feature
  runs on every OS.

  Scenario: Every byte passes through unchanged in both directions
    Given a fake language server
    When a scripted LSP session runs through codetags-lsp record
    Then the fake server received exactly the bytes the client sent
    And the client received exactly the bytes the fake server sent
    And it exits with status 0

  Scenario: Each message is recorded once, with its direction
    Given a fake language server
    When a scripted LSP session runs through codetags-lsp record
    Then the recording holds 9 messages
    And the recording shows "initialize" sent by the client
    And the recording shows "response 1" sent by the server
    And the recording shows "window/logMessage" sent by the server
    And the recording shows "client/registerCapability" sent by the server
    And the recording shows "response 900" sent by the client
    And the recording shows "exit" sent by the client

  Scenario: The server's exit status is the recorder's exit status
    Given a fake language server that exits with status 3
    When a scripted LSP session runs through codetags-lsp record
    Then it exits with status 3
    And the recording shows "shutdown" sent by the client

  Scenario: --version goes straight to the real server and is not recorded
    Given a fake language server
    When codetags-lsp record runs the fake server with the arguments "--version"
    Then it exits with status 0
    And stdout matches "^fake-lsp 9\.9\.9\n$"
    And no recording was written

  Scenario: A non-LSP invocation keeps the real server's exit status
    Given a fake language server that exits with status 4
    When codetags-lsp record runs the fake server with the arguments "--version"
    Then it exits with status 4
    And no recording was written
