Feature: The privileged helper accelerates the watcher, and is never required
  `codetags-privhelper` (PLAN.md P4.4, C2, §2.8) is an optional helper an
  admin runs as root. On Linux it watches whole filesystems with fanotify and
  serves an event stream on a Unix socket: each event names a path, what
  happened to it, and the PID of the process that wrote it. The watcher uses
  the helper when its socket answers the handshake and accepts the project,
  and falls back to notify otherwise. Either way it logs which mode is active,
  and its batches are the same.

  The helper filters events per client. A client, identified by SO_PEERCRED,
  receives an event only if it could list the directory holding the path,
  checked as that client's user, never as root. So a root helper never shows
  one user another user's file names.

  The privileged scenarios start the helper with `sudo -n` (CI only, the
  privileged-linux job), while the scenarios themselves, and the watcher,
  run as the unprivileged runner user.

  Scenario: With no helper running, the watcher falls back to notify
    Given no privileged helper is listening
    And a project directory holding the files "src/lib.rs"
    And the project is being watched
    Then the watcher watches with "notify"
    When the file "src/lib.rs" is written
    And the file "src/new.rs" is written
    Then within 10 seconds the watcher reports exactly "changed:src/lib.rs created:src/new.rs"

  @linux @privileged
  Scenario: With the helper running, the watcher uses fanotify and reports the same batches
    Given the privileged helper is running
    And a project directory holding the files "src/lib.rs src/old.rs"
    And the project is being watched
    Then the watcher watches with "fanotify"
    When the file "src/lib.rs" is written
    And the file "src/new.rs" is written
    And the file "src/old.rs" is deleted
    Then within 10 seconds the watcher reports exactly "changed:src/lib.rs created:src/new.rs deleted:src/old.rs"
    When the directory "src/sub" is created holding the files "a.rs b.rs"
    Then within 10 seconds the watcher reports exactly "created:src/sub created:src/sub/a.rs created:src/sub/b.rs"

  @linux @privileged
  Scenario: A write by another process is reported with that process's PID
    Given the privileged helper is running
    And a project directory holding the files "src/lib.rs"
    And a helper client is subscribed to the project
    When another process writes the file "src/lib.rs"
    Then within 10 seconds the helper reports a write to "src/lib.rs" by that process

  @linux @privileged
  Scenario: Paths the client cannot read are never delivered
    The project belongs to the runner user, but root makes a directory in it
    that only root can read, and writes a file there. The client sees the
    directory's own name, which it can list, but nothing inside it.
    Given the privileged helper is running
    And a project directory holding the files "src/lib.rs"
    And the directory "secret" in the project is readable only by root
    And a helper client is subscribed to the project
    When root writes the file "secret/hidden.rs"
    And another process writes the file "src/lib.rs"
    Then within 10 seconds the helper reports a write to "src/lib.rs" by that process
    And the helper reported nothing under "secret"

  @linux @privileged
  Scenario: The helper refuses a root the client cannot read
    Given the privileged helper is running
    And a project directory holding the files "src/lib.rs"
    And the directory "secret" in the project is readable only by root
    When a helper client subscribes to "secret" in the project
    Then the helper refuses the subscription
