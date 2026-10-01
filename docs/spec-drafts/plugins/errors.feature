# DRAFT for human review (P7.1). Not run: drafts live outside features/.
#
# Questions for review:
#  1. Naming. A failed output is replaced by `<output name>.error` in the
#     same place (`broken/list.txt.error`). A plugin that fails to load has
#     no outputs, so its error goes to `<plugin name>.error` at the view
#     root. PLAN.md §2.9 only says "<name>.error". Agreed?
#  2. Templates run with strict undefined behaviour, so a typo is an error
#     file rather than silently empty output. Agreed?
#  3. `codetags doctor` exits 1 when any plugin failed, as it does for other
#     fixable problems. In a batch run (no daemon), doctor renders the root
#     mounts itself to count failures; result-set failures are counted only
#     by a running daemon. Is that acceptable?

Feature: A failing plugin renders an error file and never takes the views down
  PLAN.md §2.9: a plugin failure renders an error file. It never takes the
  mount down, and `codetags doctor` counts it.

  Background:
    Given the view fixture "shop"
    And the project file ".codetags/plugins/broken/plugin.toml" contains:
      """
      name = "broken"
      version = "1.0.0"
      schema = 1

      [[output]]
      path = "list.txt"
      mount = ["root", "query"]
      query = "list.sql"
      template = "list.txt.j2"
      """
    And the project file ".codetags/plugins/broken/list.txt.j2" contains:
      """
      {% for row in rows -%}
      {{ row.name }}
      {% endfor -%}
      """

  Scenario: A failing query renders an error file in place of the output
    Given the project file ".codetags/plugins/broken/list.sql" contains:
      """
      SELECT nme FROM ct.symbols ORDER BY nme
      """
    Then listing "broken" gives exactly "list.txt.error"
    And the first line of "broken/list.txt.error" is "# VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file"
    And the content of "broken/list.txt.error" matches "^# plugin broken 1\.0\.0 failed in list\.sql: .*nme"

  Scenario: A failing template renders an error file in place of the output
    Given the project file ".codetags/plugins/broken/list.sql" contains:
      """
      SELECT name FROM ct.symbols ORDER BY name
      """
    And the project file ".codetags/plugins/broken/list.txt.j2" contains:
      """
      {{ rows[0].nme }}
      """
    Then listing "broken" gives exactly "list.txt.error"
    And the content of "broken/list.txt.error" matches "^# plugin broken 1\.0\.0 failed in list\.txt\.j2: .*nme"

  Scenario: The rest of the tree keeps working
    Given the project file ".codetags/plugins/broken/list.sql" contains:
      """
      SELECT nme FROM ct.symbols ORDER BY nme
      """
    Then "broken" is a directory
    And listing "arch" gives exactly "api.md api.mmd billing.md billing.mmd modules.md modules.mmd notify.md notify.mmd ordering.md ordering.mmd"
    And "files/src/ordering/service.rs.skel" is a file
    And the first line of "README.md" is "<!-- VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file -->"
    And every view file under "" starts with a VIEW header

  @query-tree
  Scenario: A failure in a result set is local to that result set
    Given the project file ".codetags/plugins/broken/list.sql" contains:
      """
      SELECT nme FROM ct.symbols ORDER BY nme
      """
    Then listing "q/module=billing/@views" gives exactly "callgraph.md callgraph.mmd list.txt.error"
    And "q/module=billing/@views/callgraph.mmd" is a file
    And "q/module=billing/@symbols/billing.order.sym" is a file

  Scenario: A plugin that fails to load renders an error file at the root
    Given the project file ".codetags/plugins/broken/plugin.toml" contains:
      """
      version = "1.0.0"
      schema = 1
      """
    Then "broken.error" is a file
    And the content of "broken.error" matches "^# plugin broken failed to load \.codetags/plugins/broken/plugin\.toml: .*name"
    And "broken" does not exist

  Scenario: A manifest name must match its directory
    Given the project file ".codetags/plugins/broken/plugin.toml" contains:
      """
      name = "other"
      version = "1.0.0"
      schema = 1
      """
    Then "broken.error" is a file
    And the content of "broken.error" matches "name \"other\" does not match its directory \"broken\""

  Scenario: codetags doctor counts plugin failures
    Given the project file ".codetags/plugins/broken/list.sql" contains:
      """
      SELECT nme FROM ct.symbols ORDER BY nme
      """
    When codetags is run in the project with "doctor"
    Then it exits with status 1
    And stdout matches "(?m)^view plugins: 1 failed \(broken: broken/list\.txt\)$"
