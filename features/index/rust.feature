@providers
Feature: Rust provider (rust-analyzer scip)
  The pinned rust-analyzer indexes tests/fixtures/rust into a SCIP index
  (PLAN.md §9 P1.4, brief §4.4 Rust row). Every provider failure is loud.
  Line numbers here are 1-based.

  Scenario: Every source file becomes a document
    When the Rust provider indexes the fixture "rust"
    Then the provider run succeeds
    And the index documents are exactly "src/charge.rs src/ledger.rs src/lib.rs src/macros.rs src/pipeline.rs"

  Scenario: Definitions carry enclosing ranges
    When the Rust provider indexes the fixture "rust"
    Then every definition of a function or method has an enclosing range
    And the definition of "pipeline/run()." encloses lines 6 to 16
    And the definition of "charge/impl#[`Charge<T>`][Apply]apply()." encloses lines 36 to 38
    And the definition of "ledger/impl#[Ledger]record()." encloses lines 11 to 14

  Scenario: Calls, dyn calls, functions used as values, and macros occur where written
    When the Rust provider indexes the fixture "rust"
    Then "pipeline/apply_dyn()." occurs in "src/pipeline.rs" on line 10
    And "charge/Apply#apply()." occurs in "src/pipeline.rs" on line 20
    And "charge/double()." occurs in "src/pipeline.rs" on line 13
    And "charge/double()." occurs in "src/pipeline.rs" on line 30
    And "macros/log_line!" occurs in "src/lib.rs" on line 22
    And "pipeline/run()." occurs in "src/lib.rs" on line 21

  Scenario: Each implementation of a trait method is its own definition
    When the Rust provider indexes the fixture "rust"
    Then the definition of "charge/Apply#apply()." encloses lines 5 to 6
    And the definition of "charge/impl#[Fee][Apply]apply()." encloses lines 24 to 26
    And the definition of "charge/impl#[Discount][Apply]apply()." encloses lines 30 to 32

  # V64: rust-analyzer writes no relationships at all, so trait and dyn calls
  # cannot be expanded through is_implementation. This scenario fails when an
  # upgrade starts writing them, so the expansion is revisited then.
  Scenario: The pinned rust-analyzer writes no relationships
    When the Rust provider indexes the fixture "rust"
    Then no symbol in the index has a relationship

  Scenario: Local symbols can be filtered out
    When the Rust provider indexes the fixture "rust"
    Then the document "src/pipeline.rs" has occurrences of local symbols
    And filtering local symbols keeps every other occurrence in "src/pipeline.rs"

  Scenario: A root whose Cargo.toml does not parse fails loudly
    When the Rust provider indexes a directory whose Cargo.toml is "not toml [[["
    Then the provider run fails with stderr matching "Failed to load the project"

  Scenario: A root that does not exist fails loudly
    When the Rust provider indexes a directory that does not exist
    Then the provider run fails with stderr matching "Error:"
