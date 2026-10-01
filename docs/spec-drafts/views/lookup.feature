# DRAFT for human review (P2.1). Not run: drafts live outside features/.
#
# Questions for review:
#  1. Result groups at q/ itself (`q/@symbols`) do not exist: the empty
#     query has no result set, so the whole repo is never one directory.
#     Agreed? (tagma treats an empty query as an error.)
#  2. The unquoted `q/calls=billing.Charge<T>.apply` is ENOENT: tagma lexes
#     the first unquoted "<" as an operator, which leaves an invalid value
#     token (tagma SPEC §2). Should codetags instead accept it, or say why
#     in an error somewhere (e.g. a hint in README.md)?
#  3. "Valid prefix" here means: the elements so far parse and leave no
#     stack underflow. tagma's leftover-stack fold makes every such prefix
#     a query. A malformed atom (`kind=`), an unterminated quote and an
#     operator underflow are all ENOENT, not EINVAL. Agreed?
#  4. Operators are case-insensitive in tagma (`NOT` is `not`). The drafts
#     rely on that only implicitly; say if a scenario is wanted.

@query-tree
Feature: Lookup in q/ appends raw tagma postfix elements
  PLAN.md §2.7 (D15) and Appendix B. In a query directory, a name starting
  with "@" is a result group. Any other name is appended to the query, as
  typed, as the next postfix element: a valid prefix is a directory, and an
  invalid one does not exist. Values that need it are quoted the tagma way.
  Names inside a result group are result names, never query elements.

  Background:
    Given the view fixture "shop"

  Scenario Outline: A valid prefix is a directory
    Then "<path>" is a directory

    Examples: atoms and implicit AND
      | path                                  |
      | q/kind=method                         |
      | q/kind=method/lang=rust               |
      | q/owner                               |
      | q/prov:source=scip                    |
    Examples: operators, relations and patterns
      | path                                  |
      | q/kind=method/not                     |
      | q/kind=method/module=billing/not/and  |
      | q/kind=callsite/line>60               |
      | q/dispatch!=static                    |
      | q/file~src.billing.charge.rs          |
    Examples: quoted values
      | path                                  |
      | q/calls="billing.Charge<T>.apply"     |
      | q/target="billing.Charge<T>.apply"    |

  Scenario Outline: An invalid prefix does not exist
    Then "<path>" does not exist

    Examples: operator underflow
      | path                                |
      | q/or                                |
      | q/kind=method/and                   |
    Examples: malformed elements
      | path                                |
      | q/kind=                             |
      | q/"unterminated                     |
      | q/calls=billing.Charge<T>.apply     |
    Examples: "@" names that are not result groups
      | path                                |
      | q/@symbols                          |
      | q/kind=method/@                     |
      | q/kind=method/@bogus                |

  Scenario: Elements fold together with AND
    Then listing "q/kind=method/lang=rust/@symbols" gives exactly "billing.Charge<T>.apply.sym notify.Mailer.send.sym ordering.OrderRepo.save.sym ordering.OrderService.place.sym ordering.PgOrderRepo.save.sym"

  Scenario: Explicit operators
    Then listing "q/kind=method/module=billing/not/and/@symbols" gives exactly "notify.Mailer.send.sym ordering.OrderRepo.save.sym ordering.OrderService.place.sym ordering.PgOrderRepo.save.sym"

  Scenario: A relational element
    Then listing "q/kind=callsite/line>60/@sites" gives exactly "4413.site 4414.site 4415.site"

  Scenario: A path pattern matches a file location
    Then listing "q/file~src.billing.charge.rs/@symbols" gives exactly "billing.Charge.sym billing.Charge<T>.apply.sym"
    And listing "q/file~src.billing.charge.rs/@sites" gives exactly "4417.site"

  Scenario: A quoted value holds tagma syntax
    Then listing 'q/calls="billing.Charge<T>.apply"/@symbols' gives exactly "ordering.OrderService.place.sym"
    And listing 'q/target="billing.Charge<T>.apply"/@sites' gives exactly "4413.site 4414.site"

  Scenario: User tags and derived facets mix in one query
    Then listing "q/owner=orders/kind=file/@files" gives exactly "src/"
    And listing "q/owner=orders/kind=file/@files/src" gives exactly "ordering/"

  Scenario: A name inside a result group is never a query element
    Then "q/kind=method/@symbols/lang=rust" does not exist
    And "q/kind=method/@symbols/and" does not exist
    And "q/kind=method/@views/lang=rust" does not exist
    And "q/kind=method/@symbols/ordering.OrderService.place.sym" is a file
