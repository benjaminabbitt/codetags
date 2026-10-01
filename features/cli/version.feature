Feature: The CLI reports its version
  The codetags CLI is the unprivileged baseline on every OS (PLAN.md D3),
  so the simplest thing it does must work everywhere.

  Scenario: --version prints the program name and a SemVer version
    When codetags is run with "--version"
    Then it exits with status 0
    And stdout matches "^codetags \d+\.\d+\.\d+\n$"
