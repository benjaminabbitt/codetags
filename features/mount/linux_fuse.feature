@linux
Feature: Linux FUSE spike (P0b S1)
  On Linux, live views are a FUSE filesystem that an unprivileged user mounts
  through the setuid fusermount3 helper (PLAN.md C3). The spike serves a single
  file. It retires the mount risk before the view core exists (D2).

  @mount
  Scenario: Mount, list, read, and unmount as an unprivileged user
    Given the hello filesystem is mounted on an empty directory
    Then the mount lists exactly "hello.txt"
    And reading "hello.txt" through the mount gives "hello from codetags"
    When the mount is unmounted
    Then the mount directory is empty again

  Scenario: doctor names the fusermount3 that mounts will use
    When codetags is run with "doctor"
    Then stdout matches "fusermount3: /\S*fusermount3 \(setuid root: (yes|no)\)"

  Scenario: doctor flags a fusermount3 that is not setuid root
    Given a fusermount3 that is not setuid root comes first on PATH
    When codetags is run with "doctor"
    Then it exits with status 1
    And stdout matches "setuid root: no"
    And stdout matches "FUSERMOUNT_PATH"
