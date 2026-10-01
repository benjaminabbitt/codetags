# P3 re-plan under D14: upstream lspmux, unmodified (draft)

**Status:** draft for human review, 2026-09-30. Proposes replacement rows for
PLAN.md §11 (P3.1–P3.8). Nothing here is built. Source analysis:
`docs/proxy-zero-change.md` (requirements R1–R46, gaps C-1–C-5). Pin under
analysis: lspmux `main` @ `18861f9d59e74ece8d867772cf07fa302c2dae98` (V35).

## 0. The human's open questions, and the defaults this draft uses

Every row that depends on one of these says so with **[DEFAULT a]** … **[DEFAULT e]**.
If the human overturns a default, only the rows that carry its marker change.

| Q | Question | Default used here | Rows that depend on it |
|---|---|---|---|
| a | Editors-only document-content policy: only editor-role sessions send document contents; agent-role sessions never do (R30–R32), instead of the brief's multi-writer reconciliation (R28, R29) | **yes** | P3.6, P3.10 |
| b | Is multi-user Windows in scope (C-3: loopback TCP lets any local user spawn any program as the daemon's owner)? | **out of v1**: accepted risk, stated by `codetags doctor` | P3.9 |
| c | Who authors the upstream PR forwarding dropped server requests (C-1 `workspace/applyEdit`, C-2 `showMessageRequest`)? | **a human**; the gap is accepted until it lands | P3.11, P3.10 |
| d | May codetags write the user's lspmux config file (it has no path override, §3.3 of the analysis)? | **only through an explicit `codetags lsp setup`**; nothing else ever writes it | P3.9 |
| e | Pin lspmux by git rev `18861f9`, or wait for the release the maintainer mentioned? | **pin `18861f9` now**; re-run the analysis's checks on any bump | P3.1 |

## 1. What changes from the current §11

- **No `crates/lspx`, no EUPL code in the repo.** lspmux is an installed,
  pinned, separately executed tool. We never link it (D9 holds trivially), so
  the scoped EUPL exception in `deny.toml` and the `lspx` entry in PLAN.md §3
  go away.
- **The owned components** (all BSD-3):
  1. **The shim**, a new crate `crates/codetags-lsp` with its own small binary
     `codetags-lsp`. It must **not** link DuckDB: a binary that links DuckDB
     does not find the library outside cargo (PLAN.md §4, V29), and editors
     launch the shim outside cargo. It also keeps the shim's start-up cheap.
  2. **Wrappers.** One per (server, role). Proposed form: a copy or hard link
     of `codetags-lsp` named after the wrapper (`rust-analyzer`,
     `rust-analyzer-agent`), which looks up its own file stem in a
     `wrappers.toml` beside it. That avoids shell scripts, which matters on
     Windows: ⚠ Node refuses to spawn `.cmd`/`.bat` files without
     `shell: true` (Node's April 2024 security releases), so VS Code may not
     launch a `.cmd` wrapper. The brief's sh-script wrappers remain possible on
     Unix.
  3. **The watcher client**, a library module in `codetags-lsp` that P4's
     router uses to reach each lspmux instance as one extra client.
  4. **`codetags lsp setup`** (config generation) and the lspmux checks in
     **`codetags doctor`**.
  5. **Test fixtures:** a fake LSP server (test-only) and scripted-session
     steps.
- **Transport stays upstream's default, loopback TCP (D14).** The analysis
  (§1 item 6, R44) recommends a Unix socket in a 0700 directory on Linux and
  macOS. That is config only, but it departs from D14's wording, so this
  draft does not adopt it; it is review question 3.
- **Out of P3, unchanged in scope:** the merged configuration's `Union` and
  `Strongest` policies (R34, R37 beyond the authority's answers) need a
  cross-session coordinator (class b\*). This draft builds only `Authority`
  (§3, P3.6) and proposes deferring the rest (review question 4).

## 2. Proposed rows for PLAN.md §11

`## 11. P3 — proxy on upstream lspmux (D14; brief §4.1–§4.2; docs/proxy-zero-change.md)`

| ID | Tag | Task | Done when |
|---|---|---|---|
| P3.0 | SPEC | *(done)* Zero-change analysis, `docs/proxy-zero-change.md`. | Reviewed by the human; this re-plan approved. |
| P3.1 | MECH | **Pin and install lspmux. [DEFAULT e]** Record the pin (rev `18861f9d59e74ece8d867772cf07fa302c2dae98`, source URL) in `providers.toml` under a `[tools.lspmux]` table. `just setup` installs it with `cargo install --locked --git … --rev … --root <tool dir>`, where the tool dir is codetags-owned (proposed: `target/tools/` for development and CI; the user's codetags data dir for `codetags lsp setup`), so it never shadows a user's own `lspmux` on `PATH`. Delete the `lspx` carve-out from `deny.toml` and `lspx` from PLAN.md §2.1, §3 and §4. The justfile change is its own commit (PLAN.md §0.5). | `just setup` run twice is idempotent; `<tool dir>/.crates.toml` names the pinned rev; `<tool dir>/bin/lspmux --version` exits 0 on all three CI runners; `just license-check` passes with no EUPL entry in `deny.toml`. |
| P3.2 | SPEC | **`features/proxy/*.feature`** for every owned component, written against a fake LSP server so they run on all OSes without providers: non-LSP pass-through, routing-key injection, root normalization, multi-root rejection, the fixed capability set, the role policy, `workspace/configuration` answers, cancellation, crash recovery, daemon autostart, `codetags lsp setup`, the doctor checks, and the watcher client. New steps go in `docs/steps.md` (a [SPEC] change), and a new capability tag `@lspmux` for scenarios that need the installed daemon. In the same task, verify and record (next free V-numbers): Node/VS Code launching a `.cmd` wrapper on Windows; rust-analyzer and gopls answering requests for documents the session never opened (needed by [DEFAULT a]); rust-analyzer's status-notification capability path (brief ⚠); `lspmux status --json`'s output shape at the pin. | Features reviewed and approved by the human; each ⚠ above has a V-entry. |
| P3.3 | CORE | **Test harness.** A fake LSP server binary in `codetags-bdd` that answers `initialize`, records every message it receives (with the process id, so a test can count spawns), and can be scripted to send server requests and notifications, or to exit. Steps that start an isolated `lspmux server` on an ephemeral loopback port with its own config dir (lspmux has no config-path override, so the steps set `XDG_CONFIG_HOME` / `HOME` / `APPDATA` for the daemon and clients), and that drive scripted sessions over stdio. | A smoke scenario: two scripted sessions through bare `lspmux client` share one fake-server process (one spawn recorded) on all three CI runners. |
| P3.4 | CORE | **Shim core** (`crates/codetags-lsp`, no DuckDB dependency): wrapper resolution from its own file stem and `wrappers.toml`; non-LSP invocations (`--version`, `version`, any argument list the wrapper marks non-LSP) exec the real server binary directly without contacting the daemon (R2; V6 corrected: `lspmux client --version` exits 2); otherwise exec, or on Windows pipe through, `lspmux client --server-path <canonical absolute server path>` with normalized args (R3, R6, R7); routing-key variables `CODETAGS_KEY_TOOLCHAIN`, `CODETAGS_KEY_ROOT`, `CODETAGS_KEY_CFG` set in the client's environment (R9, R11, R35); start a detached `lspmux server` when the connection is refused, tolerating a race with another shim (analysis §3.4). | Scenarios: `<wrapper> --version` prints the real server's version and exits with its status while no daemon runs; two sessions with the same key share one server, two with different `CODETAGS_KEY_TOOLCHAIN` get two; with no daemon running, the first session starts one and a concurrent second session joins it; killing a shim leaves the other session working (R4). |
| P3.5 | CORE | **`initialize` rewrite.** (1) Multi-root: an `initialize` with more than one workspace folder is answered by the shim itself with an LSP error (proposed `-32602`, message naming multi-root and the brief's [OPEN]); `lspmux client` is never started (R12). (2) Root normalization per the brief's table (R10), walking up from the client's root **as the client spells it**, so the server's root and the documents' URIs share one spelling; `canonicalize` is used only to detect aliases, which keep separate servers through `CODETAGS_KEY_ROOT` and log a warning (R11). (3) The fixed capability set replaces the client's (R15): `window.workDoneProgress`, `general.positionEncodings: ["utf-16"]`, rust-analyzer's status notification, **no** `workspace.workspaceFolders` and **no** `window.showDocument` (R26), and **no** watched-files dynamic registration until P4 turns it on (R42). | Scenarios against the fake server: a two-folder `initialize` gets the error and no daemon connection is made, and the daemon is never asked to spawn; the fake server receives the normalized root and exactly the fixed capabilities whatever the client sent; two spellings of one directory (symlink) give two servers and one warning on stderr; a Rust crate inside a workspace reports the workspace root. |
| P3.6 | CORE | **Session policy. [DEFAULT a]** By role: editor-role shims forward document sync unchanged; agent-role shims forward no `didOpen`/`didChange`/`didClose`, except for servers on a per-server exception list (tsserver-based), which may open only files no other session holds (read from `lspmux status --json`) (R30–R32). Every shim answers `$/cancelRequest` itself with `RequestCancelled` (-32800) and discards the late response (C-4). Drop the session's own `didChangeWatchedFiles` (R20), but only once P4's watcher is authoritative; until then servers watch for themselves (R42). Configuration, `Authority` policy only: the shim that lspmux picks for `workspace/configuration` answers it (R23); an editor-role shim forwards it to its editor and persists the answer per (server, root) in the per-user state dir; an agent-role shim answers from the persisted answers, else `null` per item. Editor `didChangeConfiguration` is forwarded only when it differs from the persisted answers; agent sessions' are dropped (R34, reduced). | Scenarios: an agent session's `didOpen` never reaches the fake server; an editor session's does; a cancelled request is answered with -32800 immediately; a `workspace/configuration` sent while only an agent session is attached is answered from the editor's earlier answer; a repeated identical editor configuration change reaches the server once. |
| P3.7 | CORE | **Crash recovery** in the shim (R39). Detect the server's exit (socket EOF, or the PID gone from `status --json`); fail the session's in-flight requests with an error; start a fresh `lspmux client`, which respawns the server; replay `initialize` (discarding the reply), `initialized`, and the session's own `didOpen`s with current contents. Fallback: exit, so the editor restarts the server itself. | Scenario: the fake server exits mid-request; that request fails with an error, the next request succeeds, the new server process received the replayed `didOpen`, and the other attached session also recovers. |
| P3.8 | CORE | **Watcher client** (R40, R41, R31), for P4.3 to call. Connect through `lspmux client` with the same server path, args, filtered environment and root as the shims, only when `status --json` already lists that instance (so it never spawns a server with its own `initialize`); send the shims' fixed `initialize`; read watched-file registrations from the replay and broadcasts; drain every broadcast; answer `workspace/configuration` like an agent shim; send one `didChangeWatchedFiles` per batch with only the paths that match the registered globs, and nothing to servers that registered nothing; detach when it is the instance's last client, so the idle GC can fire. | Scenarios against the fake server: a batch matching its registered globs arrives as one notification holding only the matching paths; an instance with no registrations receives nothing; with no instance running, nothing is spawned; after the last shim leaves, the watcher client detaches and the instance is collected after `instance_timeout`. |
| P3.9 | CORE | **`codetags lsp setup` and doctor. [DEFAULT d] [DEFAULT b]** `codetags lsp setup` is the only code path that writes lspmux's config file. It writes: `listen`/`connect` = `127.0.0.1` with the default port; `pass_environment` = a curated allowlist (no `PATH`; plus `CODETAGS_KEY_*`) (R8, R43); the wrappers and `wrappers.toml` into a codetags-owned directory. It prints, never writes, the editor snippets: Claude Code `lspServers` (`command`, `workspaceFolder`), VS Code `rust-analyzer.server.path` and `go.alternateTools` (R5). If an lspmux config already exists with other content, it shows the difference and refuses unless `--replace`, which keeps a backup. `codetags doctor`: lspmux at the pinned rev; the effective config (`lspmux config`) equals what setup wrote, since a config error silently reverts to defaults (V36); the listen address is loopback (R43); on Windows, a notice that any local user can reach the daemon and run programs as its owner, accepted for single-user machines only (C-3). | Scenarios with an isolated config dir: setup on a fresh machine writes exactly the expected config and wrappers; setup with a foreign config refuses and changes nothing; `--replace` keeps a backup; doctor fails, naming the key, when the config has a typo (`pass_enviroment`) and when `listen` is `0.0.0.0`; doctor passes after setup. |
| P3.10 | CORE | **Integration with real servers** (the brief's P3 condition), `@providers @lspmux`: a VS Code-style session (editor role, `--version` probe first, document sync, configuration answers) and a Claude Code-style session (agent role, `workspaceFolder` = root, same version on every `didChange`, killed on shutdown timeout) share **one rust-analyzer** on a fixture crate: one server process, hover and find-references work in both, and killing the Claude Code shim leaves VS Code's session working. The same for **gopls**, where the gopls commands that send `workspace/applyEdit` are a known gap **[DEFAULT c]**: the scenario expecting such a command to complete is written, and listed in `ci/expected-skips.txt` with the reason "C-1, awaiting upstream" until the fix lands and the pin moves. A two-folder handshake gets an LSP error and nothing panics. | Green in the `providers` job on Linux, macOS and Windows. |
| P3.11 | SPEC | **Upstream PR for C-1/C-2. [DEFAULT c]** The agent writes only the problem statement, the reproduction against the fake server, and the gopls call sites (V37) into `docs/upstream/lspmux-server-requests.md`. A human writes and submits the PR, motivated by editor use (analysis §2). The gap is accepted until then; `codetags doctor` names it when gopls is configured. | The note is reviewed by the human. Nothing else is gated on the PR. |

**Done when** P3.3–P3.10's scenarios pass on all three OSes, `codetags doctor`
reports the proxy healthy on each, and M3a (§4) has run on this repo.

## 3. Requirement coverage

Every requirement the analysis classed **b** maps to a row. Classes **a** need
no work beyond being exercised by P3.10. Class **b\*** and **c** items are
shown with what this draft does about them.

| Row | Requirements (analysis §5) |
|---|---|
| P3.4 | R2, R3, R6, R7, R9, R11 (key part), R35; R4 and R1 exercised |
| P3.5 | R10, R11 (warning), R12, R15, R26, R42 (held off) |
| P3.6 | R20, R23, R30, R31 (shim half), R32, R34 (authority only), C-4 workaround |
| P3.7 | R39 |
| P3.8 | R40, R41, R31 (watcher half), R21 (glob collection) |
| P3.9 | R5, R8, R43, R44 (Unix: loopback TCP per D14; Windows: C-3 accepted) |
| P4.3 (existing) | R38 readiness gate, implemented in the shim from broadcast `serverStatus`/`$/progress`; R42 turned on |
| none (b\*, deferred) | R28, R29 (unneeded under [DEFAULT a]); R34 and R37 beyond `Authority` |
| none (c, accepted) | C-1/C-2 → P3.11; C-3 → P3.9 notice; C-4 → P3.6 workaround; C-5 accepted, re-checked on any bump; R36 restarts: not built (no setting is yet shown to need one) |

Consequences for P4 (not rewritten here): P4.3 routes through P3.8's watcher
client and implements the readiness gate inside the shim (R38); turning on
watched-files dynamic registration in the fixed capability set (R42) and
dropping clients' own `didChangeWatchedFiles` (R20) happen in P4.3, together.

## 4. Dogfooding milestone M3

M3 is split, because the shared server is useful before the watcher exists.

**M3a, after P3.9: this repo's rust-analyzer through lspmux.**

1. `just setup` installs the pinned lspmux into the tool dir (P3.1).
2. The developer runs `codetags lsp setup` once **[DEFAULT d]**. It writes the
   lspmux config and the `rust-analyzer` / `rust-analyzer-agent` wrappers,
   and prints the two snippets.
3. VS Code: `rust-analyzer.server.path` → the editor wrapper (user settings,
   not committed, since the path is per machine).
4. Claude Code: the repo carries a project-scope `lspServers` entry pointing at
   the agent wrapper by a stable per-user path, with `workspaceFolder` = the
   repo root. ⚠ Whether a committed entry can name a per-user path portably is
   to be checked in P3.2; otherwise it stays per user, like VS Code's.
5. Both attach. `lspmux status --json` shows one rust-analyzer instance with
   two clients, keyed by this repo's root.

**Checks** (recorded in `docs/verification.md` as the M3a run):

- one rust-analyzer process for this repo while both clients are attached;
- VS Code's `--version` probe succeeds through the wrapper;
- hover and find-references answer in both clients;
- killing Claude Code's shim (its shutdown timeout) leaves VS Code working;
- after both detach, the instance is collected after `instance_timeout`;
- `codetags doctor` passes.

**Known limits during M3a:** rust-analyzer watches files itself (no dynamic
registration advertised), so external edits behave as they do today; no
`applyEdit` gap applies to rust-analyzer (V37).

**M3b, after P4.3:** the watcher client feeds this repo's rust-analyzer;
dynamic registration is advertised; a file changed by `sed -i` while only
Claude Code is attached shows up in its next find-references (the brief's P4
condition, on this repo).

## 5. Done-condition per owned component

| Component | Done when |
|---|---|
| lspmux pin and install (P3.1) | `.crates.toml` names `18861f9…`; `lspmux --version` runs on all three runners; no EUPL entry left in `deny.toml` |
| Shim, `codetags-lsp` (P3.4–P3.7) | its `features/proxy` scenarios pass on all three OSes against the fake server; `cargo tree -p codetags-lsp` (run through a `just` recipe) shows no `duckdb`; a non-LSP invocation never connects to the daemon |
| Wrappers (P3.4, P3.9) | each wrapper resolves to its `wrappers.toml` entry on all three OSes; VS Code launches the Windows wrapper (⚠ checked in P3.2) |
| Multi-root rejection (P3.5) | an LSP error is returned, no lspmux connection is made, nothing panics |
| Root normalization (P3.5) | per-language root scenarios pass; symlinked spellings give separate servers and one warning |
| Watcher client (P3.8) | batch, no-registration, no-spawn and detach scenarios pass; P4.3 uses it unchanged |
| `codetags lsp setup` (P3.9) | writes only on explicit invocation; refuses to clobber a foreign config; doctor detects drift, typos and non-loopback listens |
| Integration (P3.10) | scripted VS Code-style and Claude Code-style sessions share one rust-analyzer and one gopls on all three OSes; multi-root gets an error |
| M3a | the checks in §4 recorded for this repo |

## 6. Risks found while planning

- **The daemon's environment leaks into every server.** `pass_environment`
  without `PATH` means the server runs with the **daemon's** `PATH` and other
  unpassed variables (analysis §3.3), and the daemon inherits them from
  whichever shim started it (P3.4 autostart). Two sessions with different
  toolchains get different instances (through `CODETAGS_KEY_TOOLCHAIN`) but
  could still both run with the first shim's `PATH`. Proposed mitigation, for
  review: the shim starts the daemon with a fixed, minimal environment, and
  passes the resolved toolchain explicitly (`RUSTUP_TOOLCHAIN`, `CARGO`,
  `RUSTC`, `GOROOT`, the interpreter path) through `pass_environment`.
- **Multi-root rejection breaks VS Code multi-root workspaces outright:** the
  rust-analyzer extension sends every workspace folder. That is the brief's
  default, but it is likely to surface in M3a if the developer opens a
  multi-root workspace.
- **Tests depend on an external tool build** (lspmux, about 50 s cold). P3.1
  puts it in `just setup` and the CI cache; if that is unwelcome in every
  `check` job, the proxy features need their own recipe, which changes the
  frozen Appendix A (review question 6).
- **Upstream `dev` (an actor refactor) will replace `main`.** Bumping the pin
  means re-running the analysis's V1–V6 and C-1–C-5 checks; P3.3's fake-server
  features are the regression suite for that.

## 7. Questions for review, beyond (a)–(e)

1. Is the split into a separate `codetags-lsp` binary acceptable (reason: the
   DuckDB library is not found outside cargo, V29)?
2. Wrappers as named copies of the shim plus `wrappers.toml`, rather than sh
   scripts, on every OS?
3. D14 keeps loopback TCP. The analysis recommends a Unix socket in a 0700
   directory on Linux and macOS (config only, still zero-change). Adopt it?
4. Defer the `Union` and `Strongest` configuration policies (they need a
   cross-session coordinator), building only `Authority` in v1?
5. Root normalization keeps the client's spelling and uses `canonicalize`
   only to detect aliases. The brief says to canonicalize the root, which
   would give the server a root spelled differently from the documents' URIs.
   Confirm the reading.
6. Install lspmux in `just setup` (every job) or behind a new recipe?
7. The daemon-environment mitigation in §6.
