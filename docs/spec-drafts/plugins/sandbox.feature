# DRAFT for human review (P7.1, for P7.3). Not run: drafts live outside features/.
#
# Questions for review:
#  1. V22 is still pending: which DuckDB settings block what in 1.5.x
#     (enable_external_access, lock_configuration, autoinstall and autoload
#     of extensions). These scenarios state the required outcome, not the
#     mechanism or DuckDB's error text, so they hold whatever V22 finds.
#  2. Should a plugin also be refused the raw tables (`symbol`, `call_site`)
#     and see only the versioned `ct` schema? PLAN.md §2.9 says plugins
#     query the view schema "so internal schema changes don't break
#     plugins", but a read-only connection can still read raw tables. This
#     draft leaves it unspecified.
#  3. Should the runtime accept exactly one SQL statement per file? A
#     multi-statement file is the easiest way to smuggle a SET or ATTACH in
#     front of a SELECT; this draft only requires that it be refused.
#  4. Relative paths in SQL resolve against the views process's working
#     directory, not the project. The "no file exists" checks look in both.

Feature: Declarative plugins run in a sandbox
  PLAN.md §2.8 and P7.3: a declarative plugin from a freshly cloned repo is
  data, not code. Its SQL runs on a read-only DuckDB connection with external
  access disabled and the configuration locked (V22). Its template engine has
  no loader, so no filesystem or network access. Anything else is refused,
  and the refusal is an error file like any other plugin failure.

  Background:
    Given the view fixture "shop"
    And the project file "secrets.csv" contains:
      """
      name,value
      token,hunter2
      """
    And the project file ".codetags/plugins/leak/plugin.toml" contains:
      """
      name = "leak"
      version = "1.0.0"
      schema = 1

      [[output]]
      path = "leak.txt"
      mount = ["root"]
      query = "leak.sql"
      template = "leak.txt.j2"
      """
    And the project file ".codetags/plugins/leak/leak.txt.j2" contains:
      """
      {% for row in rows %}{{ row }}
      {% endfor %}
      """

  Scenario: A query over the plugin schema is allowed
    Given the project file ".codetags/plugins/leak/leak.sql" contains:
      """
      SELECT count(*) AS n FROM ct.symbols
      """
    Then "leak/leak.txt" is a file
    And "leak/leak.txt" contains "11"

  Scenario Outline: SQL that reaches outside the generation is refused
    Given the project file ".codetags/plugins/leak/leak.sql" contains:
      """
      <sql>
      """
    Then listing "leak" gives exactly "leak.txt.error"
    And the content of "leak/leak.txt.error" matches "^# plugin leak 1\.0\.0 failed in leak\.sql: "
    And no file under "" contains "hunter2"

    Examples: reading files
      | sql                                                        |
      | SELECT * FROM read_csv('secrets.csv')                      |
      | SELECT * FROM read_text('secrets.csv')                     |
      | SELECT * FROM 'secrets.csv'                                |
    Examples: undoing the sandbox
      | sql                                                                         |
      | SET enable_external_access = true                                           |
      | SELECT 1; SET enable_external_access = true; SELECT * FROM 'secrets.csv'    |
      | RESET enable_external_access                                                |
    Examples: extensions
      | sql                                                        |
      | INSTALL httpfs                                             |
      | LOAD httpfs                                                |
    Examples: writing
      | sql                                                        |
      | CREATE TABLE stolen AS SELECT 1 AS one                     |
      | CREATE TEMP TABLE stolen AS SELECT 1 AS one                |

  Scenario Outline: SQL that would create a file is refused and creates nothing
    Given the project file ".codetags/plugins/leak/leak.sql" contains:
      """
      <sql>
      """
    Then listing "leak" gives exactly "leak.txt.error"
    And no file "<file>" exists in the project or the working directory

    Examples:
      | sql                                     | file           |
      | ATTACH 'stolen.duckdb' AS stolen        | stolen.duckdb  |
      | COPY (SELECT 1 AS one) TO 'out.csv'     | out.csv        |
      | EXPORT DATABASE 'exported'              | exported       |

  Scenario: ATTACH of another generation is refused
    Given the project file ".codetags/plugins/leak/leak.sql" contains:
      """
      ATTACH '.codetags/index/gen-1.duckdb' AS raw (READ_ONLY)
      """
    Then listing "leak" gives exactly "leak.txt.error"

  Scenario Outline: A template cannot load another file
    Given the project file ".codetags/plugins/leak/leak.sql" contains:
      """
      SELECT 1 AS one
      """
    And the project file ".codetags/plugins/leak/leak.txt.j2" contains:
      """
      <template>
      """
    And the project file ".codetags/plugins/leak/secret.j2" contains:
      """
      hunter2
      """
    Then listing "leak" gives exactly "leak.txt.error"
    And the content of "leak/leak.txt.error" matches "^# plugin leak 1\.0\.0 failed in leak\.txt\.j2: "
    And no file under "" contains "hunter2"

    Examples:
      | template                                  |
      | {% include "secret.j2" %}                 |
      | {% import "secret.j2" as s %}             |
      | {% from "secret.j2" import x %}           |
      | {% extends "secret.j2" %}                 |
      | {% include "../../../secrets.csv" %}      |
