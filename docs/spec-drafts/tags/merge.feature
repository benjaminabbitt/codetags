# DRAFT for human review (P6.1).
#
# PLAN.md §2.3 says a merge "conflicts only when both sides tagged the same
# item". That is not what git does (V70, verified with git 2.47.3): git's
# three-way merge also conflicts when the two sides change *adjacent* lines,
# including two new lines inserted into the same gap. Two branches that tag
# different items merge cleanly only when at least one unchanged line lies
# between their changes. The scenarios below specify git's real behaviour
# with no merge driver.
#
# Questions for review:
# Q1. Accept git's behaviour (adjacent items conflict; resolving means
#     keeping both lines) and document it? Or add a per-item merge driver,
#     `codetags tags merge %O %A %B`, which conflicts only when both sides
#     changed the same item differently? A driver needs a `merge.<name>.driver`
#     entry in each clone's `.git/config`, which cannot be committed; only
#     the `.gitattributes` line can. `codetags lsp setup`-style, it would be
#     an explicit `codetags tags setup-git`. The @pending-merge-driver
#     scenario at the end shows what it would do.
# Q2. git's built-in `merge=union` driver (committable in
#     `.codetags/.gitattributes`, no per-clone config) merges the adjacent
#     case cleanly, but resolves every overlapping hunk by keeping both
#     sides' lines (V70): ids get duplicated, lines fall out of order, and a
#     tag one side removed survives on the other side's copy of the line.
#     `tags check` catches the duplicates and the order, but a resolution
#     that unions the duplicate lines would resurrect removed tags.
#     Proposed: do not use union.
# Q3. `tag` and `untag` refuse to write while the file holds conflict
#     markers, rather than guessing a resolution. OK?

Feature: The tags file merges like any sorted, line-per-item text file
  The file is committed so tags diff, merge and review in git (C5). One line
  per item, sorted, keeps unrelated changes apart, and a conflict is shown
  with whole lines that `tags check` and the next write can validate.

  Background:
    Given a project with the files "a.rs b.rs c.rs m.rs n.rs z.rs"
    And the tags file holds:
      """
      file:a.rs x=1
      file:m.rs x=1
      file:z.rs x=1
      """
    And the project is committed on branch "main"

  Scenario: Tags on items separated by an untouched line merge cleanly
    Given on a new branch "left" from "main", codetags is run with "tag b.rs y=1" and the result committed
    And on a new branch "right" from "main", codetags is run with "tag n.rs y=1" and the result committed
    When branch "right" is merged into "left"
    Then the merge succeeds
    And the tags file is:
      """
      file:a.rs x=1
      file:b.rs y=1
      file:m.rs x=1
      file:n.rs y=1
      file:z.rs x=1
      """
    When codetags is run with "tags check"
    Then it exits with status 0

  Scenario: Tags on different items that sort into the same gap conflict
    Given on a new branch "left" from "main", codetags is run with "tag b.rs y=1" and the result committed
    And on a new branch "right" from "main", codetags is run with "tag c.rs y=1" and the result committed
    When branch "right" is merged into "left"
    Then the merge stops with a conflict in ".codetags/tags"
    And the tags file is:
      """
      file:a.rs x=1
      <<<<<<< HEAD
      file:b.rs y=1
      =======
      file:c.rs y=1
      >>>>>>> right
      file:m.rs x=1
      file:z.rs x=1
      """

  Scenario: Changes to two adjacent items conflict
    Given on a new branch "left" from "main", codetags is run with "tag a.rs y=1" and the result committed
    And on a new branch "right" from "main", codetags is run with "tag m.rs y=2" and the result committed
    When branch "right" is merged into "left"
    Then the merge stops with a conflict in ".codetags/tags"
    And the tags file is:
      """
      <<<<<<< HEAD
      file:a.rs x=1 y=1
      file:m.rs x=1
      =======
      file:a.rs x=1
      file:m.rs x=1 y=2
      >>>>>>> right
      file:z.rs x=1
      """

  Scenario: Both branches tagging the same item conflict on that line
    Given on a new branch "left" from "main", codetags is run with "tag m.rs owner=a" and the result committed
    And on a new branch "right" from "main", codetags is run with "tag m.rs owner=b" and the result committed
    When branch "right" is merged into "left"
    Then the merge stops with a conflict in ".codetags/tags"
    And the tags file is:
      """
      file:a.rs x=1
      <<<<<<< HEAD
      file:m.rs owner=a x=1
      =======
      file:m.rs owner=b x=1
      >>>>>>> right
      file:z.rs x=1
      """

  Scenario: Both branches making the same change merge cleanly
    Given on a new branch "left" from "main", codetags is run with "tag b.rs y=1" and the result committed
    And on a new branch "right" from "main", codetags is run with "tag b.rs y=1" and the result committed
    When branch "right" is merged into "left"
    Then the merge succeeds
    And the tags file is:
      """
      file:a.rs x=1
      file:b.rs y=1
      file:m.rs x=1
      file:z.rs x=1
      """

  Scenario: A conflicted file is reported by tags check, with each marker's line
    Given on a new branch "left" from "main", codetags is run with "tag m.rs owner=a" and the result committed
    And on a new branch "right" from "main", codetags is run with "tag m.rs owner=b" and the result committed
    And branch "right" is merged into "left"
    When codetags is run with "tags check"
    Then it exits with status 1
    And stderr matches "\.codetags/tags:2: unresolved merge conflict marker"
    And stderr matches "\.codetags/tags:4: unresolved merge conflict marker"
    And stderr matches "\.codetags/tags:6: unresolved merge conflict marker"

  Scenario: Writes are refused while the file holds conflict markers
    Given on a new branch "left" from "main", codetags is run with "tag m.rs owner=a" and the result committed
    And on a new branch "right" from "main", codetags is run with "tag m.rs owner=b" and the result committed
    And branch "right" is merged into "left"
    When codetags is run with "tag a.rs y=1"
    Then it exits with status 1
    And stderr matches "\.codetags/tags has unresolved merge conflicts; resolve them, then run codetags tags check"
    And the tags file is unchanged

  Scenario: A resolution that keeps both lines for one item is caught
    Given the tags file holds:
      """
      file:a.rs x=1
      file:m.rs owner=a x=1
      file:m.rs owner=b x=1
      file:z.rs x=1
      """
    When codetags is run with "tags check"
    Then it exits with status 1
    And stderr matches "\.codetags/tags:3: file:m\.rs is also on line 2; merge its tags into one line"

  @pending-merge-driver
  Scenario: With the per-item merge driver, items in the same gap merge cleanly
    Given the project uses the codetags merge driver for ".codetags/tags"
    And on a new branch "left" from "main", codetags is run with "tag b.rs y=1" and the result committed
    And on a new branch "right" from "main", codetags is run with "tag c.rs y=1" and the result committed
    When branch "right" is merged into "left"
    Then the merge succeeds
    And the tags file is:
      """
      file:a.rs x=1
      file:b.rs y=1
      file:c.rs y=1
      file:m.rs x=1
      file:z.rs x=1
      """
