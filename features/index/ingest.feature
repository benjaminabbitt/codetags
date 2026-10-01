@providers
Feature: SCIP ingest into a generation
  The Rust provider's index of tests/fixtures/rust is ingested into a new
  generation of the index (PLAN.md §9 P1.3, brief §4.4 rules, PLAN.md §2.2,
  §2.4, §2.7). Lines are 1-based.

  Symbols are named by their canonical names (D15) in tables, and by their
  SCIP descriptors where a step says "descriptors". A call site's target is
  the canonical name of its declared target.

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
      | name                         | descriptors                              | kind         | file          | lines | module | external |
      | settle                       | settle().                                | function     | src/lib.rs    | 17-24 |        | no       |
      | charge                       | charge/                                  | module       | src/charge.rs | 1-45  |        | no       |
      | charge.double                | charge/double().                         | function     | src/charge.rs | 41-44 | charge | no       |
      | charge.Apply.apply           | charge/Apply#apply().                    | trait_method | src/charge.rs | 5-6   | charge | no       |
      | charge.Charge<T>.Apply.apply | charge/impl#[`Charge<T>`][Apply]apply(). | method       | src/charge.rs | 36-38 | charge | no       |
      | charge.Fee.Apply.apply       | charge/impl#[Fee][Apply]apply().         | method       | src/charge.rs | 24-26 | charge | no       |
      | ledger.Ledger.record         | ledger/impl#[Ledger]record().            | method       | src/ledger.rs | 11-14 | ledger | no       |
      | macros.log_line              | macros/log_line!                         | macro        | src/macros.rs | 3-8   | macros | no       |
    And the symbols defined in "src/pipeline.rs" are exactly:
      | name                 |
      | pipeline             |
      | pipeline.run         |
      | pipeline.apply_dyn   |
      | pipeline.transform   |
      | pipeline.doubled_all |
    And the ingest reports no canonical-name collisions

  # P1.3c (dogfooding); defaults pending human review.
  Scenario: Impl blocks are not symbols, and a field named like its getter is suffixed
    When the fixture "rust" is ingested into a new generation
    Then the symbols defined in "src/ledger.rs" are exactly:
      | name                        |
      | ledger                      |
      | ledger.Ledger               |
      | ledger.Ledger.entries+field |
      | ledger.Ledger.entries       |
      | ledger.Ledger.record        |
      | ledger.Ledger.line          |
      | ledger.Ledger.with          |
    And the generation's symbols include:
      | name                        | descriptors                        | kind   | file          | lines | module |
      | ledger.Ledger.entries+field | ledger/Ledger#entries.             | field  | src/ledger.rs | 6-7   | ledger |
      | ledger.Ledger.entries       | ledger/impl#[Ledger]entries().     | method | src/ledger.rs | 26-29 | ledger |
      | charge.Charge.amount        | charge/Charge#amount.              | field  | src/charge.rs | 19-20 | charge |
    And the symbols defined in "src/charge.rs" are exactly:
      | name                         |
      | charge                       |
      | charge.Apply                 |
      | charge.Apply.apply           |
      | charge.Fee                   |
      | charge.Fee.0                 |
      | charge.Discount              |
      | charge.Discount.0            |
      | charge.Charge                |
      | charge.Charge.payload        |
      | charge.Charge.amount         |
      | charge.Fee.Apply.apply       |
      | charge.Discount.Apply.apply  |
      | charge.Charge<T>.Apply.apply |
      | charge.double                |
    And the ingest reports no canonical-name collisions

  Scenario: Targets without a definition in the project are symbols too
    When the fixture "rust" is ingested into a new generation
    Then the generation's symbols include:
      | name                        | descriptors                                   | kind   | file | lines | module | external |
      | boxed.Box<T>.new            | boxed/impl#[`Box<T>`]new().                   | method |      |       | boxed  | yes      |
      | macros.vec                  | macros/vec!                                   | macro  |      |       | macros | yes      |
      | ledger.Ledger.Default.default | ledger/impl#[Ledger][Default]default().     | method |      |       | ledger | no       |

  Scenario: Calls are attributed to the innermost enclosing definition
    When the fixture "rust" is ingested into a new generation
    Then the call sites in "src/lib.rs" on line 21 are:
      | caller | target       | kind | dispatch | external |
      | settle | pipeline.run | call | static   | no       |
    And the call sites in "src/pipeline.rs" on line 10 are:
      | caller       | target                          | kind | dispatch | external |
      | pipeline.run | pipeline.apply_dyn              | call | static   | no       |
      | pipeline.run | boxed.Box<T, A>.AsRef<T>.as_ref | call | static   | yes      |
    And the call sites in "src/pipeline.rs" on line 11 are:
      | caller       | target               | kind | dispatch | external |
      | pipeline.run | ledger.Ledger.record | call | static   | no       |
    And the call sites in "src/pipeline.rs" on line 15 are:
      | caller       | target             | kind | dispatch | external |
      | pipeline.run | pipeline.transform | call | static   | no       |

  Scenario: Trait calls resolve to the trait method and are virtual
    When the fixture "rust" is ingested into a new generation
    Then the call sites in "src/pipeline.rs" on line 20 are:
      | caller             | target             | kind | dispatch | external |
      | pipeline.apply_dyn | charge.Apply.apply | call | virtual  | no       |

  Scenario: Functions used as values are call sites of kind value
    When the fixture "rust" is ingested into a new generation
    Then the call sites in "src/pipeline.rs" on line 13 are:
      | caller       | target        | kind  | dispatch | external |
      | pipeline.run | charge.double | value | static   | no       |
    And the call sites in "src/pipeline.rs" on line 30 are:
      | caller               | target                             | kind  | dispatch | external |
      | pipeline.doubled_all | slice.[T].iter                     | call  | static   | yes      |
      | pipeline.doubled_all | iter.traits.iterator.Iterator.copied  | call  | virtual  | yes      |
      | pipeline.doubled_all | iter.traits.iterator.Iterator.map     | call  | virtual  | yes      |
      | pipeline.doubled_all | charge.double                      | value | static   | no       |
      | pipeline.doubled_all | iter.traits.iterator.Iterator.collect | call  | virtual  | yes      |

  Scenario: Macro invocations are call sites; the calls inside them are lost
    When the fixture "rust" is ingested into a new generation
    Then the call sites in "src/lib.rs" on line 22 are:
      | caller | target          | kind  | dispatch | external |
      | settle | macros.log_line | macro | static   | no       |
    And the call sites in "src/lib.rs" on line 19 are:
      | caller | target           | kind  | dispatch | external |
      | settle | macros.vec       | macro | static   | yes      |
      | settle | boxed.Box<T>.new | call  | static   | yes      |
      | settle | boxed.Box<T>.new | call  | static   | yes      |

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
      | what                         | count |
      | files                        | 5     |
      | symbols                      | 43    |
      | call sites                   | 22    |
      | local occurrences            | 71    |
      | operator references          | 18    |
      | references outside a definition | 1  |

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
