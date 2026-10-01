# DRAFT for human review (P2.1). Not run: drafts live outside features/.
#
# Questions for review:
#  1. D11 makes some operations in q/ succeed: writes to @files/<path>, and
#     create, mv, rm and mkdir as tag operations (PLAN.md §2.10). Those
#     belong to P6.1 and are left out here. Every operation below is refused
#     under D11 as well.
#  2. "Refused" accepts EROFS, EACCES or EPERM, because the static tree is
#     read-only by permissions (chmod a-w gives EACCES) while live mounts
#     report EROFS. Should the contract require one errno per backend?

Feature: Derived views are read-only
  Brief §4.5: views use non-source extensions and are read-only, since
  weaker models sometimes try to edit code shown to them. Derived views
  (.skel, cards, diagrams, README, FACETS) stay read-only under every
  decision so far, D11 included.

  Background:
    Given the view fixture "shop"

  Scenario Outline: Writing to a root view is refused
    When "<path>" is written with "edited"
    Then the operation is refused as read-only

    Examples:
      | path                               |
      | README.md                          |
      | files/src/ordering/service.rs.skel |
      | modules.dot                        |
      | arch/modules.mmd                   |

  Scenario: Creating a file in the tree is refused
    When the file "notes.txt" is created
    Then the operation is refused as read-only

  Scenario: Deleting a view is refused
    When "orphans.txt" is deleted
    Then the operation is refused as read-only

  Scenario: Creating a directory in the tree is refused
    When the directory "files/src/new" is created
    Then the operation is refused as read-only

  Scenario: Renaming a view is refused
    When "arch/modules.mmd" is renamed to "arch/renamed.mmd"
    Then the operation is refused as read-only

  @query-tree
  Scenario Outline: Result-set views are read-only
    When "<path>" is written with "edited"
    Then the operation is refused as read-only

    Examples:
      | path                                                   |
      | q/kind=method/@symbols/ordering.OrderService.place.sym |
      | q/dispatch=virtual/@sites/4412.site                    |
      | q/kind=method/@views/callgraph.mmd                     |

  @query-tree
  Scenario: Creating a file in a result group other than @files/ is refused
    When the file "q/kind=method/@symbols/new.sym" is created
    Then the operation is refused as read-only

  @query-tree
  Scenario: Deleting a result-set view is refused
    When "q/kind=method/@views/callgraph.mmd" is deleted
    Then the operation is refused as read-only
