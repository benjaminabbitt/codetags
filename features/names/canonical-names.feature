Feature: Canonical symbol names are dotted and keep their real characters
  A symbol's canonical name is the dotted path of its SCIP descriptors,
  without the package (PLAN.md §2.7, D15). Separators and Rust `::` become
  `.`, a package path's `/` becomes `.`, and an overload disambiguator
  `(+N)` becomes `+N`. Nothing else is escaped. The exact SCIP symbol stays
  in DuckDB, so a canonical name only has to be unique.

  The symbols below were emitted by rust-analyzer 1.96.1, scip-go 0.2.7,
  scip-typescript 0.4.0 and scip-python 0.6.6 for small demo projects (V38),
  except the overload row, which follows SCIP's documented `(+N)` form.

  Scenario Outline: Each indexer's symbols have readable canonical names
    When the canonical name of the SCIP symbol "<symbol>" is taken
    Then the canonical name is "<canonical>"

    Examples: rust-analyzer
      | symbol                                                                              | canonical                         |
      | rust-analyzer cargo demo 0.1.0 billing/Charge#                                      | billing.Charge                    |
      | rust-analyzer cargo demo 0.1.0 billing/Charge#amount.                               | billing.Charge.amount             |
      | rust-analyzer cargo demo 0.1.0 billing/Apply#apply().                               | billing.Apply.apply               |
      | rust-analyzer cargo demo 0.1.0 billing/impl#[`Charge<T>`]new().                     | billing.Charge<T>.new             |
      | rust-analyzer cargo demo 0.1.0 billing/impl#[`Charge<T>`][Apply]apply().            | billing.Charge<T>.Apply.apply     |
      | rust-analyzer cargo demo 0.1.0 billing/impl#[`Charge<u8>`][`From<u8>`]from().       | billing.Charge<u8>.From<u8>.from  |
      | rust-analyzer cargo demo 0.1.0 billing/impl#[`inner::Fee`][Apply]apply().           | billing.inner.Fee.Apply.apply     |
      | rust-analyzer cargo demo 0.1.0 billing/inner/Fee#                                   | billing.inner.Fee                 |
      | rust-analyzer cargo demo 0.1.0 charge!                                              | charge                            |
      | rust-analyzer cargo demo 0.1.0 `r#type`().                                          | r#type                            |

    Examples: scip-go
      | symbol                                                                                    | canonical                                  |
      | scip-go gomod github.com/acme/billing . `github.com/acme/billing/billing`/                | github.com.acme.billing.billing            |
      | scip-go gomod github.com/acme/billing . `github.com/acme/billing/billing`/Charge#         | github.com.acme.billing.billing.Charge     |
      | scip-go gomod github.com/acme/billing . `github.com/acme/billing/billing`/Charge#Apply(). | github.com.acme.billing.billing.Charge.Apply |
      | scip-go gomod github.com/acme/billing . `github.com/acme/billing/billing`/Applier#Apply.  | github.com.acme.billing.billing.Applier.Apply |
      | scip-go gomod github.com/acme/billing . `github.com/acme/billing/billing`/New().          | github.com.acme.billing.billing.New        |

    Examples: scip-typescript
      | symbol                                                                         | canonical                         |
      | scip-typescript npm @acme/billing 1.0.0 src/`billing.ts`/Charge#               | src.billing.ts.Charge             |
      | scip-typescript npm @acme/billing 1.0.0 src/`billing.ts`/Charge#[T]            | src.billing.ts.Charge.T           |
      | scip-typescript npm @acme/billing 1.0.0 src/`billing.ts`/Charge#`<constructor>`(). | src.billing.ts.Charge.<constructor> |
      | scip-typescript npm @acme/billing 1.0.0 src/`billing.ts`/$make().              | src.billing.ts.$make              |
      | scip-typescript npm @acme/billing 1.0.0 src/`billing.ts`/Charge#apply2(+1).    | src.billing.ts.Charge.apply2+1    |

    Examples: scip-python
      | symbol                                                                         | canonical                          |
      | scip-python python acme-billing 0.1.0 `acme_billing.charge`/Charge#apply().    | acme_billing.charge.Charge.apply   |
      | scip-python python acme-billing 0.1.0 `acme_billing.charge`/Charge#__init__(). | acme_billing.charge.Charge.__init__ |
      | scip-python python acme-billing 0.1.0 `acme_billing.charge`/__init__:          | acme_billing.charge.__init__       |
      | scip-python python acme-billing 0.1.0 acme_billing/__init__:                   | acme_billing.__init__              |

  Scenario Outline: Locals and parameters have no canonical name
    When the canonical name of the SCIP symbol "<symbol>" is taken
    Then the symbol has no canonical name

    Examples:
      | symbol                                                                            |
      | local 0                                                                           |
      | scip-python python acme-billing 0.1.0 `acme_billing.charge`/make().(n)            |
      | scip-typescript npm @acme/billing 1.0.0 src/`billing.ts`/Charge#`<constructor>`().(amount) |

  Scenario: Two symbols with one canonical name are reported, never merged
    Given these SCIP symbols:
      | symbol                                                     |
      | rust-analyzer cargo demo 0.1.0 charge!                     |
      | rust-analyzer cargo demo 0.1.0 charge().                   |
      | rust-analyzer cargo demo 0.1.0 billing/Charge#             |
      | rust-analyzer cargo other 0.2.0 billing/Charge#            |
      | rust-analyzer cargo demo 0.1.0 billing/Apply#apply().      |
    When their canonical names are assigned
    Then the collisions are:
      | name           | symbol                                          |
      | charge         | rust-analyzer cargo demo 0.1.0 charge!          |
      | charge         | rust-analyzer cargo demo 0.1.0 charge().        |
      | billing.Charge | rust-analyzer cargo demo 0.1.0 billing/Charge#  |
      | billing.Charge | rust-analyzer cargo other 0.2.0 billing/Charge# |

  Scenario: The same symbol twice is not a collision
    Given these SCIP symbols:
      | symbol                                         |
      | rust-analyzer cargo demo 0.1.0 billing/Charge# |
      | rust-analyzer cargo demo 0.1.0 billing/Charge# |
    When their canonical names are assigned
    Then there are no collisions
