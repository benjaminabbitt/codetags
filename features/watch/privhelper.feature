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
  checked as that client's user, never as root, and only if that directory is
  still the one the event came from. So a root helper never shows one user
  another user's file names. Each client's share of the helper (connections,
  checker processes, subscriptions, queued events) is capped.

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

  @linux @privileged
  Scenario: A path renamed after its event is checked against the directory the event came from
    The race of threat model F1, made deterministic. The client reads nothing
    while the project's files are rewritten, so its connection fills up and
    the helper's event thread waits with the rest queued. Root then writes in
    its own directory, and the runner renames that directory away and makes
    one it can list under the same name. The helper resolved the event's
    path before the rename, and checks it after. The check must find that
    the path now leads to another directory, and withhold the name.
    Given the privileged helper is running with "--queue 100000"
    And a project directory holding 2000 files in "flood"
    And the directory "secret" in the project is readable only by root
    And a helper client is subscribed to the project and reads nothing yet
    When a formatter rewrites every file in "flood"
    And root writes the file "secret/hidden.rs"
    And the directory "secret" is moved to "secret.old" and replaced by one the runner owns
    And another process writes the file "flood/f001.rs"
    And the helper client starts reading
    Then within 30 seconds the helper reports a write to "flood/f001.rs" by that process
    And the helper reported nothing under "secret"

  @linux @privileged
  Scenario: The helper refuses to put its socket in a directory another user can change
    Root creates, replaces and binds the socket, so its directory must be one
    only root can change (threat model F2). The runner's scratch directory is
    not.
    When the privileged helper is started with its socket in a directory the runner owns
    Then the helper refuses to start, naming that directory

  @linux @privileged
  Scenario: The helper refuses a subscription past its per-connection limit
    Given the privileged helper is running with "--max-subscriptions 1"
    And a project directory holding the files "src/lib.rs tests/a.rs"
    When a helper client subscribes to "src" and then "tests" in the project
    Then the helper refuses the subscription, saying "maximum of 1 subscriptions"
