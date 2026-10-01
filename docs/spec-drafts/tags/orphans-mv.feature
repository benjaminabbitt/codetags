# DRAFT for human review (P6.1).
#
# Questions for review:
# Q1. For the CLI, an orphan is a tagged id whose file is missing from the
#     working tree. The views' `orphans.txt` (P2.3) uses the current
#     generation instead. The two can disagree between an edit and the next
#     index run. OK?
# Q2. `tags check` treats an orphan as an error (exit 1), so `just check`
#     fails until the tags are re-pointed or removed. The alternative is a
#     warning with exit 0. Which? (With the error, a branch that deletes a
#     tagged file must also untag it, in the same commit.)
# Q3. `tags mv` onto a file that already has tags merges the two sets.
#     Should it refuse instead?
# Q4. `tags mv` with two directories re-points every tagged file beneath
#     the first, for `git mv` of a directory. Wanted in v1?
# Q5. Should codetags ever re-point tags itself, e.g. from git's rename
#     detection after a pull? Proposed: no; it only reports.

Feature: Orphaned tags are reported, and re-pointed with tags mv
  A tagged file that is deleted or renamed leaves an orphan: its tags stay in
  `.codetags/tags`, are never silently dropped, and are reported (PLAN.md
  §2.4, from BATFS). `codetags tags mv <old> <new>` re-points them after a
  rename, and `untag` removes them after a deletion.

  Background:
    Given a project with the files "src/old.rs src/keep.rs"
    And the tags file holds:
      """
      file:src/keep.rs owner=core
      file:src/old.rs owner=billing risk=high
      """

  Scenario: Deleting a tagged file leaves its tags, reported as orphans
    When the file "src/old.rs" is deleted
    And codetags is run with "tags check"
    Then it exits with status 1
    And stderr matches "\.codetags/tags:2: orphan: file:src/old\.rs no longer exists \(codetags tags mv, or codetags untag\)"
    And the tags file is unchanged

  Scenario: An orphan's tags are removed with untag, by its old path
    When the file "src/old.rs" is deleted
    And codetags is run with "untag src/old.rs owner=billing risk=high"
    Then it exits with status 0
    And the tags file is:
      """
      file:src/keep.rs owner=core
      """
    When codetags is run with "tags check"
    Then it exits with status 0

  Scenario: An orphan cannot be given new tags
    When the file "src/old.rs" is deleted
    And codetags is run with "tag src/old.rs extra"
    Then it exits with status 1
    And stderr matches "src/old\.rs: no such file"
    And the tags file is unchanged

  Scenario: Other writes keep orphans
    When the file "src/old.rs" is deleted
    And codetags is run with "tag src/keep.rs urgent"
    Then the tags file is:
      """
      file:src/keep.rs owner=core urgent
      file:src/old.rs owner=billing risk=high
      """

  Scenario: After a rename, tags mv re-points the tags
    When the file "src/old.rs" is renamed to "src/new.rs"
    And codetags is run with "tags mv src/old.rs src/new.rs"
    Then it exits with status 0
    And the tags file is:
      """
      file:src/keep.rs owner=core
      file:src/new.rs owner=billing risk=high
      """
    When codetags is run with "tags check"
    Then it exits with status 0

  Scenario: tags mv onto a tagged file merges the two sets
    When codetags is run with "tags mv src/old.rs src/keep.rs"
    Then it exits with status 0
    And the tags file is:
      """
      file:src/keep.rs owner=billing owner=core risk=high
      """

  Scenario: tags mv from an untagged path fails
    When codetags is run with "tags mv src/nothing.rs src/keep.rs"
    Then it exits with status 1
    And stderr matches "src/nothing\.rs has no tags"
    And the tags file is unchanged

  Scenario: tags mv to a path that does not exist fails
    When codetags is run with "tags mv src/old.rs src/missing.rs"
    Then it exits with status 1
    And stderr matches "src/missing\.rs: no such file"
    And the tags file is unchanged

  Scenario: tags mv takes paths relative to the working directory
    When the file "src/old.rs" is renamed to "src/new.rs"
    And codetags is run in "src" with "tags mv old.rs new.rs"
    Then it exits with status 0
    And the tags file is:
      """
      file:src/keep.rs owner=core
      file:src/new.rs owner=billing risk=high
      """

  Scenario: tags mv of a directory re-points every tagged file beneath it
    When the directory "src" is renamed to "lib"
    And codetags is run with "tags mv src lib"
    Then it exits with status 0
    And the tags file is:
      """
      file:lib/keep.rs owner=core
      file:lib/old.rs owner=billing risk=high
      """
