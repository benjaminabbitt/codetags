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

  # D31: rust-analyzer writes no is_implementation relationships (V64). After
  # the SCIP ingest, `codetags index` starts the same rust-analyzer as an LSP
  # server on the project, waits until it reports the workspace loaded
  # (`experimental/serverStatus`, quiescent), and asks
  # `textDocument/implementation` at the definition of each of the project's
  # trait methods that a call site targets. Each implementation becomes a
  # call target of every call to the trait method, with the method
  # `lsp-impl`; candidates is how many targets of its method the call site
  # has. The pass is its own run, `rust-analyzer lsp-impl`, whose edges are
  # its call targets per file, so the edge-count checks cover it.
  Rule: Trait and dyn calls reach their implementations through rust-analyzer

    Scenario: A dyn call reaches every implementation of the trait method
      Given the scenario's directory holds a copy of the fixture "rust"
      When codetags is run in the scenario's directory with "index"
      Then it exits with status 0
      And the call targets of the call to "billing.charge.Apply.apply" in "src/pipeline.rs" on line 20 are:
        | target                               | method   | candidates |
        | billing.charge.Apply.apply           | declared | 1          |
        | billing.charge.Fee.Apply.apply       | lsp-impl | 3          |
        | billing.charge.Discount.Apply.apply  | lsp-impl | 3          |
        | billing.charge.Charge<T>.Apply.apply | lsp-impl | 3          |
      And the call targets of the call to "billing.pipeline.apply_dyn" in "src/pipeline.rs" on line 10 are:
        | target                     | method   | candidates |
        | billing.pipeline.apply_dyn | declared | 1          |
      And the edge counts of the run of "rust-analyzer lsp-impl" are:
        | file            | edges |
        | src/charge.rs   | 0     |
        | src/ledger.rs   | 0     |
        | src/lib.rs      | 0     |
        | src/macros.rs   | 0     |
        | src/pipeline.rs | 3     |
      And stdout matches "(?m)^rust-analyzer lsp-impl: 1 called trait method, 3 implementations, 3 call targets on 1 call site$"

    Scenario: A trait with one implementation gives one candidate, and a standard library trait is not expanded
      Given the file "Cargo.toml" in the scenario's directory holds:
        """
        [package]
        name = "single"
        version = "0.1.0"
        edition = "2021"
        """
      And the file "src/lib.rs" in the scenario's directory holds:
        """
        //! A trait with one implementation.

        /// Something that speaks.
        pub trait Speak {
            /// Its words.
            fn speak(&self) -> String;
        }

        /// The only speaker.
        pub struct Dog;

        impl Speak for Dog {
            fn speak(&self) -> String {
                "woof".to_string()
            }
        }

        /// Speaks through the trait.
        pub fn talk(speaker: &dyn Speak) -> String {
            speaker.speak()
        }

        /// Copies through a trait of the standard library.
        pub fn copy<T: Clone>(value: &T) -> T {
            value.clone()
        }
        """
      When codetags is run in the scenario's directory with "index"
      Then it exits with status 0
      And the call targets of the call to "single.Speak.speak" in "src/lib.rs" on line 20 are:
        | target                 | method   | candidates |
        | single.Speak.speak     | declared | 1          |
        | single.Dog.Speak.speak | lsp-impl | 1          |
      And the call targets of the call to "core.clone.Clone.clone" in "src/lib.rs" on line 25 are:
        | target                 | method   | candidates |
        | core.clone.Clone.clone | declared | 1          |
      And stdout matches "(?m)^rust-analyzer lsp-impl: 1 called trait method, 1 implementation, 1 call target on 1 call site$"

    Scenario: The run fails loudly when rust-analyzer does not load the workspace in time
      Given the scenario's directory holds a copy of the fixture "rust"
      And the environment variable "CODETAGS_RUST_ANALYZER_LOAD_TIMEOUT" is "0"
      When codetags is run in the scenario's directory with "index"
      Then it exits with status 1
      And stderr matches "rust-analyzer lsp-impl: rust-analyzer did not finish loading the workspace within 0 s"
