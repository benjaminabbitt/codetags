# Proxy track: using lspmux unmodified (draft)

**Status:** draft for human review, 2026-09-30. Analysis only; nothing here is
built yet. It answers whether the proxy track (brief §4.1–§4.3, PLAN.md §11)
can run on upstream lspmux with **zero changes**, and what is left if it can't.

**Checked at:** lspmux `main` @ `18861f9d59e74ece8d867772cf07fa302c2dae98`
(2026-03-11), the latest commit on the default branch. Citations below are
`file:line` at that revision. Upstream's `dev` branch is 10 commits ahead
(`f37d5cf`, 2026-03-26; an actor-model refactor); it is not analysed here.

Class markers used in §5:

- **a**: upstream already does it.
- **b**: achievable outside lspmux with zero changes: in our wrapper/shim
  before `lspmux client`, in a watcher that connects as an extra lspmux
  client, or in lspmux's config.
- **b\***: achievable outside lspmux, but only with state shared across
  sessions (a codetags-owned coordinator that every shim talks to).
- **c**: impossible without changing lspmux: an upstream PR, a local change
  (last resort), or an accepted gap.

---

## 1. Summary and recommendation

**Yes, upstream can be used unmodified.** Of the 46 requirements in §5, 11
are already done (a), 25 can be done outside lspmux (b), and 5 need
cross-session state outside lspmux (b\*). Five gaps cannot be closed without a
change (c). None of
the (c) items blocks milestone M3 (this repo's rust-analyzer through the
proxy): rust-analyzer never sends the server requests lspmux drops (§6, C-1).

Recommendation:

1. **Run upstream lspmux as a separate executable, pinned by git revision**
   (§3.1), not the crates.io release. Nothing EUPL enters this repo, so D9
   holds trivially: we exec it, never link it. No `deny.toml` exception is
   needed, and `crates/lspx` disappears from the plan.
2. **Build a BSD-3 shim** (codetags-owned; a separate process) that editors
   launch instead of the server. It handles non-LSP invocations, rewrites the
   `initialize` (root, fixed capabilities, multi-root rejection), injects
   routing-key variables, then execs or pipes through `lspmux client`. It
   answers `workspace/configuration` itself and filters a few notifications.
   This is the brief's "shim" with more duties; the multiplexing stays in
   lspmux.
3. **Run the watcher as an extra lspmux client** per instance (§5, R40).
4. **For v1, adopt a single-writer document policy** (agent sessions never
   send document contents for files an editor holds; §5, R28–R32). That
   avoids every (b\*) item in document ownership, the area where real users
   have hit corruption (upstream #1).
5. **Send one upstream PR**, written and submitted by a human (§2, maintainer
   note): answer or forward the server requests lspmux drops today (C-1, C-2).
   It's needed before gopls goes through the proxy, not before M3.
6. **Use the Unix socket on Linux and macOS** (config only), with the socket
   in a 0700 directory. On Windows, accept TCP on single-user machines, per the
   human's decision, but note C-3: it is arbitrary code execution for any local
   user, not just access to a language server.

---

## 2. Maturity and adoption (checked 2026-09-30)

| Signal | Value | Source |
|---|---|---|
| crates.io `lspmux` | 1,440 downloads all-time; 486 in the last 90 days. One version, 0.3.0 (2025-10-12, EUPL-1.2). | crates.io API |
| crates.io `ra-multiplex` (former name) | 19,111 all-time; 280 in the last 90 days. 0.1.4 (2022-02-01) to 0.2.6 (2025-10-12). MIT until 0.2.5; 0.2.6 is EUPL-1.2. | crates.io API |
| Codeberg `p2502/lspmux` | 55 stars, 13 forks, 4 watchers. Created 2025-10-12. Not archived. | Codeberg API |
| GitHub `pr2502/ra-multiplex` | 515 stars, 31 forks. Archived ("moved to codeberg"), last push 2025-10-12. | GitHub API |
| Issues | Codeberg: 2 open, 8 closed. GitHub (old repo): 1 open, 34 closed. | Codeberg API; GitHub search |
| PRs | Codeberg: 1 open, 8 closed. GitHub: 53 closed, 0 open. | same |
| Release cadence | 0.1.x burst in Jan–Feb 2022; then 0.2.0 2022-07, 0.2.1 2023-02, 0.2.2 2023-05, 0.2.3 2024-03, 0.2.4 2024-05, 0.2.5 2024-08, 0.2.6 and 0.3.0 2025-10-12. About 1–3 releases a year, none for 11½ months. No binary releases on Codeberg. On 2026-09-19 the maintainer wrote "i should make a new release at some point". | crates.io; Codeberg #21 |
| Last commit | `main` 2026-03-11 (`18861f9`); `dev` 2026-03-26 (`f37d5cf`). 22 non-merge commits on `main` in the last 12 months. | git |
| Contributors | 24 authors on `main`; one maintainer wrote 135 of the non-merge commits; the next has 9. | `git shortlog` |
| Upstream CI | GitHub Actions file, Linux only (`ubuntu-latest`): build, test, clippy, fmt. Whether it runs on Codeberg is unknown. | `.github/workflows/ci.yml` |
| rust-analyzer docs | No mention of `ra-multiplex` or `lspmux` in `docs/` or `editors/` at rust-analyzer `03fcb77` (2026-09-27). | sparse clone, `grep` |
| Known use | README examples for Neovim (CoC, `init.lua`) and Helix; systemd and launchd units. Issues show use with Neovim, VS Code and OpenCode on Windows (#8), Serena MCP (#12), opencode (#1), clojure-lsp (#19), gopls (#13), and typescript-language-server, pyright, lua-language-server, eslint (#21). Distro packages: README points to repology; not checked (API unreachable from here). | README; Codeberg issues |

**Reading.** It is a small, single-maintainer tool with a steady trickle of
users, and real use with editors and agent tools, including on Windows. It is
not widely adopted (hundreds of downloads a quarter), and it is not endorsed by
rust-analyzer. The maintainer responds within days and merges outside PRs (#5,
#9/#15, #10, #11, #14). Two things matter for upstream PRs:

- On #12 (Serena MCP) he wrote that he is "not personally interested in
  debugging LLM tools".
- On #21 he told a reporter that "pasting clanker output here is against
  Codeberg ToS".

So any upstream PR from us should be written and submitted by a human, and
motivated by editor use, not by agents.

---

## 3. Using upstream unmodified

### 3.1 How to obtain it

- **crates.io 0.3.0** (`cargo install lspmux --locked`): **unsuitable.** It
  predates fixes on `main` that the proxy track needs:
  - `ff5d377` (2025-10-14): servers that send a message before their
    `initialize` response fail to connect. Upstream #21 (2026-09-18) lists
    typescript-language-server, pyright and lua-language-server, two of the
    brief's targets.
  - `446ff72` (2026-02-02): the client's `processId` is replaced by lspmux's,
    so a server that follows the spec no longer exits when the first editor
    exits.
  - `112c56f` (2026-01-28): workspace roots compared by device and inode
    (V3), and Windows URI fixes (#15).
- **Pinned git revision** (recommended):
  `cargo install --locked --git https://codeberg.org/p2502/lspmux --rev 18861f9d59e74ece8d867772cf07fa302c2dae98 --root <codetags tool dir>`.
  Verified locally: 50 s cold, and `.crates.toml` records the rev (V35).
  Installing into a codetags-owned `--root` avoids clashing with a user's own
  `lspmux` on `PATH`. A justfile recipe would own this (Appendix A names are
  frozen, so `setup` would grow it, as its own task).
- **Binary pin:** none upstream; Codeberg has no releases with assets.
- **Upgrades:** `dev` refactors the core into an actor (`49878a8`), adds `stop`
  (`6116b19`) and removes `sync` again (`a3cce7e`). The next release will
  likely come from it. Re-run this analysis's checks on any bump.

### 3.2 Platforms

- Builds on Linux, macOS and Windows. Windows-specific code is the
  workspace-root identity (`instance.rs:69-83`, `winapi-util`) and drive-letter
  URI handling (`lsp.rs:258-267`).
- Transport: TCP everywhere; a Unix socket optionally on Unix
  (`config.rs:126-132`, `socketwrapper.rs:238-259`). Windows is TCP-only.
- Upstream tests: 13 unit tests, all pass on Rust 1.96 at the pinned rev. CI
  is Linux only, so macOS and Windows behaviour rests on user reports (#8, #12).

### 3.3 Config file and environment

- **Path:** `ProjectDirs::from("", "", "lspmux").config_dir()/config.toml`
  (`config.rs:205-217`). That is `$XDG_CONFIG_HOME/lspmux/config.toml`
  (default `~/.config/…`) on Linux, `~/Library/Application Support/lspmux/`
  on macOS, and `%APPDATA%\lspmux\config\` on Windows.
- **There is no override** for the path: no flag and no environment variable.
  `XDG_CONFIG_HOME` redirects it on Linux, but the daemon would pass that on to
  every language server it spawns (rust-analyzer reads its own user config
  from there), so we should not use it. codetags has to manage the user's real
  lspmux config file, and coexist with any lspmux use of the user's own.
- **A config error is silent:** `main.rs:74-86` logs at `info` and continues
  with the defaults, which are TCP 127.0.0.1:27631 and `pass_environment =
  ["*"]`. A typo such as `pass_enviroment` reverts routing to the whole
  environment (reproduced, V36). `codetags doctor` should run `lspmux config`,
  which prints the effective config, and compare it with what codetags wrote.
- **Keys:** `instance_timeout` (300 s, or `false`), `gc_interval` (10 s),
  `listen`, `connect`, `log_filters`, `pass_environment` (globs, with `!` to
  exclude; `config.rs:151-175`). Environment: `LSPMUX_SERVER` (the default
  `--server-path`, `main.rs:18-27`, `96`) and `RUST_LOG` (overrides `log_filters`).
- **Filtering happens in the client process** (`proxy.rs:18-41`): each
  `lspmux client` reads the config itself and sends the filtered environment
  in its handshake. The daemon spawns the server with that map added to its
  *own* environment (`instance.rs:597-604`), so variables not passed come from
  the daemon's environment.

### 3.4 Daemon lifecycle

- `lspmux server` must already be running; `lspmux client` does not start it
  and fails with "connection refused" (observed).
- Upstream ships a systemd user unit (`lspmux.service`) and a launchd plist
  (`lspmux.plist`); there is nothing for Windows.
- Our shim can start a detached `lspmux server` when the connection fails,
  on every OS. A race between two shims starting it is harmless: the second
  bind fails and that process exits.

### 3.5 How clients point at it

The wrapper chain, per server:

```
editor ─stdio─> wrapper script ─exec─> codetags shim (BSD) ─stdio─> lspmux client (EUPL, exec'd)
                                                                   └─ TCP / Unix socket ─> lspmux server ─> language server
```

- **Claude Code:** `.lsp.json` / `lspServers` `command` = the wrapper, and
  `workspaceFolder` = the project root.
- **VS Code:** `rust-analyzer.server.path` = the wrapper; `go.alternateTools`
  for gopls. The TS7 and Pyright questions in brief §4.1 are unchanged.
- **`--version`:** upstream does **not** pass it to the real server (V6,
  corrected): `lspmux --version` prints `lspmux 0.3.0`, and `lspmux client
  [--server-path X] --version` exits 2 ("unexpected argument"). The shim must
  pass non-LSP invocations to the real binary itself.

### 3.6 Licensing (D9)

We distribute no lspmux code and link none. The user installs it from source
with cargo; the shim talks to it over stdio, as a separate process. So
cargo-deny never sees it, the BSD-3 workspace stays clean, and the scoped
EUPL exception reserved in `deny.toml` is unnecessary. If codetags ever ships a
bundle that includes an lspmux binary, the EUPL obligations for that binary
apply to the bundle (source offer, licence text), as for any aggregated tool.

---

## 4. V1–V6 at `18861f9`

| ID | Result |
|---|---|
| V1 | **Confirmed**, and observed: with no config file, `ss` shows `LISTEN 127.0.0.1:27631`. `config.rs:27-30`, `126-132`; `defaults.toml`. Windows is TCP-only. **Worse than recorded:** the handshake names the executable, arguments and environment to spawn (`proxy.rs:55-70`, `client.rs:63-79`, `instance.rs:597-604`), so any local user who can connect can run any program as the daemon's owner. |
| V2 | **Confirmed.** `defaults.toml`; `config.rs:40-45`; filtering at `proxy.rs:18-41` (was cited as `14-35`). Upstream #20 (2026-06-25): the default defeats sharing, since `NVIM` holds a PID; the maintainer's answer is to configure exclusions. |
| V3 | **Confirmed.** `instance.rs:86-101`. |
| V4 | **Confirmed.** `client.rs:296-311`. The panic aborts only that connection's tokio task; the daemon survives. |
| V5 | **Confirmed**, with two additions. `instance.rs:908-935` (null-answered broadcasts), `937-950` (`workspace/configuration` to one arbitrary client), `952-1003` (register/unregister), `1005-1010` (everything else dropped, never answered). Additions: with no client attached, `workspace/configuration` is never answered (`947-949`); and an error *response* from a client is logged and dropped, never forwarded (`client.rs:406-408`), so the server waits forever then too. |
| V6 | **Corrected:** `--version` is not passed through. `lspmux --version` answers with lspmux's own version (CHANGELOG 0.1.x "client recognizes `--version`"); `lspmux client … --version` exits 2. The unreleased items are on `main`: PID replacement (`instance.rs:639-643`), early server messages ignored during the handshake (`718-733`), the `sync` subcommand (`main.rs:59-67`, `94`). `dev` removes `sync` again. |

---

## 5. Requirement classification

"Shim" means our BSD shim (§3.5). "Watcher client" means the watcher
connecting through `lspmux client` with the same server path, arguments,
filtered environment and workspace root as the shims, so it lands on the same
instance.

### 5.1 Client wiring and shim (brief §4.1)

| # | Requirement | Class | How / where |
|---|---|---|---|
| R1 | Shim poses as the server binary and forwards stdio to the daemon | a | `lspmux client`: `proxy.rs:43-84` |
| R2 | Non-LSP invocations (`--version`, `version`, subcommands) go to the real binary without the daemon | b | Shim detects them and execs the real binary. Upstream answers `--version` itself (V6). |
| R3 | One wrapper per server; both clients present identical binary and args | b | Wrapper scripts, as the brief already designs |
| R4 | Killing the shim only detaches that session | a | Socket EOF ends `output_task`, then `cleanup_client` closes that session's files (`client.rs:356-361`, `430-432`; `instance.rs:266-285`) |
| R5 | Claude Code and VS Code point at the wrapper | b | Configuration only (§3.5) |

### 5.2 Routing key (brief §4.2)

| # | Requirement | Class | How / where |
|---|---|---|---|
| R6 | `server` is a canonical absolute path | b | Wrapper passes `--server-path <abs>`. The key holds it verbatim (`client.rs:246-251`). |
| R7 | `args` normalized | b | Wrappers |
| R8 | `env` is a curated allowlist, never raw `PATH` | b | `pass_environment` in config (`proxy.rs:18-41`). Without `PATH`, the server is resolved against the daemon's `PATH`, so R6 matters. |
| R9 | `toolchain`, `semantic_cfg`, `host_fs` in the key | b | **Env-key injection:** the shim sets `CODETAGS_KEY_TOOLCHAIN=<hash>` and so on, and `pass_environment` admits `CODETAGS_KEY_*`. They become part of `InstanceKey.env` (`instance.rs:33-38`). Side effect: the server process sees them too; harmless. |
| R10 | Per-language project root, and the server is initialized with it | b | Shim rewrites `workspaceFolders[0]`/`rootUri`/`rootPath` in the `initialize` it passes on. lspmux takes the root verbatim (`client.rs:291-327`), and the first session's params initialize the server (`instance.rs:680-700`). |
| R11 | Different spellings of one directory don't share a server; warn | b | Env-key injection of the spelled root (`CODETAGS_KEY_ROOT=<spelling>`) refines lspmux's device+inode equality (`instance.rs:86-101`). The shim logs the warning. |
| R12 | Multi-root handshake gets an LSP error, never a panic | b | Shim answers the editor's `initialize` with an error itself and never starts `lspmux client`. Upstream's `assert!` (`client.rs:311`) stays reachable by non-shim clients but kills only that connection. |
| R13 | First `initialize` result reused for later sessions | a | `client.rs:255-267` |

### 5.3 Handshake and lifecycle

| # | Requirement | Class | How / where |
|---|---|---|---|
| R14 | Exactly one real `initialize` per server | a | `instance.rs:588-650`, `680-750` |
| R15 | Proxy's own fixed capability set | b | Shim replaces `capabilities` in every `initialize` with the fixed set; only the first reaches the server, and every shim sends the same one. lspmux passes `capabilities` through as opaque JSON (`lsp.rs:78`). |
| R16 | Answer each session's `initialize` from the cache; swallow `initialized` | a | `client.rs:255-282` |
| R17 | `shutdown`/`exit` detach only that session; stop on refcount plus idle timeout | a | `client.rs:371-382`; GC `instance.rs:521-552`; `instance_timeout` defaults to 300 s |
| R18 | Forward unknown client methods verbatim | a | Requests `client.rs:384-389`; notifications `422-426` |
| R19 | Forward unknown *server-to-client* requests | c | Dropped, never answered: `instance.rs:1005-1010`. See C-1. |
| R20 | Drop a session's own `workspace/didChangeWatchedFiles` | b | Shim filters it; lspmux would forward it (`client.rs:422-426`) |

### 5.4 Server-to-client requests

| # | Requirement | Class | How / where |
|---|---|---|---|
| R21 | `register`/`unregisterCapability`: answer, keep the globs for the watcher router | a, b | lspmux answers null, caches, broadcasts (`instance.rs:952-1003`) and replays to new clients (`199-227`). The watcher client reads the globs from the broadcast and the replay. |
| R22 | `window/workDoneProgress/create`: answer; fan `$/progress` out to sessions and the readiness gate | a | `instance.rs:908-935`; notifications broadcast at `1012-1017` |
| R23 | `workspace/configuration` from the merged configuration, per `scopeUri` | b\* | lspmux forwards it to one arbitrary client (`937-950`). Every shim and the watcher client answer it themselves, never passing it to the editor, so it doesn't matter which one lspmux picks. The response is a success response that echoes the received id, so the shim needs no knowledge of lspmux's id tags. The *merged* value needs the coordinator. |
| R24 | `workspace/applyEdit` routed to the session whose request caused it | c | Dropped at `instance.rs:1005-1010`. See C-1. |
| R25 | `window/showMessageRequest` to an interactive session, otherwise auto-dismissed | c | Same location; no client capability gates it. See C-2. |
| R26 | (not in the brief's table) `workspace/workspaceFolders`, `window/showDocument` | b | Leave `workspace.workspaceFolders` and `window.showDocument` out of the fixed capability set, so servers don't send them. gopls gates `showDocument` on the capability (gopls `settings.go:1076-1077`). Check per server. |

### 5.5 Document ownership

| # | Requirement | Class | How / where |
|---|---|---|---|
| R27 | Per-file refcount: `didOpen` once, `didClose` when the last holder closes | a | `instance.rs:319-404`; on disconnect `266-285` |
| R28 | Rewrite versions into one increasing sequence per file | b\*, or avoided | Needs a shared counter across sessions. Unnecessary under the single-writer policy (R30). |
| R29 | Drop a `didChange` whose result equals the server's content | b\*, or avoided | Needs the server-side content, i.e. the last writer across sessions. Unnecessary under single-writer. |
| R30 | Conflict policy: a dirty editor buffer wins | b | **Single-writer policy** in the shim: an agent-role shim (the wrapper passes `--role agent`) forwards no `didOpen`/`didChange`/`didClose` for a file any other session holds. It reads holders from `lspmux status --json`, which lists each client's open files (`instance.rs:406-437`). The editor then stays the only writer, as the brief prefers ("agent sessions should not open documents"). |
| R31 | No watcher events for files a session holds open | b | Watcher client consults the same status data |
| R32 | Agent sessions don't open documents (tsserver excepted) | b | Shim policy by role, with a per-server exception list |

### 5.6 Configuration merge

| # | Requirement | Class | How / where |
|---|---|---|---|
| R33 | Never re-initialize to apply configuration | a | lspmux never re-initializes |
| R34 | Push `didChangeConfiguration` only when the merged result changes | b\* | Shims drop the editors' own `didChangeConfiguration`; the coordinator sends the merged change through one client connection (the watcher client). lspmux forwards client notifications verbatim (`client.rs:422-426`). |
| R35 | RouteKey settings: a mismatch means a separate server | b | Env-key injection of the RouteKey settings' hash (as R9) |
| R36 | Restart only for settings shown to need it | b | No restart command at this rev. Kill the server's PID from `status --json`; lspmux drops the instance (`instance.rs:812-821`), and the next session respawns it. Crude, but zero-change. (`dev` adds `stop`.) |
| R37 | Persist the authority session's last answers | b\* | Coordinator state |

### 5.7 Readiness gate

| # | Requirement | Class | How / where |
|---|---|---|---|
| R38 | Hold a server's requests after a watcher flush until it is idle | b | Each shim gates its own session's requests. `experimental/serverStatus` and `$/progress` are broadcast to every client (`instance.rs:1012-1017`), so every shim sees quiescence directly. rust-analyzer sends `serverStatus` only if the first `initialize` asked for it, so it goes in the fixed capability set (R15). |

### 5.8 Crash recovery

| # | Requirement | Class | How / where |
|---|---|---|---|
| R39 | Respawn, re-handshake, replay `didOpen` with current contents, fail in-flight requests | b | Upstream drops the instance and clears its client map on exit (`instance.rs:812-821`) but does not respawn, replay or fail anything. A session only notices on its next message, when the send to the dead instance fails (`client.rs:384-389`), because `output_task` still holds a sender. The shim tracks its own in-flight requests and open documents anyway (R30). It detects the exit by polling `status --json` for the PID or by socket EOF. It then fails its in-flight requests with an error, starts a fresh `lspmux client` (which respawns the server), replays `initialize` (discarding the reply), `initialized` and its `didOpen`s. The server re-registers its watchers itself. Fallback: the shim exits, and VS Code restarts the server and replays on its own; Claude Code's restart behaviour is ⚠ unverified. |

### 5.9 Watcher routing (brief §4.3, PLAN.md §2.12)

| # | Requirement | Class | How / where |
|---|---|---|---|
| R40 | One `didChangeWatchedFiles` per batch per server, only paths matching its registered globs, spelled as that server spells its folders | b | **Watcher client** per instance: it sends the batch as a notification, which lspmux forwards verbatim to that instance (`client.rs:422-426`). Scoping is by instance key: one connection per instance. Caveats: (1) connecting when no instance exists spawns one with the watcher's `initialize`, so send the shims' fixed `initialize`, or connect only when `status` lists the instance. (2) The watcher client counts as a client, so the idle GC never fires while it is attached (`instance.rs:543-548`); detach when `status` shows it is the last client. (3) It receives every broadcast (diagnostics and so on) and must drain them, and it can be the client picked for `workspace/configuration` (answer as in R23). |
| R41 | Servers that register nothing receive nothing | b | Watcher client sends only for registered globs |
| R42 | Advertise dynamic registration only once register-and-notify works | b | Fixed capability set (R15) |

### 5.10 Security and other gaps found

| # | Requirement or finding | Class | How / where |
|---|---|---|---|
| R43 | Never listen on a non-loopback TCP port (brief §2) | b | Config only: upstream accepts any address, `0.0.0.0` included (reproduced, V36). `codetags doctor` checks the effective config. |
| R44 | Other local users cannot reach the daemon | b on Unix, c on Windows | Unix: `listen` = a socket path inside a 0700 directory (config only). The socket's own mode follows the umask (`0755` under umask 022; reproduced), so the directory is the protection. Windows: TCP only, reachable by every local user. See C-3. |
| R45 | `$/cancelRequest` reaches the server | c | Notifications are forwarded verbatim (`client.rs:422-426`), but requests' ids were rewritten (`384-389`; tag format `ext.rs:22-35`), so the server never matches a cancel. See C-4. |
| R46 | Server messages during `initialize`; no global stall | c | `instance.rs:702-733`: messages before the `initialize` response are dropped, requests included, and the instance-map lock is held throughout the handshake (upstream FIXME at `710-717`). See C-5. |

Also noted, accepted: lspmux re-serializes `InitializeParams` through a
struct with no catch-all, so unknown top-level fields such as `workDoneToken`
are dropped (`lsp.rs:52-97`). And `Listener::bind` deletes whatever is at the
socket path, even a regular file (`socketwrapper.rs:246-253`; reproduced), so
codetags must own that path.

**Tally** (46 rows; R44 is b on Unix and c on Windows, and R19 and R24 are
one gap, C-1):

- a 11: R1, R4, R13, R14, R16, R17, R18, R21, R22, R27, R33.
- b 25: R2, R3, R5–R12, R15, R20, R26, R30–R32, R35, R36, R38–R43, and R44
  on Unix.
- b\* 5: R23, R28, R29, R34, R37.
- c 5: C-1 (R19, R24), C-2 (R25), C-3 (R44 on Windows), C-4 (R45), C-5 (R46).

R28 and R29 drop out under the single-writer policy, and R23, R34 and R37 are
what a v1 coordinator would hold.

---

## 6. The (c) items: how often they bite, and what to do

| ID | Item | Bites in practice? | Recommendation |
|---|---|---|---|
| C-1 | Server-to-client requests other than the six refresh/progress methods, configuration and (un)registration are dropped and never answered (R19, R24; `instance.rs:1005-1010`; V37). | **rust-analyzer: never.** At `03fcb77` it sends no `applyEdit`; its only other request is `showMessageRequest` (C-2). **gopls: yes.** At `134264d`, `applyChanges` sends `workspace/applyEdit` with no timeout and no client-capability check (`command.go:1013-1027`). Commands that use it: `AddImport`, `RemoveDependency`, the go.mod update commands, `ExtractToNewFile`, `MoveType`, `AddTest`, `ChangeSignature`, `ImplementInterface`, `ModifyTags`, `MoveDeclaration`. Through lspmux, the editor's `workspace/executeCommand` for these never completes, though the rest of gopls keeps working. Quick fixes that VS Code resolves through `codeAction/resolve` don't use it. No upstream issue reports this; the README says server requests are dropped, and #13 ("gopls not working", unresolved) may be related. No client-side workaround: the request never reaches any client, and gopls ignores the capability. | **Upstream PR.** Forward these requests to one client with the existing `Forward` tag (the `workspace/configuration` pattern, `937-950`). Prefer the client with the most recent in-flight `workspace/executeCommand`, since JSON-RPC records no causal link. With no client, answer with an error. A few dozen lines. Needed before Go goes through the proxy; not for M3. A local change only if upstream declines and Go support can't wait. |
| C-2 | `window/showMessageRequest` is dropped (R25). | Rarely, and it doesn't hang. gopls sends it for the telemetry and vulncheck prompts, with a 15 s timeout (`prompt.go:351-375`). rust-analyzer sends it only when `open_server_logs` is set, and handles the reply asynchronously (`lsp/utils.rs:38-71`). | **Accept the gap**, and fold it into C-1's PR: forwarding to a client also covers it. |
| C-3 | Windows has only TCP, so any local user can connect, and the handshake names any executable to spawn as the daemon's owner (R44; V1). | Only on multi-user Windows hosts (shared workstations, terminal servers). Irrelevant on a single-user machine. No upstream issue. | **Human decision.** The human ruled TCP acceptable. This records the extent: local code execution as the daemon's owner, not just language-server access. Accept for single-user Windows and say so in `codetags doctor`. Propose a named-pipe transport upstream only if multi-user Windows matters; that's a larger PR (it touches `socketwrapper.rs` and `config.rs`). On Unix, use the socket (R44). |
| C-4 | `$/cancelRequest` is never matched (R45). | Constantly, but harmlessly: VS Code cancels hover, completion and similar requests as you type. The server finishes work it could have dropped, and its late response goes back to the client, which ignores it. Correctness is unaffected. | **Accept.** The shim can answer the editor at once with `RequestCancelled` (-32800) and discard the late response, so the editor feels no delay. Optional tiny upstream PR: tag the id inside `$/cancelRequest` params at `client.rs:422`. |
| C-5 | During a server's `initialize`, its messages are dropped and every other connection waits on the map lock (R46). | Only with servers that are slow to answer `initialize`. The upstream FIXME names kotlin-language-server. rust-analyzer, gopls, pyright and TS7 answer promptly. Before `ff5d377` it broke connections outright (#19, #21); now the messages are only lost. | **Accept**, and re-check on the next upstream bump: `dev`'s actor refactor (`49878a8`) restructures this code. |

### Riskiest items

1. **C-1 for Go.** Silent hangs of specific gopls commands, with no
   workaround outside lspmux. It does not affect rust-analyzer (M3).
2. **C-3, if multi-user Windows is in scope.** It is a security property, not
   a functional gap.
3. **Not (c), but the highest implementation risk: the b\* document-ownership
   items, if the single-writer policy is rejected.** Upstream #1 (2025-10-21,
   11 comments) is real corruption: persistent phantom syntax errors when
   Neovim and opencode edit the same file. Upstream's mitigation is the manual
   `sync` command, which `dev` removes again. Building multi-writer ownership
   outside lspmux means a coordinator that sees every session's document
   traffic, which amounts to a second multiplexer. If multi-writer is
   required, an upstream design discussion is the better route.

---

## 7. What we would build (all BSD-3, all outside lspmux)

- **Wrapper scripts**, one per server (brief §4.1), calling the shim with the
  real server's absolute path and a role (`editor` or `agent`).
- **The shim** (a `codetags` subcommand or a small binary):
  - passes non-LSP invocations to the real binary (R2);
  - rewrites `initialize`: the root (R10), the fixed capabilities (R15, R26,
    R42), and multi-root rejection (R12);
  - sets the `CODETAGS_KEY_*` variables (R9, R11, R35);
  - starts `lspmux server` if it isn't running (§3.4);
  - filters `didChangeWatchedFiles` and `didChangeConfiguration` (R20, R34);
  - answers `workspace/configuration` (R23);
  - enforces the single-writer policy (R30, R32);
  - gates its requests on readiness (R38);
  - recovers from crashes (R39);
  - answers cancellations itself (C-4).
- **The watcher client** (R40–R42, R31), part of `codetags-watch`'s router.
- **A small coordinator**, only for the merged configuration (R23, R34, R37).
  It could live in `codetagsd`, or start as a file the shims read.
- **`codetags doctor` checks:** lspmux installed at the pinned rev; effective
  config (`lspmux config`) matches ours; listen address loopback or a socket in
  a 0700 directory (§3.3, R43, R44).

## 8. Open questions for the human

1. Accept the single-writer document policy for v1 (R30), instead of the
   brief's multi-writer reconciliation (version rewriting, dedupe)?
2. C-3: Is multi-user Windows in scope? If not, record the accepted risk.
3. C-1: Will a human author and submit the upstream PR? The maintainer's
   stance on AI-generated contributions is in §2.
4. codetags would write the user's own lspmux config file (§3.3), since it
   has no path override. Is that acceptable, or should codetags refuse to
   proceed when one already exists with other content?
5. Pin by git rev (recommended), or wait for the release the maintainer
   mentioned on 2026-09-19 and pin that?
