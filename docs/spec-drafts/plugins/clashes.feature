# DRAFT for human review (P7.1, for P7.2). Not run: drafts live outside features/.
#
# Questions for review:
#  1. On a clash, the plugin loaded first keeps serving and the later one is
#     refused. Load order: built-ins, then the defaults embedded in the
#     binary (mermaid), then .codetags/plugins/ in name order. So a project
#     can never replace a built-in or default by reusing its name. Should
#     there be an explicit override (e.g. `replaces = "mermaid"` in the
#     manifest) for projects that want their own Mermaid output?
#  2. Clashes are checked on the plugin name and on every path an output
#     would take, at the root and under @views/. `q` and the root files the
#     core owns are reserved. Is anything else reserved?
#  3. A drill-down for a module named "modules" would be arch/modules.mmd,
#     which is the module graph's own path (PLAN.md §2.9 layout). This draft
#     does not settle it; one option is arch/module/<module>.mmd.

Feature: Plugin name and path clashes are errors naming both sources
  PLAN.md P7.2: plugins load built-ins first, then .codetags/plugins/. A
  name clash is an error naming both sources. The plugin that loaded first
  keeps serving; the clashing one is refused with an error file.

  Background:
    Given the view fixture "shop"

  Scenario: A project plugin may not reuse a built-in plugin's name
    Given the project file ".codetags/plugins/readme/plugin.toml" contains:
      """
      name = "readme"
      version = "1.0.0"
      schema = 1
      """
    Then "readme.error" is a file
    And the content of "readme.error" matches "plugin name \"readme\" is taken: built in, and \.codetags/plugins/readme/plugin\.toml"
    And the first line of "README.md" is "<!-- VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file -->"
    And "README.md" contains "FACETS.md"

  Scenario: A project plugin may not reuse a default plugin's name
    Given the project file ".codetags/plugins/mermaid/plugin.toml" contains:
      """
      name = "mermaid"
      version = "9.0.0"
      schema = 1
      """
    Then "mermaid.error" is a file
    And the content of "mermaid.error" matches "plugin name \"mermaid\" is taken: default \(embedded\), and \.codetags/plugins/mermaid/plugin\.toml"
    And the first line of "arch/modules.mmd" is "%% VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file"

  @query-tree
  Scenario: Two plugins may not claim one result-set path
    Given the project file ".codetags/plugins/graphs/plugin.toml" contains:
      """
      name = "graphs"
      version = "1.0.0"
      schema = 1

      [[output]]
      path = "callgraph.mmd"
      mount = ["query"]
      query = "one.sql"
      template = "graph.j2"
      """
    And the project file ".codetags/plugins/graphs/one.sql" contains:
      """
      SELECT 1 AS one
      """
    And the project file ".codetags/plugins/graphs/graph.j2" contains:
      """
      flowchart LR
      """
    Then "graphs.error" is a file
    And the content of "graphs.error" matches "@views/callgraph\.mmd is taken: mermaid \(default \(embedded\)\), and graphs \(\.codetags/plugins/graphs/plugin\.toml\)"
    And "q/kind=method/@views/callgraph.mmd" contains "subgraph"

  Scenario Outline: A plugin may not take a root path the core or another plugin owns
    Given the project file ".codetags/plugins/<name>/plugin.toml" contains:
      """
      name = "<name>"
      version = "1.0.0"
      schema = 1
      dir = "<dir>"
      """
    Then "<name>.error" is a file
    And the content of "<name>.error" matches "<message>"

    Examples:
      | name      | dir       | message                                                          |
      | queries   | q         | root directory "q" is reserved                                   |
      | skeletons | files     | root directory "files" is taken: skel \(built in\)               |
      | diagrams  | arch      | root directory "arch" is taken: mermaid \(default \(embedded\)\) |
      | notes     | README.md | root directory "README.md" is taken: readme \(built in\)         |

  Scenario: codetags doctor reports a clash
    Given the project file ".codetags/plugins/readme/plugin.toml" contains:
      """
      name = "readme"
      version = "1.0.0"
      schema = 1
      """
    When codetags is run in the project with "doctor"
    Then it exits with status 1
    And stdout matches "plugin name \"readme\" is taken: built in, and \.codetags/plugins/readme/plugin\.toml"
