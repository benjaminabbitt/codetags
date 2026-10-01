# DRAFT for human review (P2.1). Not run: drafts live outside features/.
#
# Questions for review:
#  1. arch/ comes from the default mermaid plugin (P7.4), so this layout
#     cannot pass until P7 lands, though P2's done-condition needs it.
#     Should the P2 view features run with default plugins disabled, and
#     arch/ and @views/ be asserted only in features/plugins/?
#  2. unresolved/ lists a file for every module, including those with no
#     unresolved sites ("0 of N"), so the rate is visible everywhere. Is
#     that right, or only modules with unresolved sites?
#  3. The .skel format, the module-graph format and the per-line edge
#     format below ("<file>:<line> <receiver> -> <target> [<dispatch>;
#     <source>]", with "also" for dispatch expansions) are proposals; the
#     brief only says "signatures + ordered call sites with file:line and
#     provenance". The second header line "# tags: ..." carries user tags
#     (P6.3) and is omitted when a file has none.
#  4. modules.dot draws no intra-module edges. Should it draw self-loops?

Feature: The view tree has a fixed layout
  Every frontend serves the same tree under <views root>/<repo>/
  (PLAN.md Appendix B). The static tree has no q/; every other backend
  does. The fixture is the "shop" project: four modules (api, billing,
  notify, ordering), six Rust files, seven call sites (one unresolved), and
  one orphaned tag.

  Background:
    Given the view fixture "shop"

  @static-tree
  Scenario: The static tree holds every root view and no q/
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
      unresolved/
      unresolved/api.txt
      unresolved/billing.txt
      unresolved/notify.txt
      unresolved/ordering.txt
      """

  @query-tree
  Scenario: A query tree adds an empty q/ to the same layout
    Then listing "" gives exactly "FACETS.md README.md arch/ files/ modules.dot orphans.txt q/ unresolved/"
    And listing "q" gives exactly ""

  Scenario: The README describes the layout
    Then "README.md" contains "FACETS.md"
    And "README.md" contains "files/"
    And "README.md" contains "modules.dot"
    And "README.md" contains "unresolved/"
    And "README.md" contains "orphans.txt"
    And "README.md" contains "arch/"

  @static-tree
  Scenario: The static README points agents to codetags q
    Then "README.md" contains "codetags q '"
    And "q" does not exist

  @query-tree
  Scenario: The README tells agents to single-quote query paths
    Then "README.md" contains "single-quote"
    And "README.md" contains "'q/"

  Scenario: FACETS.md lists facets with their counts
    Then "FACETS.md" contains the line "| facet | source | items | values |"
    And "FACETS.md" contains the line "| kind | derived | 24 | 6 |"
    And "FACETS.md" contains the line "| owner | user | 3 | 2 |"

  Scenario: A skeleton shows signatures and ordered call sites with provenance
    Then reading "files/src/ordering/service.rs.skel" gives:
      """
      # VIEW of src/ordering/service.rs @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      # tags: owner=orders

      struct ordering.OrderService  src/ordering/service.rs:10-14
        pub struct OrderService

      method ordering.OrderService.place  src/ordering/service.rs:40-70
        pub fn place(&self, order: Order) -> Result<OrderId>
        src/ordering/service.rs:58 self.repo -> ordering.OrderRepo.save [virtual; scip@a1b2c3d]
        src/ordering/service.rs:58 also -> ordering.PgOrderRepo.save [cha]
        src/ordering/service.rs:61 self.charge -> billing.Charge<T>.apply [static; scip@a1b2c3d]
        src/ordering/service.rs:63 self.charge -> billing.Charge<T>.apply [static; scip@a1b2c3d]
        src/ordering/service.rs:66 self.mailer -> notify.Mailer.send [static; scip@a1b2c3d]
      """

  Scenario: An unresolved call site is marked in its skeleton
    Then "files/src/ordering/repo.rs.skel" contains the line "  src/ordering/repo.rs:30 self.pool -> ? [dynamic; scip@a1b2c3d]"

  Scenario: The module graph weights edges by call sites
    Then reading "modules.dot" gives:
      """
      // VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      digraph modules {
        "api";
        "billing";
        "notify";
        "ordering";
        "api" -> "ordering" [weight=1, label="1"];
        "ordering" -> "billing" [weight=2, label="2"];
        "ordering" -> "notify" [weight=1, label="1"];
      }
      """

  Scenario: Unresolved call sites are reported per module
    Then reading "unresolved/ordering.txt" gives:
      """
      # VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      # module ordering: 1 of 5 call sites unresolved
      src/ordering/repo.rs:30 ordering.PgOrderRepo.save: self.pool -> ? [dynamic; scip@a1b2c3d]
      """
    And reading "unresolved/notify.txt" gives:
      """
      # VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      # module notify: 0 of 0 call sites unresolved
      """

  Scenario: A tagged file that is not in the generation is an orphan
    Then reading "orphans.txt" gives:
      """
      # VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      file:src/legacy/old.rs owner=billing
      """
