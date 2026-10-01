# DRAFT for human review (P2.1). Not run: drafts live outside features/.
#
# Questions for review:
#  1. A query whose result set has items of a kind lists only that kind's
#     group, so `q/kind=method` lists @symbols/ and @views/ but not @files/,
#     although every method lives in a file. Is that right, or should a
#     symbol's file also appear under @files/?
#  2. @views/ is listed whenever the result set is non-empty and some plugin
#     mounts in result sets (by default only mermaid). With no such plugin it
#     is never listed. Agreed?
#  3. Listings in result sets have no cap (O-8 default "no cap"). Agreed for
#     v1?

@query-tree
Feature: Traversal of q/ is finite
  Brief §4.5 and PLAN.md Appendix B: listing a query directory returns only
  its non-empty result groups (@files/, @symbols/, @sites/, @views/). Facet
  and operator names resolve on lookup but are never listed, so `find`,
  `grep -r` and `rg` terminate. q/ itself is the empty query and lists
  nothing.

  Background:
    Given the view fixture "shop"

  Scenario: q/ lists nothing
    Then listing "q" gives exactly ""

  Scenario: A query directory lists only its result groups
    Then listing "q/kind=method" gives exactly "@symbols/ @views/"
    And listing "q/kind=callsite" gives exactly "@sites/ @views/"

  Scenario: Facets and operators resolve but are never listed
    Then "q/kind=method/lang=rust" is a directory
    And "q/kind=method/not" is a directory
    And listing "q/kind=method" gives exactly "@symbols/ @views/"

  Scenario: Groups with no results are neither listed nor found
    Then listing "q/kind=callsite" gives exactly "@sites/ @views/"
    And "q/kind=callsite/@symbols" does not exist
    And "q/kind=callsite/@files" does not exist

  Scenario: A valid query with no results is an empty directory
    Then "q/kind=interface" is a directory
    And listing "q/kind=interface" gives exactly ""
    And "q/kind=interface/@views" does not exist

  Scenario: A recursive walk of a query directory ends at its results
    When "q/kind=method" is walked recursively
    Then the walk visits exactly:
      """
      @symbols/
      @symbols/billing.Charge<T>.apply.sym
      @symbols/notify.Mailer.send.sym
      @symbols/ordering.OrderRepo.save.sym
      @symbols/ordering.OrderService.place.sym
      @symbols/ordering.PgOrderRepo.save.sym
      @views/
      @views/callgraph.md
      @views/callgraph.mmd
      """

  Scenario: A recursive walk of the whole tree ends, and finds q/ empty
    When "" is walked recursively
    Then the walk visits exactly:
      """
      FACETS.md
      README.md
      arch/
      arch/api.md
      arch/api.mmd
      arch/billing.md
      arch/billing.mmd
      arch/modules.md
      arch/modules.mmd
      arch/notify.md
      arch/notify.mmd
      arch/ordering.md
      arch/ordering.mmd
      files/
      files/src/
      files/src/api/
      files/src/api/routes.rs.skel
      files/src/billing/
      files/src/billing/charge.rs.skel
      files/src/billing/order.rs.skel
      files/src/notify/
      files/src/notify/mail.rs.skel
      files/src/ordering/
      files/src/ordering/repo.rs.skel
      files/src/ordering/service.rs.skel
      modules.dot
      orphans.txt
      q/
      unresolved/
      unresolved/api.txt
      unresolved/billing.txt
      unresolved/notify.txt
      unresolved/ordering.txt
      """
