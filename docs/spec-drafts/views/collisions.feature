# DRAFT for human review (P2.1). Not run: drafts live outside features/.
#
# Questions for review:
#  1. Which targets count as case-insensitive? This draft assumes: a
#     materialized tree on APFS or NTFS defaults; and no live mount (FUSE and
#     NFS are case-sensitive, and P5.3 sets WinFsp to case-sensitive search).
#     But Win32 callers open names case-insensitively even on a
#     case-sensitive WinFsp volume, so WinFsp may need suffixes too. Should
#     the static tree instead always suffix, so that one tree is the same on
#     every OS and survives being copied?
#  2. The suffix goes before the view's own extension (`Util.rs~63adf7.skel`),
#     so the extension still names the format. PLAN.md §2.7 doesn't say
#     where it goes; codetags-names leaves it to the caller.
#  3. The header still names the real source (`src/Util.rs`), which is how an
#     agent maps a suffixed name back. Is that enough, or should README.md
#     explain suffixes?

Feature: Names that collide under case folding get a stable suffix
  PLAN.md §2.7: when a tree lands on a case-insensitive target, only the
  names in one directory that collide under case folding get "~" and 6 hex
  digits of a hash of the item id. The hashes here are the ones
  features/names/collisions.feature fixes for the same ids.

  @case-insensitive
  Scenario: Colliding file views are suffixed before their extension
    Given a view fixture with only these files:
      | path        | language |
      | src/Util.rs | rust     |
      | src/util.rs | rust     |
      | src/main.rs | rust     |
    And the views target is case-insensitive
    Then listing "files/src" gives exactly "Util.rs~63adf7.skel main.rs.skel util.rs~be1149.skel"
    And the first line of "files/src/Util.rs~63adf7.skel" is "# VIEW of src/Util.rs @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file"

  @case-sensitive
  Scenario: Nothing is suffixed on a case-sensitive target
    Given a view fixture with only these files:
      | path        | language |
      | src/Util.rs | rust     |
      | src/util.rs | rust     |
      | src/main.rs | rust     |
    Then listing "files/src" gives exactly "Util.rs.skel main.rs.skel util.rs.skel"

  @case-insensitive @query-tree
  Scenario: Colliding symbol names in a result set are suffixed
    Given the view fixture "shop"
    And the views target is case-insensitive
    Then listing "q/module=billing/@symbols" gives exactly "billing.Charge.sym billing.Charge<T>.apply.sym billing.Order~d2e75b.sym billing.order~20e6ad.sym"
    And the first line of "q/module=billing/@symbols/billing.Order~d2e75b.sym" is "# VIEW of src/billing/order.rs @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file"
