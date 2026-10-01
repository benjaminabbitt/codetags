@windows
Feature: Windows WinFsp spike (P0b S3)
  On Windows, live views are a WinFsp filesystem. WinFsp is optional: an admin
  installs it once, and then an unprivileged user can mount (D3, PLAN.md C3).
  Without it, codetags still starts, and the static views and the CLI work.
  The spike serves a single file. It retires the mount risk before the view
  core exists (D2).

  @mount @winfsp
  Scenario: Mount, list, read, and unmount as an unprivileged user
    Given the hello filesystem is mounted on an empty directory
    Then the mount lists exactly "hello.txt"
    And reading "hello.txt" through the mount gives "hello from codetags"
    When the mount is unmounted
    Then the mount directory is empty again

  @mount @winfsp
  Scenario: doctor reports the WinFsp backend and its attribution notice
    When codetags is run with "doctor"
    Then it exits with status 0
    And stdout matches "mount backend: winfsp"
    And stdout matches "winfsp: \S.*winfsp-x64\.dll \(version \d+\.\d+\)"
    And stdout matches "WinFsp - Windows File System Proxy, Copyright \(C\) Bill Zissimopoulos"
    And stdout matches "https://github\.com/winfsp/winfsp"

  # The Windows `check` job has no WinFsp installed, so there this scenario
  # also proves that a binary with the WinFsp backend starts without the DLL.
  # CODETAGS_WINFSP=off makes it hold in the WinFsp job too.
  Scenario: Without WinFsp, doctor reports the fallback to static views
    Given the environment variable "CODETAGS_WINFSP" is "off"
    When codetags is run with "doctor"
    Then it exits with status 0
    And stdout matches "mount backend: winfsp"
    And stdout matches "winfsp: (disabled|not installed|not loadable)"
    And stdout matches "fallback: static views and the codetags CLI"
