# DRAFT for human review (P6.1).
#
# Questions for review:
# Q1. `codetags tags` prints the canonical file (the same bytes a write
#     would produce), even when the file on disk is not canonical. `tags
#     <path>` prints one tag per line. Is a machine format (JSON) wanted in
#     v1, for agents?
# Q2. Listing is read-only: it never takes the lock and never rewrites the
#     file. On a file with errors it prints the errors (as `tags check`
#     would) and exits 1. OK?
# Q3. `tags <path>` for a path that no longer exists but is still in the
#     file (an orphan) prints its tags and warns on stderr. OK?

Feature: List tags from the CLI
  `codetags tags` reads `.codetags/tags` without changing it, so an agent can
  see what is tagged before it changes anything.

  Background:
    Given a project with the files "src/billing/charge.rs src/main.rs docs/guide.md"
    And the tags file holds:
      """
      file:src/main.rs  urgent owner=core
      file:docs/guide.md area=docs
      """

  Scenario: `tags` with no path prints every tagged item in canonical form
    When codetags is run with "tags"
    Then it exits with status 0
    And stdout is:
      """
      file:docs/guide.md area=docs
      file:src/main.rs owner=core urgent
      """
    And the tags file is unchanged

  Scenario: `tags <path>` prints that file's tags, one per line
    When codetags is run with "tags src/main.rs"
    Then it exits with status 0
    And stdout is:
      """
      owner=core
      urgent
      """

  Scenario: An untagged file prints nothing
    When codetags is run with "tags src/billing/charge.rs"
    Then it exits with status 0
    And stdout is empty

  Scenario: With no tags file, `tags` prints nothing
    Given there is no tags file
    When codetags is run with "tags"
    Then it exits with status 0
    And stdout is empty

  Scenario: An orphan's tags are listed, with a warning
    When the file "docs/guide.md" is deleted
    And codetags is run with "tags docs/guide.md"
    Then it exits with status 0
    And stdout is:
      """
      area=docs
      """
    And stderr matches "docs/guide\.md no longer exists; its tags are orphaned"

  Scenario: Listing a file with errors reports them and fails
    Given the tags file holds:
      """
      file:src/main.rs owner="core
      """
    When codetags is run with "tags"
    Then it exits with status 1
    And stderr matches "\.codetags/tags:1: unterminated quote"
