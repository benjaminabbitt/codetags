# DRAFT for human review (P7.1). Not run: drafts live outside features/.
#
# Questions for review:
#  1. The engine, not the template, writes the header, choosing the comment
#     syntax from the output's extension, so no plugin can omit or misspell
#     it. Agreed?
#  2. The built-in table here: `#` for .txt, .skel, .sym, .site and .error;
#     `//` for .dot; `%%` for .mmd; `<!-- -->` for .md. A manifest's
#     `comment` sets a line prefix for any other extension.
#  3. A format with no comment syntax at all (JSON, CSV) is refused at load
#     time. The alternative is to allow it with no header, which breaks the
#     brief's "every file" rule. Refuse, or allow with the header elsewhere
#     (e.g. a sibling .view file)?

Feature: The engine writes the VIEW header in each format's comment syntax
  PLAN.md §2.9: every output starts with the brief's VIEW header, in the
  output format's comment syntax (`%%` for Mermaid).

  Background:
    Given the view fixture "shop"
    And the project file ".codetags/plugins/fmt/one.sql" contains:
      """
      SELECT 1 AS one
      """
    And the project file ".codetags/plugins/fmt/body.j2" contains:
      """
      body
      """

  Scenario Outline: A known extension gets its comment syntax
    Given the project file ".codetags/plugins/fmt/plugin.toml" contains:
      """
      name = "fmt"
      version = "1.0.0"
      schema = 1

      [[output]]
      path = "out.<ext>"
      mount = ["root"]
      query = "one.sql"
      template = "body.j2"
      """
    Then the first line of "fmt/out.<ext>" is "<header>"
    And "fmt/out.<ext>" contains the line "body"

    Examples:
      | ext  | header                                                                                          |
      | txt  | # VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file         |
      | skel | # VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file         |
      | dot  | // VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file        |
      | mmd  | %% VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file        |
      | md   | <!-- VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file -->  |

  Scenario: A manifest sets the comment prefix for another extension
    Given the project file ".codetags/plugins/fmt/plugin.toml" contains:
      """
      name = "fmt"
      version = "1.0.0"
      schema = 1

      [[output]]
      path = "out.sql"
      mount = ["root"]
      query = "one.sql"
      template = "body.j2"
      comment = "--"
      """
    Then the first line of "fmt/out.sql" is "-- VIEW of shop @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file"

  Scenario: An output with no comment syntax is refused at load
    Given the project file ".codetags/plugins/fmt/plugin.toml" contains:
      """
      name = "fmt"
      version = "1.0.0"
      schema = 1

      [[output]]
      path = "out.json"
      mount = ["root"]
      query = "one.sql"
      template = "body.j2"
      """
    Then "fmt.error" is a file
    And the content of "fmt.error" matches "out\.json: no comment syntax for \.json; set `comment` in plugin\.toml"
    And "fmt" does not exist
