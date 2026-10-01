# DRAFT for human review (P2.1). Not run: drafts live outside features/.
#
# Questions for review:
#  1. D11 is an unreviewed draft. What holds either way: @files/ exists when
#     the result set holds files, and it nests their paths as directories.
#     What D11 decides, and is therefore only in @pending-D11 scenarios: the
#     leaf names (the source file name, or "<name>.skel" as in the pre-D11
#     O-7 default), whether @skel/ exists, and what a leaf reads as.
#  2. The card formats (.sym, .site) are proposals that reuse the .skel edge
#     line, so one grep pattern works across files/, @symbols/ and @sites/.
#  3. A file item's @files/ tree nests directories only down to the files in
#     the result set; an intermediate directory with no matching file below
#     it is not listed. Agreed?

@query-tree
Feature: Result groups name their items canonically
  PLAN.md §2.7 and Appendix B. Results are grouped by kind: @files/<path>,
  @symbols/<canonical name>.sym and @sites/<n>.site. Canonical names are
  dotted and keep their real characters (generics included), never contain
  "/", and are never read as query elements.

  Background:
    Given the view fixture "shop"

  Scenario: Symbols are listed by canonical name
    Then listing "q/module=billing/@symbols" gives exactly "billing.Charge.sym billing.Charge<T>.apply.sym billing.Order.sym billing.order.sym"

  Scenario: Call sites are listed by number
    Then listing "q/caller=ordering.OrderService.place/@sites" gives exactly "4412.site 4413.site 4414.site 4415.site"

  Scenario: Files nest under @files/ by their project path
    Then listing "q/owner=billing/@files" gives exactly "src/"
    And listing "q/owner=billing/@files/src" gives exactly "billing/"

  Scenario: An orphaned tag matches nothing in the generation
    Then "q/owner=billing/@files/src/legacy" does not exist

  Scenario: A symbol card shows the signature, its calls and its callers
    Then reading "q/kind=method/@symbols/ordering.OrderService.place.sym" gives:
      """
      # VIEW of src/ordering/service.rs @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      method ordering.OrderService.place  src/ordering/service.rs:40-70
        pub fn place(&self, order: Order) -> Result<OrderId>

      calls
        src/ordering/service.rs:58 self.repo -> ordering.OrderRepo.save [virtual; scip@a1b2c3d]
        src/ordering/service.rs:58 also -> ordering.PgOrderRepo.save [cha]
        src/ordering/service.rs:61 self.charge -> billing.Charge<T>.apply [static; scip@a1b2c3d]
        src/ordering/service.rs:63 self.charge -> billing.Charge<T>.apply [static; scip@a1b2c3d]
        src/ordering/service.rs:66 self.mailer -> notify.Mailer.send [static; scip@a1b2c3d]

      called by
        src/api/routes.rs:9 service -> ordering.OrderService.place [static; scip@a1b2c3d]
      """

  Scenario: A site card shows every target and how it was resolved
    Then reading "q/dispatch=virtual/@sites/4412.site" gives:
      """
      # VIEW of src/ordering/service.rs @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      site 4412 in ordering.OrderService.place
        src/ordering/service.rs:58 self.repo -> ordering.OrderRepo.save [virtual; scip@a1b2c3d]
        src/ordering/service.rs:58 also -> ordering.PgOrderRepo.save [cha]
      """

  @pending-D11
  Scenario: A query with files lists @files/ and @skel/
    Then listing "q/owner=billing" gives exactly "@files/ @skel/ @views/"

  @pending-D11
  Scenario: @files/ holds the real source files
    Then listing "q/owner=billing/@files/src/billing" gives exactly "charge.rs order.rs"
    And reading "q/owner=billing/@files/src/billing/charge.rs" gives the bytes of the project file "src/billing/charge.rs"

  @pending-D11
  Scenario: @skel/ holds the same skeletons as files/
    Then listing "q/owner=billing/@skel/src/billing" gives exactly "charge.rs.skel order.rs.skel"
    And "q/owner=billing/@skel/src/billing/charge.rs.skel" has the same bytes as "files/src/billing/charge.rs.skel"
