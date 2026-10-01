# DRAFT for human review (P2.1). Not run: drafts live outside features/.
#
# Questions for review:
#  1. The brief's header names "<source path>". For views of one source file
#     (.skel, .sym, .site) that is the file. For aggregate views this draft
#     uses the repo name ("shop"), and for result-set views the repo name
#     plus the query path ("shop/q/kind=method"). Is that the wording you
#     want, and should aggregates keep "edit the source file"?
#  2. "<UTC time>" is spelled RFC 3339 to the second with "Z", taken from the
#     generation's newest run.finished_at. "<short sha>" is the first 7
#     characters of run.source_tree (a git tree SHA or a content hash).
#  3. Markdown files use an HTML comment, so the header doesn't render as a
#     heading. DOT uses "//". Formats with no comment syntax (JSON, CSV) are
#     handled in features/plugins/headers.feature.

Feature: Every view file starts with a VIEW header
  Brief §4.5: every file starts with "VIEW of <source path> @ <short sha>,
  indexed <UTC time>; read-only, edit the source file", written in the
  comment syntax of the file's format (PLAN.md §2.9). Raw source files under
  @files/ (D11) are not views and carry no header.

  Background:
    Given the view fixture "shop"

  Scenario: Every file under the root carries a header
    Then every view file under "" starts with a VIEW header

  Scenario Outline: The header uses the format's comment syntax
    Then the first line of "<path>" is "<header>"

    Examples: views of one source file name that file
      | path                               | header                                                                                                      |
      | files/src/ordering/service.rs.skel | # VIEW of src/ordering/service.rs @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file |

    Examples: aggregate views name the repo
      | path                    | header                                                                                          |
      | README.md               | <!-- VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file -->  |
      | FACETS.md               | <!-- VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file -->  |
      | modules.dot             | // VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file        |
      | unresolved/ordering.txt | # VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file         |
      | orphans.txt             | # VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file         |
      | arch/modules.mmd        | %% VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file        |
      | arch/modules.md         | <!-- VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file -->  |

  @query-tree
  Scenario: Every file in a result set carries a header
    Then every view file under "q/kind=method" starts with a VIEW header

  @query-tree
  Scenario Outline: Result-set files name their source
    Then the first line of "<path>" is "<header>"

    Examples:
      | path                                                   | header                                                                                                      |
      | q/kind=method/@symbols/ordering.OrderService.place.sym | # VIEW of src/ordering/service.rs @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file |
      | q/dispatch=virtual/@sites/4412.site                    | # VIEW of src/ordering/service.rs @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file |
      | q/kind=method/@views/callgraph.mmd                     | %% VIEW of shop/q/kind=method @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file     |
