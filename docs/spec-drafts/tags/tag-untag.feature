# DRAFT for human review (P6.1).
#
# Questions for review:
# Q1. Command shape: `codetags tag <path> <tag>...` and
#     `codetags untag <path> <tag>...`, one path per invocation, so a path is
#     never mistaken for a tag. Is a multi-path form wanted (for example
#     `codetags tag <tag>... -- <path>...`)?
# Q2. Paths are relative to the working directory, resolved inside the
#     project, and stored as project-relative POSIX paths. Symlinks are
#     tagged as the path given, not resolved. OK?
# Q3. Only files are taggable in v1 (D6, O-12); a directory is refused. Is
#     tagging a directory (meaning every file beneath it) wanted?
# Q4. `untag` removes exact tags only. Removing every value of a key
#     (`owner=*`) has no spelling, since tagma's Tag::parse refuses `*`.
#     Wanted? If so, what spelling?
# Q5. Untagging a tag the item doesn't have exits 0 and says so on stderr,
#     so scripts stay idempotent. OK, or exit 1?
# Q6. The project is the nearest ancestor holding `.codetags/`. With none,
#     inside a git work tree, `tag` creates `.codetags/` at the work-tree
#     root (with the §2.3 `.gitignore`); outside one it fails. OK?
# Q7. Case: on case-insensitive filesystems the id uses the case the
#     directory lists, not the case typed. Not specified below; confirm.

Feature: Tag and untag files from the CLI
  Users and agents tag files (D6); the tags are queryable alongside derived
  facts. The CLI is the unprivileged baseline on every OS (D3), so every tag
  change can be made through it.

  Background:
    Given a project with the files "src/billing/charge.rs src/main.rs docs/guide.md"

  Scenario: Several tags are added in one command
    When codetags is run with "tag src/billing/charge.rs owner=billing risk=high"
    Then it exits with status 0
    And the tags file is:
      """
      file:src/billing/charge.rs owner=billing risk=high
      """

  Scenario: A path is taken relative to the working directory
    When codetags is run in "src/billing" with "tag charge.rs owner=billing"
    Then it exits with status 0
    And the tags file is:
      """
      file:src/billing/charge.rs owner=billing
      """

  Scenario: Tagging a file that does not exist is refused
    When codetags is run with "tag src/nope.rs owner=core"
    Then it exits with status 1
    And stderr matches "src/nope\.rs: no such file"
    And there is no tags file

  Scenario: Tagging a path outside the project is refused
    When codetags is run in "src" with "tag ../../elsewhere.rs owner=core"
    Then it exits with status 1
    And stderr matches "outside the project"
    And there is no tags file

  Scenario: Tagging a directory is refused
    When codetags is run with "tag src/billing owner=billing"
    Then it exits with status 1
    And stderr matches "src/billing: only files can be tagged"

  Scenario: Files under .codetags are not taggable
    When codetags is run with "tag .codetags/.gitignore owner=core"
    Then it exits with status 1
    And stderr matches "\.codetags/ is not taggable"

  Scenario: A tag with invalid tagma syntax is refused and nothing is written
    When codetags is run with "tag src/main.rs owner=core a<b"
    Then it exits with status 1
    And stderr matches "a<b"
    And there is no tags file

  Scenario: A command with no tags is a usage error
    When codetags is run with "tag src/main.rs"
    Then it exits with status 2
    And stderr matches "at least one tag"

  Scenario: Untag removes exactly the named tags
    Given the tags file holds:
      """
      file:src/main.rs owner=core owner=platform urgent
      """
    When codetags is run with "untag src/main.rs owner=core urgent"
    Then it exits with status 0
    And the tags file is:
      """
      file:src/main.rs owner=platform
      """

  Scenario: Untagging an item's last tag removes its line
    Given the tags file holds:
      """
      file:docs/guide.md area=docs
      file:src/main.rs owner=core
      """
    When codetags is run with "untag src/main.rs owner=core"
    Then the tags file is:
      """
      file:docs/guide.md area=docs
      """

  Scenario: Untagging a tag the item doesn't have changes nothing
    Given the tags file holds:
      """
      file:src/main.rs owner=core
      """
    When codetags is run with "untag src/main.rs owner=platform"
    Then it exits with status 0
    And stderr matches "src/main\.rs was not tagged owner=platform"
    And the tags file is unchanged

  Scenario: Untag matches a tag however it was quoted
    Given the tags file holds:
      """
      file:src/main.rs owner=core
      """
    When codetags is run with the arguments:
      | untag | src/main.rs | owner="core" |
    Then the tags file's bytes are ""

  Scenario: A git work tree with no .codetags gets one on the first tag
    Given a git work tree with the files "src/main.rs" and no .codetags directory
    When codetags is run with "tag src/main.rs owner=core"
    Then it exits with status 0
    And the file ".codetags/.gitignore" holds:
      """
      index/
      *.lock
      local/
      """
    And the tags file is:
      """
      file:src/main.rs owner=core
      """

  Scenario: Outside any project, tagging fails
    Given a directory with the files "a.rs" that is not in a git work tree
    When codetags is run with "tag a.rs owner=core"
    Then it exits with status 1
    And stderr matches "not in a codetags project"
