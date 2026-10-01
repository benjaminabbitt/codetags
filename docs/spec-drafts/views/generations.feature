# DRAFT for human review (P2.1). Not run: drafts live outside features/.
#
# Questions for review:
#  1. On Windows, the static tree's swap (build a sibling, then rename) fails
#     while a file in the old tree is open, and P2.5 retries. The "held open"
#     scenario then waits on itself. Should it be excluded on the Windows
#     static backend, or should the materializer swap per file there?
#  2. A held directory listing (opendir before the switch, readdir after) is
#     not specified. NFSv3 has no open state for directories, so the macOS
#     backend can't promise it. Leave it unspecified?
#  3. The tags revision is the other half of the cache key (PLAN.md §2.5). A
#     tag write that changes a view is P6.3's scenario; this file covers
#     generations only.

Feature: Views are deterministic per generation, and a reader keeps its generation
  PLAN.md §2.2 and §2.5: every file's contents are a function of the
  (generation, tags revision) pair, so getattr's size matches the read and
  every frontend renders the same bytes. A file opened during generation N
  reads entirely from N, even after N+1 is published and the views switch.

  Background:
    Given the view fixture "shop"

  Scenario: Reading a file twice gives the same bytes
    Then reading every file under "" twice gives the same bytes

  @query-tree
  Scenario: Reading a result set twice gives the same bytes
    Then reading every file under "q/kind=method" twice gives the same bytes

  Scenario: The size reported before a read is the size read
    Then the size of every file under "" equals the number of bytes read

  @query-tree
  Scenario: The size reported for a result-set file is the size read
    Then the size of every file under "q/module=billing" equals the number of bytes read

  Scenario: Rebuilding from the same generation gives the same tree
    When the views are rebuilt from the same generation
    Then every file under "" has the same bytes as before

  Scenario: Every backend renders the same bytes as the in-memory walker
    Then every file under "" is byte-identical to the in-memory rendering

  @query-tree
  Scenario: Every backend renders result sets as the in-memory walker does
    Then every file under "q/module=billing" is byte-identical to the in-memory rendering

  Scenario: A file opened before a new generation keeps reading the old one
    Given "modules.dot" is held open
    And "files/src/api/routes.rs.skel" is held open
    When generation 2 is published at commit "e4f5a6b", indexed "2026-09-30T13:00:00Z", without the file "src/api/routes.rs"
    And the views switch to the newest generation
    Then the first line of the held file "modules.dot" is "// VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file"
    And the held file "modules.dot" contains the line '  "api" -> "ordering" [weight=1, label="1"];'
    And the first line of the held file "files/src/api/routes.rs.skel" is "# VIEW of src/api/routes.rs @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file"

  Scenario: A file opened after the switch reads the new generation
    When generation 2 is published at commit "e4f5a6b", indexed "2026-09-30T13:00:00Z", without the file "src/api/routes.rs"
    And the views switch to the newest generation
    Then the first line of "modules.dot" is "// VIEW of shop @ e4f5a6b, indexed 2026-09-30T13:00:00Z; read-only, edit the source file"
    And "modules.dot" does not contain '"api"'
    And "files/src/api/routes.rs.skel" does not exist
    And "unresolved/api.txt" does not exist

  @query-tree
  Scenario: A query sees the new generation after the switch
    When generation 2 is published at commit "e4f5a6b", indexed "2026-09-30T13:00:00Z", without the file "src/api/routes.rs"
    And the views switch to the newest generation
    Then listing "q/kind=function/@symbols" gives exactly "billing.order.sym"
    And "q/dispatch=static/@sites/4416.site" does not exist
