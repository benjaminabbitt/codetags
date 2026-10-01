Feature: The coalescer merges watcher events into batches
  The coalescer (brief §4.3, PLAN.md P4.2) is pure logic over a fake clock.
  Events on the same path merge to their net effect: create then delete is
  dropped, create then change is a create, delete then create is a change,
  and otherwise the latest event wins. A batch is flushed once the project
  has been quiet for the quiet window, or once the oldest pending event has
  waited the maximum wait, so continuous writes cannot hold events back
  forever. While git's index lock exists, flushing pauses (a heuristic for
  "checkout in progress"), and the batch is flushed as soon as the lock goes.
  Times are milliseconds after the scenario's start.

  Scenario Outline: Events on the same path merge to their net effect
    Given a coalescer with a quiet window of 100 ms and a maximum wait of 1000 ms
    When at 0 ms the watcher sees "src/lib.rs" <first>
    And at 10 ms the watcher sees "src/lib.rs" <second>
    Then at 200 ms the coalescer flushes exactly "<batch>"

    Examples:
      | first   | second  | batch              |
      | created | deleted |                    |
      | created | changed | created:src/lib.rs |
      | created | created | created:src/lib.rs |
      | deleted | created | changed:src/lib.rs |
      | deleted | changed | changed:src/lib.rs |
      | deleted | deleted | deleted:src/lib.rs |
      | changed | deleted | deleted:src/lib.rs |
      | changed | changed | changed:src/lib.rs |
      | changed | created | created:src/lib.rs |

  Scenario: A dropped path that reappears is reported as created
    Given a coalescer with a quiet window of 100 ms and a maximum wait of 1000 ms
    When at 0 ms the watcher sees "notes.txt" created
    And at 10 ms the watcher sees "notes.txt" deleted
    And at 20 ms the watcher sees "notes.txt" created
    Then at 200 ms the coalescer flushes exactly "created:notes.txt"

  Scenario: Events on different paths stay separate
    Given a coalescer with a quiet window of 100 ms and a maximum wait of 1000 ms
    When at 0 ms the watcher sees "a.rs" changed
    And at 5 ms the watcher sees "b.rs" deleted
    And at 6 ms the watcher sees "c.rs" created
    Then at 200 ms the coalescer flushes exactly "changed:a.rs deleted:b.rs created:c.rs"

  Scenario: Nothing is flushed until the quiet window has passed
    Given a coalescer with a quiet window of 100 ms and a maximum wait of 1000 ms
    When at 0 ms the watcher sees "a.rs" changed
    Then at 99 ms the coalescer flushes nothing
    And at 100 ms the coalescer flushes exactly "changed:a.rs"
    And at 500 ms the coalescer flushes nothing

  Scenario: Each new event restarts the quiet window
    Given a coalescer with a quiet window of 100 ms and a maximum wait of 1000 ms
    When at 0 ms the watcher sees "a.rs" changed
    And at 80 ms the watcher sees "b.rs" changed
    Then at 150 ms the coalescer flushes nothing
    And at 180 ms the coalescer flushes exactly "changed:a.rs changed:b.rs"

  Scenario: Continuous writes are flushed at the maximum wait
    Given a coalescer with a quiet window of 100 ms and a maximum wait of 300 ms
    When at 0 ms the watcher sees "a.rs" changed
    And at 90 ms the watcher sees "b.rs" changed
    And at 180 ms the watcher sees "c.rs" changed
    And at 270 ms the watcher sees "d.rs" changed
    Then at 299 ms the coalescer flushes nothing
    And at 300 ms the coalescer flushes exactly "changed:a.rs changed:b.rs changed:c.rs changed:d.rs"

  Scenario: The maximum wait counts from the oldest event of the next batch
    Given a coalescer with a quiet window of 100 ms and a maximum wait of 300 ms
    When at 0 ms the watcher sees "a.rs" changed
    Then at 100 ms the coalescer flushes exactly "changed:a.rs"
    When at 1000 ms the watcher sees "b.rs" changed
    And at 1090 ms the watcher sees "c.rs" changed
    Then at 1150 ms the coalescer flushes nothing
    And at 1190 ms the coalescer flushes exactly "changed:b.rs changed:c.rs"

  Scenario: Flushing pauses while git's index lock exists, and resumes when it goes
    Given a coalescer with a quiet window of 100 ms and a maximum wait of 300 ms
    When at 0 ms git takes its index lock
    And at 10 ms the watcher sees "src/a.rs" changed
    And at 20 ms the watcher sees "src/b.rs" deleted
    Then at 500 ms the coalescer flushes nothing
    And at 5000 ms the coalescer flushes nothing
    When at 5001 ms git releases its index lock
    Then at 5001 ms the coalescer flushes exactly "changed:src/a.rs deleted:src/b.rs"

  Scenario: Events after the index lock goes start a new batch
    Given a coalescer with a quiet window of 100 ms and a maximum wait of 300 ms
    When at 0 ms git takes its index lock
    And at 10 ms the watcher sees "a.rs" changed
    And at 50 ms git releases its index lock
    Then at 50 ms the coalescer flushes exactly "changed:a.rs"
    When at 60 ms the watcher sees "b.rs" changed
    Then at 100 ms the coalescer flushes nothing
    And at 160 ms the coalescer flushes exactly "changed:b.rs"

  Scenario: A stale index lock stops pausing after the lock cap
    A git process that crashed leaves index.lock behind. Without a cap the
    coalescer would hold every event until someone deletes it.
    Given a coalescer with a quiet window of 100 ms, a maximum wait of 300 ms and an index-lock cap of 2000 ms
    When at 0 ms git takes its index lock
    And at 10 ms the watcher sees "a.rs" changed
    Then at 1999 ms the coalescer flushes nothing
    And at 2000 ms the coalescer flushes exactly "changed:a.rs"

  Scenario: A rescan's difference replaces the pending events
    Given a coalescer with a quiet window of 100 ms and a maximum wait of 1000 ms
    When at 0 ms the watcher sees "a.rs" changed
    And at 10 ms the watcher sees "b.rs" created
    And at 20 ms a rescan finds "deleted:a.rs created:c.rs"
    Then at 120 ms the coalescer flushes exactly "deleted:a.rs created:c.rs"
    And the flushed batch is marked as a rescan

  Scenario: Events after a rescan merge with its difference
    Given a coalescer with a quiet window of 100 ms and a maximum wait of 1000 ms
    When at 0 ms a rescan finds "created:c.rs"
    And at 10 ms the watcher sees "c.rs" deleted
    And at 20 ms the watcher sees "d.rs" changed
    Then at 120 ms the coalescer flushes exactly "changed:d.rs"
    And the flushed batch is marked as a rescan
