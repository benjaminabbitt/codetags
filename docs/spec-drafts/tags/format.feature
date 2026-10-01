# DRAFT for human review (P6.1). Not under features/: these steps are not
# defined yet (docs/spec-drafts/steps-proposed-tags.md).
#
# Questions for review:
# Q1. Ids follow D15 and PLAN.md §2.3: an id is quoted the tagma way only
#     when it contains whitespace or `"`, and inside quotes `"` and `\` are
#     written `\"` and `\\` (tagma's qtoken grammar, SPEC.md §2). D15 says
#     "no escaping"; these two escapes are tagma's own quoting, not
#     percent-encoding. Confirm. NB the P1.2 code and
#     features/names/item-ids.feature still percent-encode ids (pre-D15).
# Q2. Sort order. Lines: by the decoded id, bytewise (UTF-8), so quoting
#     never changes the order. Tags within a line: by (namespace, key,
#     value) decoded and bytewise, no namespace first and no value first.
#     The alternative is to sort by the rendered text. Which?
# Q3. Tags are a set: `owner=a` and `owner=b` can both hold (tagma keys are
#     multi-valued). Should `tag` instead replace a key's existing value?
# Q4. Line endings: always written LF. Proposed: `.codetags/.gitattributes`
#     holds `tags text eol=lf`, and the reader accepts CRLF. §2.3 lists no
#     .gitattributes; adding one is a change to the layout.
# Q5. When the last tag is removed, the file stays, empty (0 bytes), rather
#     than being deleted. OK?
# Q6. Values, keys and namespaces may not contain `/` or a control
#     character (D15: `/` is kept out of values by design). Confirm this
#     applies to user tags too.

Feature: The tags file has one canonical, byte-exact rendering
  `.codetags/tags` is committed, and holds one `<id> <tag>...` line per tagged
  item (PLAN.md §2.3, C5). Every write renders the whole file canonically:
  lines sorted, tags sorted within each line, each component bare when tagma
  allows it and quoted otherwise, LF line endings, a final newline. So two
  people who make the same change get the same bytes, and diffs stay minimal.

  Background:
    Given a project with the files "src/billing/charge.rs src/main.rs docs/guide.md"

  Scenario: A first tag creates the file with one line
    When codetags is run with "tag src/main.rs owner=core"
    Then it exits with status 0
    And the tags file is:
      """
      file:src/main.rs owner=core
      """

  Scenario: Lines are sorted by id, and tags within a line are sorted
    When codetags is run with "tag src/main.rs risk=low"
    And codetags is run with "tag src/billing/charge.rs risk=high"
    And codetags is run with "tag src/billing/charge.rs owner=billing"
    And codetags is run with "tag docs/guide.md area=docs"
    Then the tags file is:
      """
      file:docs/guide.md area=docs
      file:src/billing/charge.rs owner=billing risk=high
      file:src/main.rs risk=low
      """

  Scenario: Tags without a namespace sort before namespaced tags
    When codetags is run with "tag src/main.rs review:status=needs-audit owner=core urgent"
    Then the tags file is:
      """
      file:src/main.rs owner=core urgent review:status=needs-audit
      """

  Scenario: Tagging an item with a tag it already has changes nothing
    Given the tags file holds:
      """
      file:src/main.rs owner=core
      """
    When codetags is run with "tag src/main.rs owner=core"
    Then it exits with status 0
    And the tags file is unchanged

  Scenario: A key may hold several values
    When codetags is run with "tag src/main.rs owner=core"
    And codetags is run with "tag src/main.rs owner=platform"
    Then the tags file is:
      """
      file:src/main.rs owner=core owner=platform
      """

  Scenario: An id with a space is quoted
    Given the project also has the file "docs/My Notes.md"
    When codetags is run with the arguments:
      | tag | docs/My Notes.md | area=docs |
    Then it exits with status 0
    And the tags file is:
      """
      "file:docs/My Notes.md" area=docs
      """

  @linux @macos
  Scenario: Quotes and backslashes inside a quoted id are escaped the tagma way
    Given the project also has the file 'say "hi" \ bye.txt'
    When codetags is run with the arguments:
      | tag | say "hi" \\ bye.txt | area=fun |
    Then the tags file is:
      """
      "file:say \"hi\" \\ bye.txt" area=fun
      """

  @linux @macos
  Scenario: A backslash alone does not make an id need quoting
    Given the project also has the file 'a\b.txt'
    When codetags is run with the arguments:
      | tag | a\\b.txt | area=fun |
    Then the tags file is:
      """
      file:a\b.txt area=fun
      """

  # Sorted by rendered text, the quoted line would come first, since `"`
  # sorts before `f`.
  Scenario: Quoting never changes the sort order
    Given the project also has the file "z z.rs"
    And the project also has the file "ab.rs"
    When codetags is run with the arguments:
      | tag | z z.rs | x |
    And codetags is run with "tag ab.rs x"
    Then the tags file is:
      """
      file:ab.rs x
      "file:z z.rs" x
      """

  Scenario: A value that needs quoting is quoted, and only then
    When codetags is run with the arguments:
      | tag | src/main.rs | area="my docs" | owner="core" |
    Then the tags file is:
      """
      file:src/main.rs area="my docs" owner=core
      """

  Scenario: An unquoted value with a space is refused, with the spelling that works
    When codetags is run with the arguments:
      | tag | src/main.rs | area=my docs |
    Then it exits with status 1
    And stderr matches "area=\"my docs\""
    And the tags file is unchanged

  Scenario Outline: A tag that contains a slash or a control character is refused
    When codetags is run with the arguments:
      | tag | src/main.rs | <tag> |
    Then it exits with status 1
    And stderr matches "<message>"
    And the tags file is unchanged

    # In a Gherkin table cell `\n` is a newline, so the third row's value
    # holds a real line break inside its quotes.
    Examples:
      | tag            | message                              |
      | area="a/b"     | may not contain "/"                  |
      | "x/y"=1        | may not contain "/"                  |
      | area="a\nb"    | may not contain a control character  |

  Scenario: The file is written with LF line endings and a final newline
    When codetags is run with "tag src/main.rs owner=core"
    Then the tags file's bytes are "file:src/main.rs owner=core\n"

  Scenario: A file with CRLF line endings is read, and rewritten with LF
    Given the tags file's bytes are "file:src/main.rs owner=core\r\n"
    When codetags is run with "tag docs/guide.md area=docs"
    Then the tags file's bytes are "file:docs/guide.md area=docs\nfile:src/main.rs owner=core\n"

  Scenario: A hand-edited file is rewritten canonically by the next write
    Given the tags file holds:
      """
      file:src/main.rs   urgent  owner="core"
      "file:docs/guide.md" area=docs
      """
    When codetags is run with "tag src/billing/charge.rs risk=high"
    Then the tags file is:
      """
      file:docs/guide.md area=docs
      file:src/billing/charge.rs risk=high
      file:src/main.rs owner=core urgent
      """

  Scenario: Removing the last tag leaves an empty file
    Given the tags file holds:
      """
      file:src/main.rs owner=core
      """
    When codetags is run with "untag src/main.rs owner=core"
    Then it exits with status 0
    And the tags file's bytes are ""
