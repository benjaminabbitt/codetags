# DRAFT for human review (P7.1). Not run: drafts live outside features/.
#
# Questions for review:
#  1. The manifest format is a proposal: `[[output]]` tables, each with a
#     `path`, a `mount` list ("root", "query" or both), one SQL file and one
#     template. The engine runs the query and passes its rows to the
#     template as `rows`. Per-entry outputs (one file per module) are in
#     mermaid.feature.
#  2. Where outputs land. PLAN.md §2.9 says `<root>/<plugin>/...` at the root
#     and `@views/<plugin>.<ext>` in result sets, but mermaid's root
#     directory is `arch/` and its result-set file is `callgraph.mmd`, neither
#     of which is the plugin's name. This draft uses: at the root,
#     `<dir>/<path>`, where `<dir>` is the manifest's `dir` (default: the
#     plugin's name); in result sets, `@views/<path>`. Mermaid sets
#     `dir = "arch"`.
#  3. The plugin schema names (`ct.symbols`, `ct.scope`) are placeholders for
#     docs/plugin-schema.md, which P7.3 writes. `ct.scope` holds the item ids
#     of the result set, or every item at the root.
#  4. Determinism of a declarative plugin depends on its SQL having an ORDER
#     BY. Should the runtime enforce it (refuse a query whose result order
#     is undefined, or sort rows itself), or leave it to the author?
#  5. The cache key is (generation, tags revision, plugin version). Editing a
#     project plugin's files without bumping `version` would then serve stale
#     output. Should the key use a hash of the plugin's files instead?

Feature: Plugins render deterministic files at the root and in result sets
  PLAN.md §2.9 (D8): every view comes from a ViewPlugin. A plugin may mount
  at the root, inside every q/ result set under @views/, or both. Its output
  is deterministic per (generation, tags revision, plugin version). This
  feature uses a small declarative project plugin, "methods", that lists the
  methods in scope.

  Background:
    Given the view fixture "shop"
    And the project file ".codetags/plugins/methods/plugin.toml" contains:
      """
      name = "methods"
      version = "1.0.0"
      schema = 1

      [[output]]
      path = "methods.txt"
      mount = ["root", "query"]
      query = "methods.sql"
      template = "methods.txt.j2"
      """
    And the project file ".codetags/plugins/methods/methods.sql" contains:
      """
      SELECT s.name
      FROM ct.symbols AS s
      JOIN ct.scope AS sc ON sc.item_id = s.item_id
      WHERE s.kind = 'method'
      ORDER BY s.name
      """
    And the project file ".codetags/plugins/methods/methods.txt.j2" contains:
      """
      {% for row in rows -%}
      {{ row.name }}
      {% endfor -%}
      """

  Scenario: A root mount renders under the plugin's directory
    Then listing "methods" gives exactly "methods.txt"
    And reading "methods/methods.txt" gives:
      """
      # VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      billing.Charge<T>.apply
      notify.Mailer.send
      ordering.OrderRepo.save
      ordering.OrderService.place
      ordering.PgOrderRepo.save
      """

  @query-tree
  Scenario: A result-set mount renders over just that result set
    Then listing "q/module=billing/@views" gives exactly "callgraph.md callgraph.mmd methods.txt"
    And reading "q/module=billing/@views/methods.txt" gives:
      """
      # VIEW of shop/q/module=billing @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file
      billing.Charge<T>.apply
      """

  @query-tree
  Scenario: An empty result set has no @views/
    Then listing "q/kind=interface" gives exactly ""
    And "q/kind=interface/@views/methods.txt" does not exist

  @query-tree
  Scenario: A root-only plugin is not in result sets
    Given the project file ".codetags/plugins/methods/plugin.toml" contains:
      """
      name = "methods"
      version = "1.0.0"
      schema = 1

      [[output]]
      path = "methods.txt"
      mount = ["root"]
      query = "methods.sql"
      template = "methods.txt.j2"
      """
    Then "methods/methods.txt" is a file
    And listing "q/module=billing/@views" gives exactly "callgraph.md callgraph.mmd"

  Scenario: A query-only plugin has nothing at the root
    Given the project file ".codetags/plugins/methods/plugin.toml" contains:
      """
      name = "methods"
      version = "1.0.0"
      schema = 1

      [[output]]
      path = "methods.txt"
      mount = ["query"]
      query = "methods.sql"
      template = "methods.txt.j2"
      """
    Then "methods" does not exist

  Scenario: Plugin output reads the same every time
    Then reading every file under "methods" twice gives the same bytes
    And the size of every file under "methods" equals the number of bytes read

  @query-tree
  Scenario: Result-set output reads the same every time
    Then reading every file under "q/module=billing/@views" twice gives the same bytes
    And the size of every file under "q/module=billing/@views" equals the number of bytes read

  Scenario: Plugin output survives a rebuild unchanged
    When the views are rebuilt from the same generation
    Then every file under "methods" has the same bytes as before
    And every file under "arch" has the same bytes as before

  Scenario: Every backend renders plugin output as the in-memory walker does
    Then every file under "methods" is byte-identical to the in-memory rendering
    And every file under "arch" is byte-identical to the in-memory rendering
