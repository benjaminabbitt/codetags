# DRAFT for human review (P6.1).
#
# Questions for review:
# Q1. The derived-facet keys are generated into docs/facets.md (PLAN.md
#     §2.4), which doesn't exist yet. The examples use keys from the brief's
#     ingest sketch (kind, lang, module, calls, caller, target, line,
#     dispatch) plus `file`, which §2.7 queries with `file~...`. Is `file`
#     a reserved key?
# Q2. `tagma.*` is read as: the namespace `tagma` and every namespace that
#     starts with `tagma.`. Is the bare `tagma` namespace reserved too?
# Q3. Matching is exact and case-sensitive (`Kind=x` is allowed). Should it
#     fold case, so a user can't make a near-duplicate of a facet?
# Q4. Extra reserved keys come from `.codetags/config.toml`. Proposed table
#     and key: `[tags] reserved = ["team"]`. Do they reserve null-namespace
#     keys only, or also namespaces?
# Q5. The message lists the whole reserved set, as §2.4 asks. With dozens
#     of facet keys that is long; proposed: name the offending tag and why,
#     then list the namespaces and point to FACETS.md for the keys.

Feature: Reserved names cannot be written by users
  Derived facets are rebuilt from the code with every generation, so a user
  tag with the same name would be ambiguous in a query. Users may not write
  any null-namespace key a derived facet uses, the namespaces `prov`, `ct`,
  `fs` and `tagma.*`, or any key `config.toml` reserves (PLAN.md §2.4). The
  refusal names the reserved set, and nothing is written.

  Background:
    Given a project with the files "src/main.rs"

  Scenario Outline: A reserved name is refused, and the message says why
    When codetags is run with "tag src/main.rs <tag>"
    Then it exits with status 1
    And stderr matches "<tag>: \"<name>\" is reserved: <why>"
    And stderr matches "reserved namespaces: prov, ct, fs, tagma\.\*"
    And stderr matches "reserved keys: the derived facets listed in FACETS\.md"
    And there is no tags file

    Examples:
      | tag                | name       | why                  |
      | kind=function      | kind       | a derived facet key  |
      | lang=rust          | lang       | a derived facet key  |
      | module=billing     | module     | a derived facet key  |
      | calls=x            | calls      | a derived facet key  |
      | line=10            | line       | a derived facet key  |
      | prov:source=user   | prov       | a reserved namespace |
      | ct:note=x          | ct         | a reserved namespace |
      | fs:ext=rs          | fs         | a reserved namespace |
      | tagma.hide:x=y     | tagma.hide | a reserved namespace |

  Scenario: One reserved tag refuses the whole command
    When codetags is run with "tag src/main.rs owner=core kind=function"
    Then it exits with status 1
    And there is no tags file

  Scenario: A key reserved in config.toml is refused, and the message names the config
    Given the project config holds:
      """
      [tags]
      reserved = ["team"]
      """
    When codetags is run with "tag src/main.rs team=core"
    Then it exits with status 1
    And stderr matches "team=core: \"team\" is reserved: listed in \.codetags/config\.toml"

  Scenario Outline: A reserved key is fine inside a user namespace, and vice versa
    When codetags is run with "tag src/main.rs <tag>"
    Then it exits with status 0

    Examples:
      | tag                       |
      | review:kind=security      |
      | owner=billing             |
      | review:status=needs-audit |
      | provenance=vendor         |

  Scenario: Untagging a reserved name is refused the same way
    When codetags is run with "untag src/main.rs kind=function"
    Then it exits with status 1
    And stderr matches "\"kind\" is reserved"

  Scenario: A reserved name already in the file is reported by tags check
    Given the tags file holds:
      """
      file:src/main.rs kind=function owner=core
      """
    When codetags is run with "tags check"
    Then it exits with status 1
    And stderr matches "\.codetags/tags:1: kind=function: \"kind\" is reserved: a derived facet key"
