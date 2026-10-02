@providers
Feature: The resolution summary and drop check after every codetags index
  Every `codetags index` run ends with one resolution line per language: its
  call sites, how many resolved, how many did not, and the unresolved rate
  (brief §4.4, "Validation output"; PLAN.md D33). It then runs the
  edge-count regression check of `codetags report` against the previous
  complete generation, and exits 1 on a sudden drop: a file that loses more
  than half of its edges, or all of them, is a provider failure, not a code
  change (brief §4.4). The generation is complete either way; the next run
  compares against it. Per-module detail stays in `codetags report`.

  Scenario: The first run reports its resolution, with nothing to compare against
    Given the scenario's directory holds a copy of the fixture "rust"
    When codetags is run in the scenario's directory with "index"
    Then it exits with status 0
    And stdout matches "(?m)^resolution rust: 22 call sites, 22 resolved, 0 unresolved \(unresolved rate 0\.0%\)$"
    And stdout matches "(?m)^edge counts: no previous generation to compare with$"

  Scenario: A second run over unchanged sources passes the drop check
    Given the scenario's directory holds a copy of the fixture "rust"
    When codetags is run in the scenario's directory with "index"
    And codetags is run in the scenario's directory with "index"
    Then it exits with status 0
    And stdout matches "generation 2[^0-9]"
    And stdout matches "(?m)^resolution rust: 22 call sites, 22 resolved, 0 unresolved \(unresolved rate 0\.0%\)$"
    And stdout matches "(?m)^edge counts: no regression against generation 1$"

  Scenario: A sudden drop in a file's edges fails the run
    Given the scenario's directory holds a copy of the fixture "rust"
    When codetags is run in the scenario's directory with "index"
    Then it exits with status 0
    Given the file "src/pipeline.rs" in the scenario's directory holds:
      """
      //! The pipeline, cut down to one function that calls nothing.

      use crate::charge::Apply;
      use crate::ledger::Ledger;

      /// Returns `total` unchanged.
      pub fn run(total: i64, _adjustments: &[Box<dyn Apply>], _ledger: &mut Ledger) -> i64 {
          total
      }
      """
    When codetags is run in the scenario's directory with "index"
    Then it exits with status 1
    And stdout matches "generation 2[^0-9]"
    And stdout matches "(?m)^resolution rust: 11 call sites, 11 resolved, 0 unresolved \(unresolved rate 0\.0%\)$"
    And stdout matches "against generation 1; likely a provider failure"
    And stdout matches "(?m)^  rust-analyzer scip src/pipeline\.rs: 11 -> 0 edges$"
    And stderr matches "an edge-count check failed"
