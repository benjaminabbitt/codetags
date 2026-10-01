# DRAFT for human review (P6.1).
#
# Questions for review:
# Q1. A writer waits for `tags.lock` rather than failing at once (unlike the
#     generation store's write.lock, which refuses a second writer). Default
#     wait: 10 s, then fail. Is waiting right for tags, and is 10 s right?
# Q2. The wait is set by the environment variable
#     CODETAGS_TAGS_LOCK_TIMEOUT_MS, so tests need not wait 10 s. Is an
#     environment variable acceptable, or should it be a config key?
# Q3. The temporary file is written in `.codetags/local/` (already ignored,
#     §2.3) and renamed over `.codetags/tags`. Both are in one directory
#     tree on one filesystem, so the rename is atomic. A writer removes stale
#     temporary files it finds there. OK, or keep the temp file beside
#     `tags` and add `*.tmp` to `.codetags/.gitignore`?
# Q4. Each writer re-reads the file after taking the lock, so a change made
#     while it waited (including a hand edit or a `git pull`) is kept. A
#     hand edit made *during* another writer's critical section can still
#     be lost; that is accepted, since editors don't take the lock.

Feature: Concurrent writers never lose a change
  Every write takes `.codetags/tags.lock`, re-reads the file, applies its
  change, writes a temporary file, and renames it over `.codetags/tags`
  (PLAN.md §2.3), retrying the rename on a Windows sharing violation. Readers
  never take the lock and always see a whole file.

  Background:
    Given a project with the files "a.rs b.rs c.rs d.rs e.rs f.rs g.rs h.rs"

  Scenario: Eight concurrent writers tagging different files all land
    When codetags is run concurrently, once with each of:
      | tag a.rs n=1 |
      | tag b.rs n=2 |
      | tag c.rs n=3 |
      | tag d.rs n=4 |
      | tag e.rs n=5 |
      | tag f.rs n=6 |
      | tag g.rs n=7 |
      | tag h.rs n=8 |
    Then every run exits with status 0
    And the tags file is:
      """
      file:a.rs n=1
      file:b.rs n=2
      file:c.rs n=3
      file:d.rs n=4
      file:e.rs n=5
      file:f.rs n=6
      file:g.rs n=7
      file:h.rs n=8
      """

  Scenario: Concurrent writers tagging the same file all land
    When codetags is run concurrently, once with each of:
      | tag a.rs x=1 |
      | tag a.rs x=2 |
      | tag a.rs x=3 |
      | tag a.rs x=4 |
    Then every run exits with status 0
    And the tags file is:
      """
      file:a.rs x=1 x=2 x=3 x=4
      """

  Scenario: A concurrent tag and untag of different tags both land
    Given the tags file holds:
      """
      file:a.rs old
      """
    When codetags is run concurrently, once with each of:
      | untag a.rs old |
      | tag a.rs new   |
    Then every run exits with status 0
    And the tags file is:
      """
      file:a.rs new
      """

  Scenario: A writer waits for a lock that is released in time
    Given another process holds the tags lock for 500 ms
    When codetags is run with "tag a.rs x=1"
    Then it exits with status 0
    And the tags file is:
      """
      file:a.rs x=1
      """

  Scenario: A writer gives up on a lock held too long, and changes nothing
    Given the environment variable "CODETAGS_TAGS_LOCK_TIMEOUT_MS" is "200"
    And the tags file holds:
      """
      file:b.rs y=1
      """
    And another process holds the tags lock
    When codetags is run with "tag a.rs x=1"
    Then it exits with status 1
    And stderr matches "another writer holds .*\.codetags[/\\]tags\.lock"
    And the tags file is unchanged

  Scenario: Reading does not wait for the lock
    Given the tags file holds:
      """
      file:b.rs y=1
      """
    And another process holds the tags lock
    When codetags is run with "tags"
    Then it exits with status 0
    And stdout is:
      """
      file:b.rs y=1
      """

  Scenario: A writer killed mid-write leaves the old file whole, and the lock free
    Given the tags file holds:
      """
      file:b.rs y=1
      """
    And a writer process was killed after writing its temporary file and before renaming it
    When codetags is run with "tag a.rs x=1"
    Then it exits with status 0
    And the tags file is:
      """
      file:a.rs x=1
      file:b.rs y=1
      """
    And git reports no untracked files under ".codetags"
