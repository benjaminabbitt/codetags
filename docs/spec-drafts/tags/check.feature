# DRAFT for human review (P6.1).
#
# Questions for review:
# Q1. Error format: `<path>:<line>: <message>` on stderr, every error listed
#     (not just the first), sorted by line; exit 1 if any, else exit 0 with
#     no output. OK?
# Q2. A file that parses but is not canonical (unsorted lines, unsorted or
#     duplicated tags, needless quotes, CRLF, extra spaces) is an error, so
#     the committed file is always what a write would produce. Proposed fix:
#     `codetags tags check --fix`, which rewrites it canonically and fixes
#     nothing else. Is `--fix` wanted, or is "any tag/untag rewrites it"
#     enough?
# Q3. Comments and blank lines are not allowed. Should `#` comment lines be
#     allowed (and kept, and where would they sort)?
# Q4. `sym:` ids are refused in v1 (O-12: files only), though the id scheme
#     allows them. OK?
# Q5. An id path must be normalized: relative, `/`-separated, no empty,
#     `.` or `..` segment, no trailing `/`. A backslash is an ordinary
#     character (it is legal in Unix names), so `file:src\a.rs` is not an
#     error, but is likely an orphan on Windows. OK?

Feature: tags check validates the committed tags file
  `codetags tags check` is part of `just check` (Appendix A, `tags-check`),
  so a bad hand edit or a bad merge resolution fails CI with a message that
  names the line and says what is wrong.

  Background:
    Given a project with the files "src/a.rs src/b.rs src/c.rs"

  Scenario: A canonical file passes, silently
    Given the tags file holds:
      """
      file:src/a.rs owner=core
      file:src/b.rs area="my docs" risk=high
      """
    When codetags is run with "tags check"
    Then it exits with status 0
    And stdout is empty
    And stderr is empty

  Scenario: No tags file passes
    Given there is no tags file
    When codetags is run with "tags check"
    Then it exits with status 0

  Scenario: An empty tags file passes
    Given the tags file's bytes are ""
    When codetags is run with "tags check"
    Then it exits with status 0

  Scenario Outline: Each kind of error is reported with its line
    Given the tags file holds:
      """
      file:src/a.rs owner=core
      <line>
      """
    When codetags is run with "tags check"
    Then it exits with status 1
    And stderr matches "(?m)^\.codetags/tags:2: <message>"

    Examples:
      | line                              | message                                                 |
      | file:src/b.rs owner="core         | unterminated quote                                      |
      | "file:src/b.rs owner=core         | unterminated quote                                      |
      | file:src/b.rs a<b                 | invalid tag "a<b"                                       |
      | file:src/b.rs                     | file:src/b\.rs has no tags                              |
      | dir:src owner=core                | unknown item kind "dir:"; v1 tags only file: ids        |
      | sym:billing.Charge owner=core     | symbol ids are not taggable in v1                       |
      | file:./src/b.rs owner=core        | file:\./src/b\.rs is not a normalized project path      |
      | file:src//b.rs owner=core         | file:src//b\.rs is not a normalized project path        |
      | file:/src/b.rs owner=core         | file:/src/b\.rs is not a normalized project path        |
      | file:../b.rs owner=core           | file:\.\./b\.rs is not a normalized project path        |
      | file:src/b.rs area="a/b"          | area="a/b": a value may not contain "/"                 |
      | file:src/b.rs kind=function       | kind=function: "kind" is reserved                       |
      | file:src/a.rs risk=high           | file:src/a\.rs is also on line 1                        |
      | file:src/nope.rs owner=core       | orphan: file:src/nope\.rs no longer exists              |

  Scenario Outline: A file that is valid but not canonical is reported
    Given the tags file holds:
      """
      <content>
      """
    When codetags is run with "tags check"
    Then it exits with status 1
    And stderr matches "(?m)^\.codetags/tags:<n>: not canonical: <message>"
    And stderr matches "codetags tags check --fix"

    Examples:
      | content                                  | n | message                          |
      | file:src/b.rs x\nfile:src/a.rs x         | 2 | lines are not sorted by id       |
      | file:src/a.rs z a                        | 1 | tags are not sorted              |
      | file:src/a.rs a a                        | 1 | tag "a" appears twice            |
      | file:src/a.rs owner="core"               | 1 | needless quotes in owner="core"  |
      | "file:src/a.rs" x                        | 1 | needless quotes in the id        |
      | file:src/a.rs  x                         | 1 | fields are not single-spaced     |

  Scenario: Every error is reported, not just the first
    Given the tags file holds:
      """
      file:src/a.rs kind=function
      file:src/b.rs owner="core
      file:src/nope.rs x
      """
    When codetags is run with "tags check"
    Then it exits with status 1
    And stderr matches "\.codetags/tags:1: "
    And stderr matches "\.codetags/tags:2: "
    And stderr matches "\.codetags/tags:3: "

  Scenario: A blank line is an error
    Given the tags file's bytes are "file:src/a.rs x\n\nfile:src/b.rs x\n"
    When codetags is run with "tags check"
    Then it exits with status 1
    And stderr matches "\.codetags/tags:2: blank line"

  Scenario: A missing final newline is not canonical
    Given the tags file's bytes are "file:src/a.rs x"
    When codetags is run with "tags check"
    Then it exits with status 1
    And stderr matches "\.codetags/tags:1: not canonical: no final newline"

  Scenario: --fix rewrites a valid file canonically, and fixes nothing else
    Given the tags file holds:
      """
      file:src/b.rs  owner="core"
      file:src/a.rs z a
      """
    When codetags is run with "tags check --fix"
    Then it exits with status 0
    And the tags file is:
      """
      file:src/a.rs a z
      file:src/b.rs owner=core
      """

  Scenario: --fix does not touch a file with errors
    Given the tags file holds:
      """
      file:src/b.rs owner=core
      file:src/a.rs kind=function
      """
    When codetags is run with "tags check --fix"
    Then it exits with status 1
    And stderr matches "\.codetags/tags:2: kind=function: \"kind\" is reserved"
    And the tags file is unchanged
