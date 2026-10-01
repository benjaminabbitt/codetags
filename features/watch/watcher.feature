Feature: The watcher reports what changed in a project
  `codetags-watch` (brief §4.3, PLAN.md P4.1, C2) watches a project with
  notify (inotify on Linux, FSEvents on macOS, ReadDirectoryChangesW on
  Windows) and reports coalesced batches of project-relative paths, each
  created, changed or deleted. It runs the default coalescer settings.

  Every OS's backend reports a different sequence of raw events for the same
  edit, and FSEvents delivers them late. The watcher classifies each raw event
  by looking at the file and at what it last knew, so the batches are the
  same on every OS. The assertions here are therefore the same on every OS,
  and wait generously: "within N seconds" means the batches the watcher sent
  since the last assertion, merged with the coalescer's rule, must reach the
  expected changes within N seconds and then stay unchanged for one more
  second.

  Scenario: A file written in place is reported as changed
    Given a project directory holding the files "src/lib.rs README.md"
    And the project is being watched
    When the file "src/lib.rs" is written
    Then within 10 seconds the watcher reports exactly "changed:src/lib.rs"

  Scenario: New and deleted files are reported
    Given a project directory holding the files "src/lib.rs src/old.rs"
    And the project is being watched
    When the file "src/new.rs" is written
    And the file "src/old.rs" is deleted
    Then within 10 seconds the watcher reports exactly "created:src/new.rs deleted:src/old.rs"

  Scenario: An edit saved the way sed -i saves it is one change
    sed -i, and most editors' atomic save, write a temporary file beside the
    target and rename it over the target. The temporary file was created and
    deleted within one batch, so it is dropped.
    Given a project directory holding the files "src/lib.rs"
    And the project is being watched
    When the file "src/lib.rs" is replaced the way sed -i does it
    Then within 10 seconds the watcher reports exactly "changed:src/lib.rs"

  Scenario: A formatter rewriting many files reports each of them once
    Given a project directory holding 40 files in "src/fmt"
    And the project is being watched
    When a formatter rewrites every file in "src/fmt"
    Then within 10 seconds the watcher reports 40 changed files in "src/fmt" and nothing else

  Scenario: A checkout's writes are held while git's index lock exists
    git holds .git/index.lock while it writes the work tree, and renames it to
    .git/index when it is done. The batch is sent when the lock goes, even
    though the writes went quiet long before.
    Given a project directory holding the files ".git/HEAD src/a.rs src/b.rs"
    And the project is being watched
    When git takes its index lock
    And the file "src/a.rs" is written
    And the file "src/b.rs" is deleted
    And the file "src/c.rs" is written
    Then the watcher reports nothing for 3 seconds
    When git releases its index lock
    Then within 10 seconds the watcher reports exactly "changed:src/a.rs deleted:src/b.rs created:src/c.rs"

  Scenario: Changes in excluded directories are never reported
    Build output, dependencies, git's internals and codetags' own index are
    excluded; the directories holding them are not. The last write, to src/,
    shows the watcher has caught up.
    Given a project directory holding the files ".git/HEAD src/lib.rs target/debug/app node_modules/x/index.js"
    And the project is being watched
    When the file "target/debug/app" is written
    And the file "target/debug/new.o" is written
    And the file "node_modules/x/index.js" is deleted
    And the file "node_modules/y/index.js" is written
    And the file "dist/bundle.js" is written
    And the file "pkg/__pycache__/m.pyc" is written
    And the file ".venv/lib/site.py" is written
    And the file ".git/objects/ab/cdef" is written
    And the file ".git/HEAD" is written
    And the file ".codetags/index/gen-1.duckdb" is written
    And the file "src/lib.rs" is written
    Then within 10 seconds the watcher reports exactly "created:.codetags created:pkg changed:src/lib.rs"

  Scenario: Files in a new directory are reported, even those written before it was watched
    A watch on a new directory exists only some time after the directory
    does. The watcher scans each new directory as soon as it sees it, so
    files written in that gap are reported too.
    Given a project directory holding the files "src/lib.rs"
    And the project is being watched
    When the directory "src/new/deep" is created holding the files "a.rs b.rs"
    Then within 10 seconds the watcher reports exactly "created:src/new created:src/new/deep created:src/new/deep/a.rs created:src/new/deep/b.rs"

  Scenario: A deleted directory is reported with everything in it
    Given a project directory holding the files "src/lib.rs src/gone/a.rs src/gone/b.rs"
    And the project is being watched
    When the directory "src/gone" is deleted
    Then within 10 seconds the watcher reports exactly "deleted:src/gone deleted:src/gone/a.rs deleted:src/gone/b.rs"

  Scenario: After an overflow the watcher rescans and reports the difference
    notify reports an event-queue overflow (inotify's IN_Q_OVERFLOW, FSEvents'
    MustScanSubDirs) as a rescan. The watcher then rescans the project and
    reports what differs from what it last reported, marked as a rescan.
    An overflow cannot be caused on demand, so this scenario asks for the
    rescan the way an overflow does. On Windows, notify 8.2.0 reports no
    overflow at all (docs/verification.md V56): this scenario passes there,
    but a real ReadDirectoryChangesW overflow goes unnoticed until notify 9.
    Given a project directory holding the files "src/lib.rs src/old.rs"
    And the project is being watched
    When the file "src/new.rs" is written
    And the file "src/old.rs" is deleted
    And the watcher is told its events overflowed
    Then within 10 seconds the watcher reports exactly "created:src/new.rs deleted:src/old.rs"
    And a batch the watcher reported was marked as a rescan
