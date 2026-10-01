@providers
Feature: Resolution report and edge-count regression
  `codetags report` reads a complete generation and reports its call sites
  per language, and per module of each package: how many resolved, how many did not, and the
  unresolved rate (brief §4.4, "Validation output"; PLAN.md §9 P1.8).

  A call site is unresolved when it has no call target other than name-match
  candidates (codetags_ingest::report). Every Rust call site has its declared
  target, so the Rust fixture has none.

  It then compares the generation's per-file edge counts (`run_file`) with
  the previous complete generation's. A sudden drop is a provider failure,
  not a code change (brief §4.4): a file that loses more than half of its
  edges, or drops to zero from non-zero, fails the report. With --baseline
  it also compares them with a committed baseline, and any difference fails.

  Scenario: The report counts call sites per language, package and module
    Given the fixture "rust" has been ingested into a new generation
    When codetags is run in the scenario's directory with "report --json"
    Then it exits with status 0
    And the JSON resolution report counts:
      | scope    | language | package | module           | call sites | resolved | unresolved |
      | module   | rust     | billing |                  | 6          | 6        | 0          |
      | module   | rust     | billing | billing.ledger   | 5          | 5        | 0          |
      | module   | rust     | billing | billing.pipeline | 11         | 11       | 0          |
      | language | rust     |         |                  | 22         | 22       | 0          |
      | total    |          |         |                  | 22         | 22       | 0          |

  Scenario: The human-readable report shows unresolved rates and totals
    Given the fixture "rust" has been ingested into a new generation
    When codetags is run in the scenario's directory with "report"
    Then it exits with status 0
    And stdout matches "(?m)^rust +billing +billing\.pipeline +11 +11 +0 +0\.0%$"
    And stdout matches "(?m)^rust +billing +\(root\) +6 +6 +0 +0\.0%$"
    And stdout matches "(?m)^total +22 +22 +0 +0\.0%$"
    And stdout matches "no previous generation"

  Scenario: Unchanged edge counts pass the previous-generation check
    Given the fixture "rust" has been ingested into a new generation
    And the fixture "rust" has been ingested into a new generation
    When codetags is run in the scenario's directory with "report"
    Then it exits with status 0
    And stdout matches "generation 2"
    And stdout matches "edge counts: no regression against generation 1"

  Scenario: A file whose edges drop to zero fails as a provider failure
    Given the fixture "rust" has been ingested into a new generation
    And the fixture "rust" has been ingested into a new generation
    And the edge count of "src/pipeline.rs" in generation 2 is changed to 0
    When codetags is run in the scenario's directory with "report"
    Then it exits with status 1
    And stdout matches "src/pipeline\.rs: 11 -> 0 edges"
    And stdout matches "likely a provider failure"

  Scenario Outline: A file that loses more than half of its edges fails
    Given the fixture "rust" has been ingested into a new generation
    And the fixture "rust" has been ingested into a new generation
    And the edge count of "<file>" in generation 2 is changed to <edges>
    When codetags is run in the scenario's directory with "report"
    Then it exits with status <status>

    Examples:
      | file            | edges | status |
      | src/ledger.rs   | 3     | 0      |
      | src/lib.rs      | 3     | 0      |
      | src/ledger.rs   | 2     | 1      |
      | src/pipeline.rs | 5     | 1      |
      | src/ledger.rs   | 9     | 0      |

  Scenario: An earlier generation is reported against its own predecessor
    Given the fixture "rust" has been ingested into a new generation
    And the fixture "rust" has been ingested into a new generation
    And the edge count of "src/pipeline.rs" in generation 2 is changed to 0
    When codetags is run in the scenario's directory with "report --generation 1"
    Then it exits with status 0
    And stdout matches "generation 1"
    And stdout matches "no previous generation"

  Scenario: A baseline that matches passes
    Given the fixture "rust" has been ingested into a new generation
    And the file "baseline.json" in the scenario's directory holds:
      """
      {
        "rust-analyzer scip": {
          "src/charge.rs": 0,
          "src/ledger.rs": 5,
          "src/lib.rs": 6,
          "src/macros.rs": 0,
          "src/pipeline.rs": 11
        }
      }
      """
    When codetags is run in the scenario's directory with "report --baseline baseline.json"
    Then it exits with status 0
    And stdout matches "baseline baseline\.json: matches"

  Scenario: A baseline mismatch fails loudly with a diff
    Given the fixture "rust" has been ingested into a new generation
    And the file "baseline.json" in the scenario's directory holds:
      """
      {
        "rust-analyzer scip": {
          "src/charge.rs": 0,
          "src/gone.rs": 4,
          "src/ledger.rs": 5,
          "src/lib.rs": 6,
          "src/pipeline.rs": 12
        }
      }
      """
    When codetags is run in the scenario's directory with "report --baseline baseline.json"
    Then it exits with status 1
    And stdout matches "baseline baseline\.json: 3 differences"
    And stdout matches "(?m)^- rust-analyzer scip src/pipeline\.rs 12$"
    And stdout matches "(?m)^\+ rust-analyzer scip src/pipeline\.rs 11$"
    And stdout matches "(?m)^- rust-analyzer scip src/gone\.rs 4$"
    And stdout matches "(?m)^\+ rust-analyzer scip src/macros\.rs 0$"

  Scenario: A baseline is written only on request
    Given the fixture "rust" has been ingested into a new generation
    When codetags is run in the scenario's directory with "report --write-baseline out.json"
    Then it exits with status 0
    And the file "out.json" in the scenario's directory holds the JSON:
      """
      {
        "rust-analyzer scip": {
          "src/charge.rs": 0,
          "src/ledger.rs": 5,
          "src/lib.rs": 6,
          "src/macros.rs": 0,
          "src/pipeline.rs": 11
        }
      }
      """

  Scenario: There is nothing to report without a generation
    When codetags is run in the scenario's directory with "report"
    Then it exits with status 1
