@macos
Feature: macOS NFSv3 loopback spike (P0b S2)
  On macOS, live views are served by an in-process NFSv3 server bound to
  127.0.0.1 and mounted with the built-in mount_nfs, unprivileged, onto a
  directory the user owns (PLAN.md C3, §2.8). No kext, no install. The spike
  serves a single file; it retires the mount risk before the view core exists
  (D2), and measures how stale the NFS client's caches can make a view (R4).

  The NFS server cannot push invalidations, so every mount uses actimeo=0.
  macOS's NFS client still caches a failed lookup (a negative name cache
  entry) until it sees the directory's mtime change.

  @mount
  Scenario: Mount, list, read, and unmount as an unprivileged user
    Given the hello filesystem is mounted on an empty directory
    Then the mount lists exactly "hello.txt"
    And reading "hello.txt" through the mount gives "hello from codetags"
    When the mount is unmounted
    Then the mount directory is empty again

  @mount
  Scenario: A name added without a directory mtime change stays hidden
    Given the hello filesystem is mounted on an empty directory
    And "new.txt" is missing from the mount
    When the filesystem gains "new.txt" holding "added later"
    Then "new.txt" is still missing from the mount after 3 seconds

  @mount
  Scenario: A name added with a directory mtime bump is visible at once
    Given the hello filesystem is mounted on an empty directory
    And "new.txt" is missing from the mount
    When the filesystem gains "new.txt" holding "added later" and bumps the directory's mtime
    Then "new.txt" appears in the mount within 1 second
    And reading "new.txt" through the mount gives "added later"

  @mount
  Scenario: With nonegnamecache a name added without an mtime bump is visible at once
    Given the hello filesystem is mounted on an empty directory with the extra mount option "nonegnamecache"
    And "new.txt" is missing from the mount
    When the filesystem gains "new.txt" holding "added later"
    Then "new.txt" appears in the mount within 1 second
    And reading "new.txt" through the mount gives "added later"

  Scenario: doctor names the mount_nfs that mounts will use
    When codetags is run with "doctor"
    Then it exits with status 0
    And stdout matches "mount backend: nfs loopback"
    And stdout matches "mount_nfs: /sbin/mount_nfs"
