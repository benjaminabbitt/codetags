@providers
Feature: SCIP ingest into a generation
  The Rust provider's index of tests/fixtures/rust is ingested into a new
  generation of the index (PLAN.md §9 P1.3, brief §4.4 rules, PLAN.md §2.2,
  §2.4, §2.7). Lines are 1-based.

  Symbols are named by their canonical names (D15) in tables, and by their
  SCIP descriptors where a step says "descriptors". A call site's target is
  the canonical name of its declared target. A Rust name starts with its
  crate: the fixture's is `billing`, the standard library's `core` and
  `alloc` (P1.3c, a default pending human review).

  Policies, documented in codetags_ingest::ingest:
  - a reference is attributed to the innermost enclosing definition, using
    definitions' enclosing ranges only (V65); module definitions are not
    callers, so imports and re-exports are not call sites;
  - only references to callables (functions, methods, macros) are call
    sites, whether called or used as a value;
  - operator references (whose text is not a name, V65) are dropped;
  - references to external symbols are kept, and their symbols are marked
    external;
  - local symbols are dropped;
  - dispatch is static for a free function, an impl block's method or a
    macro, virtual for a trait method, and never dynamic from SCIP.

  Scenario: Project definitions become symbols with canonical names
    When the fixture "rust" is ingested into a new generation
    Then the generation's symbols include:
      | name                                 | descriptors                              | kind         | file          | lines | module         | external |
      | billing.settle                       | settle().                                | function     | src/lib.rs    | 17-24 |                | no       |
      | billing.charge                       | charge/                                  | module       | src/charge.rs | 1-45  |                | no       |
      | billing.charge.double                | charge/double().                         | function     | src/charge.rs | 41-44 | billing.charge | no       |
      | billing.charge.Apply.apply           | charge/Apply#apply().                    | trait_method | src/charge.rs | 5-6   | billing.charge | no       |
      | billing.charge.Charge<T>.Apply.apply | charge/impl#[`Charge<T>`][Apply]apply(). | method       | src/charge.rs | 36-38 | billing.charge | no       |
      | billing.charge.Fee.Apply.apply       | charge/impl#[Fee][Apply]apply().         | method       | src/charge.rs | 24-26 | billing.charge | no       |
      | billing.ledger.Ledger.record         | ledger/impl#[Ledger]record().            | method       | src/ledger.rs | 11-14 | billing.ledger | no       |
      | billing.macros.log_line              | macros/log_line!                         | macro        | src/macros.rs | 3-8   | billing.macros | no       |
    And the symbols defined in "src/pipeline.rs" are exactly:
      | name                         |
      | billing.pipeline             |
      | billing.pipeline.run         |
      | billing.pipeline.apply_dyn   |
      | billing.pipeline.transform   |
      | billing.pipeline.doubled_all |
    And the ingest reports no canonical-name collisions

  # P1.3c (dogfooding); defaults pending human review.
  Scenario: Impl blocks are not symbols, and a value named like another symbol is suffixed
    When the fixture "rust" is ingested into a new generation
    Then the symbols defined in "src/ledger.rs" are exactly:
      | name                                |
      | billing.ledger                      |
      | billing.ledger.Ledger               |
      | billing.ledger.Ledger.entries+field |
      | billing.ledger.Ledger.entries       |
      | billing.ledger.Ledger.record        |
      | billing.ledger.Ledger.line          |
      | billing.ledger.Ledger.with          |
      | billing.ledger.totals               |
      | billing.ledger.totals.MAX           |
      | billing.ledger.totals+fn            |
    And the generation's symbols include:
      | name                                | descriptors                    | kind     | file          | lines | module                |
      | billing.ledger.Ledger.entries+field | ledger/Ledger#entries.         | field    | src/ledger.rs | 6-7   | billing.ledger        |
      | billing.ledger.Ledger.entries       | ledger/impl#[Ledger]entries(). | method   | src/ledger.rs | 26-29 | billing.ledger        |
      | billing.charge.Charge.amount        | charge/Charge#amount.          | field    | src/charge.rs | 19-20 | billing.charge        |
      | billing.ledger.totals               | ledger/totals/                 | module   | src/ledger.rs | 37-42 | billing.ledger        |
      | billing.ledger.totals+fn            | ledger/totals().               | function | src/ledger.rs | 44-47 | billing.ledger        |
      | billing.ledger.totals.MAX           | ledger/totals/MAX.             | constant | src/ledger.rs | 40-41 | billing.ledger.totals |
    And the symbols defined in "src/charge.rs" are exactly:
      | name                                 |
      | billing.charge                       |
      | billing.charge.Apply                 |
      | billing.charge.Apply.apply           |
      | billing.charge.Fee                   |
      | billing.charge.Fee.0                 |
      | billing.charge.Discount              |
      | billing.charge.Discount.0            |
      | billing.charge.Charge                |
      | billing.charge.Charge.payload        |
      | billing.charge.Charge.amount         |
      | billing.charge.Fee.Apply.apply       |
      | billing.charge.Discount.Apply.apply  |
      | billing.charge.Charge<T>.Apply.apply |
      | billing.charge.double                |
    And the ingest reports no canonical-name collisions

  Scenario: Targets without a definition in the project are symbols too
    When the fixture "rust" is ingested into a new generation
    Then the generation's symbols include:
      | name                                  | descriptors                             | kind   | file | lines | module         | external |
      | alloc.boxed.Box<T>.new                | boxed/impl#[`Box<T>`]new().             | method |      |       | alloc.boxed    | yes      |
      | alloc.macros.vec                      | macros/vec!                             | macro  |      |       | alloc.macros   | yes      |
      | billing.ledger.Ledger.Default.default | ledger/impl#[Ledger][Default]default(). | method |      |       | billing.ledger | no       |

  Scenario: Calls are attributed to the innermost enclosing definition
    When the fixture "rust" is ingested into a new generation
    Then the call sites in "src/lib.rs" on line 21 are:
      | caller         | target               | kind | dispatch | external |
      | billing.settle | billing.pipeline.run | call | static   | no       |
    And the call sites in "src/pipeline.rs" on line 10 are:
      | caller               | target                                | kind | dispatch | external |
      | billing.pipeline.run | billing.pipeline.apply_dyn            | call | static   | no       |
      | billing.pipeline.run | alloc.boxed.Box<T, A>.AsRef<T>.as_ref | call | static   | yes      |
    And the call sites in "src/pipeline.rs" on line 11 are:
      | caller               | target                       | kind | dispatch | external |
      | billing.pipeline.run | billing.ledger.Ledger.record | call | static   | no       |
    And the call sites in "src/pipeline.rs" on line 15 are:
      | caller               | target                     | kind | dispatch | external |
      | billing.pipeline.run | billing.pipeline.transform | call | static   | no       |

  Scenario: Trait calls resolve to the trait method and are virtual
    When the fixture "rust" is ingested into a new generation
    Then the call sites in "src/pipeline.rs" on line 20 are:
      | caller                     | target                     | kind | dispatch | external |
      | billing.pipeline.apply_dyn | billing.charge.Apply.apply | call | virtual  | no       |

  Scenario: Functions used as values are call sites of kind value
    When the fixture "rust" is ingested into a new generation
    Then the call sites in "src/pipeline.rs" on line 13 are:
      | caller               | target                | kind  | dispatch | external |
      | billing.pipeline.run | billing.charge.double | value | static   | no       |
    And the call sites in "src/pipeline.rs" on line 30 are:
      | caller                       | target                                     | kind  | dispatch | external |
      | billing.pipeline.doubled_all | core.slice.[T].iter                        | call  | static   | yes      |
      | billing.pipeline.doubled_all | core.iter.traits.iterator.Iterator.copied  | call  | virtual  | yes      |
      | billing.pipeline.doubled_all | core.iter.traits.iterator.Iterator.map     | call  | virtual  | yes      |
      | billing.pipeline.doubled_all | billing.charge.double                      | value | static   | no       |
      | billing.pipeline.doubled_all | core.iter.traits.iterator.Iterator.collect | call  | virtual  | yes      |

  Scenario: Macro invocations are call sites; the calls inside them are lost
    When the fixture "rust" is ingested into a new generation
    Then the call sites in "src/lib.rs" on line 22 are:
      | caller         | target                  | kind  | dispatch | external |
      | billing.settle | billing.macros.log_line | macro | static   | no       |
    And the call sites in "src/lib.rs" on line 19 are:
      | caller         | target                 | kind  | dispatch | external |
      | billing.settle | alloc.macros.vec       | macro | static   | yes      |
      | billing.settle | alloc.boxed.Box<T>.new | call  | static   | yes      |
      | billing.settle | alloc.boxed.Box<T>.new | call  | static   | yes      |

  Scenario: Locals, operators, imports and non-callables are not call sites
    When the fixture "rust" is ingested into a new generation
    Then there are no call sites in "src/pipeline.rs" on line 3
    And there are no call sites in "src/pipeline.rs" on line 14
    And there are no call sites in "src/pipeline.rs" on line 25
    And there are no call sites in "src/charge.rs" on line 25
    And there are no call sites in "src/charge.rs" on line 43
    And no symbol or call site in the generation comes from a local symbol

  Scenario: Every call site has its declared target and its provenance
    When the fixture "rust" is ingested into a new generation
    Then every call site has exactly one call target, its declared target, with the method "declared"
    And every call site's source matches "^scip@rust-analyzer-[0-9]"
    And the generation holds one succeeded run of "rust-analyzer scip"
    And the run's edge counts are:
      | file            | edges |
      | src/charge.rs   | 0     |
      | src/ledger.rs   | 5     |
      | src/lib.rs      | 6     |
      | src/macros.rs   | 0     |
      | src/pipeline.rs | 11    |

  # Exact counts on the pinned rust-analyzer: an upgrade that changes any of
  # them must be looked at (brief §4.4, edge counts).
  Scenario: The ingest report counts what it wrote and what it skipped
    When the fixture "rust" is ingested into a new generation
    Then the ingest report counts:
      | what                            | count |
      | files                           | 5     |
      | symbols                         | 46    |
      | call sites                      | 22    |
      | local occurrences               | 72    |
      | operator references             | 18    |
      | references outside a definition | 1     |

  Scenario: Re-indexing writes generation N+1 while N stays readable
    Given the fixture "rust" has been ingested into a new generation
    And a reader has opened the newest complete generation
    When the fixture "rust" is ingested into a new generation
    Then the index holds only the generations "1 2"
    And the reader reads the files "src/charge.rs src/ledger.rs src/lib.rs src/macros.rs src/pipeline.rs"
    When the reader switches to the newest complete generation
    Then the reader reads the files "src/charge.rs src/ledger.rs src/lib.rs src/macros.rs src/pipeline.rs"
    And the generation the reader held before the switch reads the files "src/charge.rs src/ledger.rs src/lib.rs src/macros.rs src/pipeline.rs"

  Scenario: codetags index indexes a Rust project end to end
    Given the scenario's directory holds a copy of the fixture "rust"
    When codetags is run in the scenario's directory with "index"
    Then it exits with status 0
    And stdout matches "generation 1[^0-9]"
    And stdout matches " 22 call sites"
    And the index holds only the generations "1"
    And the file ".codetags/.gitignore" in the scenario's directory holds the lines "index/ *.lock local/"
    When another process opens the newest complete generation and lists its files
    Then that process reads exactly "src/charge.rs src/ledger.rs src/lib.rs src/macros.rs src/pipeline.rs"

  Scenario: codetags index fails when no provider applies
    When codetags is run in the scenario's directory with "index"
    Then it exits with status 1
