Feature: codetags lsp setup writes lspmux's configuration, and doctor checks it
  lspmux reads one per-user config file and has no option to read another
  (docs/proxy-zero-change.md §3.3). `codetags lsp setup` is the only thing
  that writes it (PLAN.md D20): a listen address only this user can reach
  (D17: a Unix socket in a 0700 directory on Linux and macOS, loopback TCP on
  Windows), an allowlist for the environment that keys a server instance,
  and the idle timeout. It backs up a different file, shows the difference,
  and writes nothing unless confirmed. `codetags doctor` reports drift and
  an unsafe listen address. Each scenario has its own home directory, so no
  real configuration is read or touched.

  @linux @macos
  Scenario: Setup listens on a socket in a private directory on Linux and macOS
    Given an isolated home for lspmux
    When codetags is run with "lsp setup --yes"
    Then it exits with status 0
    And lspmux's config listens on a socket in a private directory
    And lspmux's config sets "pass_environment" to '["CODETAGS_KEY_*"]'
    And lspmux's config sets "instance_timeout" to '300'

  @windows
  Scenario: Setup listens on loopback TCP on Windows
    Given an isolated home for lspmux
    When codetags is run with "lsp setup --yes"
    Then it exits with status 0
    And lspmux's config sets "listen" to '["127.0.0.1", 27631]'
    And lspmux's config sets "connect" to '["127.0.0.1", 27631]'
    And lspmux's config sets "pass_environment" to '["CODETAGS_KEY_*"]'
    And lspmux's config sets "instance_timeout" to '300'

  Scenario: Setup backs up a different config and shows the difference
    Given an isolated home for lspmux
    And lspmux's config file holds "instance_timeout = 5"
    When codetags is run with "lsp setup --yes"
    Then it exits with status 0
    And stdout matches "(?m)^-instance_timeout = 5$"
    And stdout matches "(?m)^\+instance_timeout = 300$"
    And a backup of lspmux's config holds "instance_timeout = 5"
    And lspmux's config sets "instance_timeout" to '300'

  Scenario: Setup writes nothing unless confirmed
    Given an isolated home for lspmux
    And lspmux's config file holds "instance_timeout = 5"
    When codetags is run with "lsp setup" and the answer "n"
    Then it exits with status 1
    And lspmux's config file still holds "instance_timeout = 5"
    And no backup of lspmux's config exists

  Scenario: Setup writes when the answer is yes
    Given an isolated home for lspmux
    When codetags is run with "lsp setup" and the answer "y"
    Then it exits with status 0
    And lspmux's config sets "instance_timeout" to '300'

  Scenario: Setup with nothing to change writes nothing
    Given an isolated home for lspmux
    And codetags lsp setup has run
    When codetags is run with "lsp setup --yes"
    Then it exits with status 0
    And stdout matches "up to date"
    And no backup of lspmux's config exists

  Scenario: Setup refuses a listen address other users can reach
    Given an isolated home for lspmux
    When codetags is run with "lsp setup --yes --listen 0.0.0.0:27631"
    Then it exits with status 2
    And lspmux's config file does not exist

  Scenario: Doctor accepts the config setup wrote
    Given an isolated home for lspmux
    And codetags lsp setup has run
    When codetags is run with "doctor"
    Then stdout matches "(?m)^lspmux config: .* \(as codetags lsp setup writes it\)$"

  Scenario: Doctor reports a config lspmux would not parse
    Given an isolated home for lspmux
    And lspmux's config file holds "pass_enviroment = []"
    When codetags is run with "doctor"
    Then it exits with status 1
    And stdout matches "unknown key `pass_enviroment`"

  Scenario: Doctor fails when lspmux listens beyond loopback
    Given an isolated home for lspmux
    And lspmux's config file holds "listen = ['0.0.0.0', 27631]"
    When codetags is run with "doctor"
    Then it exits with status 1
    And stdout matches "listen 0\.0\.0\.0:27631 is not loopback"

  Scenario: Doctor reports drift from what setup writes
    Given an isolated home for lspmux
    And codetags lsp setup has run
    And lspmux's config file has "instance_timeout = 300" replaced by "instance_timeout = 30"
    When codetags is run with "doctor"
    Then it exits with status 1
    And stdout matches "differs from what codetags lsp setup writes"

  @lspmux
  Scenario: lspmux reads the config file setup wrote
    Given an isolated home for lspmux
    And codetags lsp setup has run
    When lspmux prints its effective config
    Then lspmux's effective config listens where codetags lsp setup wrote

  @lspmux
  Scenario: Doctor names the installed lspmux and its pinned revision
    Given an isolated home for lspmux
    When codetags is run with "doctor"
    Then stdout matches "(?m)^lspmux: .* \(rev 18861f9d59e7, the pinned revision\)$"
