> **Amended.** Human decisions made after this brief was drafted are recorded in
> `PLAN.md` §1 and take precedence where they conflict. The text below is the
> original brief, unchanged.

# BRIEF: shared LSP proxy, derived code index, filesystem views

Draft brief from design discussion, 2026-09-30. Written for a coding agent.

**Markers used throughout**

- ⚠ = unverified or verified only against an older version. Verify before building on it and record the result in `docs/verification.md`.
- **[OPEN]** = decision pending with the human. Build only the stated default; do not build past it.
- Code blocks marked *sketch* are illustrative, not reviewed or compiled.

---

## 1. Goal

Make coding agents better at architectural reasoning over large codebases in Python, TypeScript, Rust, and Go. Three parts:

1. **Proxy.** A shared LSP proxy so Claude Code and VS Code use one warm language server per project, and every server is told about file changes its client currently misses.
2. **Index.** A derived index in DuckDB of symbols, call sites, and resolved call targets, with provenance on every edge. Source files remain the source of truth; the index is always rebuildable.
3. **Views.** Read-only views of the index as files that agents read with `ls`/`cat`/`grep`: first a static generated tree, later (gated) a FUSE mount whose paths are tagma postfix queries.

**Non-goals:** the database as source of truth; editing code through views; embeddings or vector search; any network-exposed service; Pylance.

## 2. Hard constraints

- **Unprivileged by default.** An optional privileged mode may add capabilities; nothing may require it.
- **Linux (Debian) is the primary target.** macOS and Windows are designed for but out of v1. **[OPEN]** whether v1 includes either.
- **Proxy base:** fork or extend lspmux (EUPL-1.2 accepted), <https://codeberg.org/p2502/lspmux>. Rust.
- **Clients:** Claude Code and VS Code, on the same machine or different machines.
- **Local transport only:** Unix socket. For cross-machine use, forward the socket over SSH. Never listen on a non-loopback TCP port. Language servers execute project code (rust-analyzer builds proc macros and runs build scripts), so a network-reachable proxy is remote code execution.
- **Path identity:** clients sharing a server must use identical file paths. Never rewrite URIs inside messages.

## 3. Architecture

```
Claude Code ─stdio─> shim ─┐                                    ┌─> rust-analyzer   (one per routing key)
VS Code ext ─stdio─> shim ─┴─ unix socket ─> proxy daemon ──────┼─> gopls
                                              ▲    │            ├─> pyright / basedpyright
                                              │    │            └─> TS server (see §4.1)
watcher (inotify | fanotify) ─> coalescer ────┘    │
                                    │              └─ readiness gate, doc ownership, config merge
                                    └─> indexer (SCIP, go/callgraph, Jelly) ─> DuckDB
                                                                                 └─> views: static tree → FUSE + tagma
```

The **index/views track** (§5, P1 → P2 → P5) and the **proxy track** (P3 → P4) are independent and can proceed in parallel.

---

## 4. Components

### 4.1 Client wiring and shim

- The shim poses as the language-server binary and forwards stdio to the daemon socket.
- Invocations that are not LSP sessions (`--version`, `version`, other subcommands) must be passed straight to the real binary without contacting the daemon. VS Code's rust-analyzer extension runs `<server.path> --version` and fails if that fails.
- Provide one wrapper script per server, so both clients present identical binaries and args (both are part of the routing key):
  ```sh
  #!/bin/sh
  exec lspx client --server rust-analyzer "$@"   # lspx = working name
  ```

**Claude Code.** Configure through a plugin's `.lsp.json` or `lspServers` (`command`, `args`, `env`, `workspaceFolder`). Claude Code runs every server over stdio and kills the server process when shutdown times out, so killing the shim must only detach that session. Set `workspaceFolder` to the project root.
Docs: <https://code.claude.com/docs/en/plugins-reference>

**VS Code.**

| Language | Setting | Status |
|---|---|---|
| Rust | `rust-analyzer.server.path`, `rust-analyzer.server.extraEnv` | Works |
| Go | `go.alternateTools: { "gopls": "<wrapper>" }` | Works; ⚠ confirm the extension's `gopls version` probes pass through |
| TypeScript | Built-in TS support uses tsserver's own protocol, not LSP, so it cannot be proxied. TypeScript 7 (released 2026-07-08) has a native LSP server and a dedicated VS Code extension. | ⚠ Whether the TS7 extension accepts a custom server path is unknown. If not, VS Code keeps its own TS server and only Claude Code's goes through the proxy. Point Claude Code at the TS7 native server, not typescript-language-server, so both could share. |
| Python | Pylance's license limits it to Microsoft products; never proxy it. Use Pyright or basedpyright. | ⚠ Whether either VS Code extension accepts a custom server command is unknown. |

### 4.2 Proxy daemon (lspmux fork)

#### Routing key

lspmux keys instances on `{server, args, env, workspace_root}`. ⚠ The behaviour below was read in ra-multiplex 0.2.6 source; recheck the current lspmux tree.

- `workspace_root` is taken verbatim from the client's single workspace folder (falling back to `rootUri`, then the shim's working directory), with no project-root discovery and no symlink resolution.
- A handshake with more than one workspace folder hits an `assert!` and panics the connection task.
- `env` includes only variables listed in `pass_environment`, which is empty by default.
- The first client's `initialize` result is reused for all later clients.

Extend it as follows:

```rust
// sketch
pub struct InstanceKey {
    pub server: String,                 // canonicalized absolute binary path
    pub args: Vec<String>,              // normalized by wrapper scripts
    pub env: BTreeMap<String, String>,  // curated allowlist, never raw PATH
    pub workspace_root: String,         // per-language project root (below)
    pub toolchain: String,              // hash of resolved `rustc -vV` / `go version` / interpreter path
    pub semantic_cfg: u64,              // hash of RouteKey-class settings only (§4.2 config)
    pub host_fs: String,                // host + filesystem identity
}
```

Normalize the root per language, then resolve symlinks with `canonicalize`:

| Language | Project root |
|---|---|
| Go | Nearest `go.work` anywhere above; otherwise nearest `go.mod`. Respect `GOWORK`. |
| Rust | Outermost `Cargo.toml` declaring a `[workspace]` that contains the crate. |
| TypeScript | Nearest `tsconfig.json`; otherwise `package.json`. |
| Python | Nearest `pyproject.toml` or `pyrightconfig.json`; the interpreter path goes into the key. |

- Initialize each server with the normalized root, not whatever the first client reported.
- If two clients' literal paths resolve to the same place but are spelled differently (for example via a symlink), do **not** share a server. Log a warning instead.
- **Multi-root workspaces.** Default: reject gracefully with an LSP error, never panic. **[OPEN]** the full approach: reuse a server whose folders are a superset, adding folders via `workspace/didChangeWorkspaceFolders` where the server supports it.

#### Handshake and lifecycle

- Send exactly one real `initialize` per server (the protocol allows only one). Use the proxy's own fixed capability set:
  - `workspace.didChangeWatchedFiles.dynamicRegistration` and `relativePatternSupport`
  - `window.workDoneProgress`
  - `general.positionEncodings: ["utf-16"]` (UTF-16 is mandatory; VS Code offers nothing else)
  - rust-analyzer's opt-in status notification (⚠ exact capability path; believed to be `experimental.serverStatusNotification`)
- Answer every session's `initialize` from the cached result, and swallow each session's `initialized`.
- A session's `shutdown` or `exit` detaches only that session. The server stops on reference count plus idle timeout (lspmux defaults to 300 s).
- Forward unknown and custom methods verbatim, for example rust-analyzer's `experimental/*`.
- Drop any `workspace/didChangeWatchedFiles` a session sends. The proxy's watcher is authoritative.

#### Server-to-client requests

lspmux's README says server requests are dropped, although it replays dynamic registrations. Implement:

| Server sends | Proxy does |
|---|---|
| `client/registerCapability` / `unregisterCapability` for watched files | Answer it itself; store the glob patterns for the watcher router |
| `window/workDoneProgress/create` | Answer it itself; fan `$/progress` out to sessions and to the readiness gate |
| `workspace/configuration` | Answer from the merged configuration, per `scopeUri` where given (gopls asks per workspace folder) |
| `workspace/applyEdit` | Route to the session whose in-flight request caused it (rename, code action) |
| `window/showMessageRequest` | Route to an interactive session if one is attached; otherwise auto-dismiss |

#### Document ownership

After `didOpen`, the client owns the document's contents, and opens and closes must balance (at most one open per document per server).

- Keep a per-file reference count: send `didOpen` once, and `didClose` only when the last session closes.
- Rewrite version numbers into one increasing sequence per file. Claude Code has been reported sending the same version on every change.
- Drop any `didChange` whose resulting content equals what the server already has (compare content hashes).
- Conflict policy: a dirty editor buffer wins. Agent queries on that file then reflect the unsaved editor state. Log it.
- Do not forward watcher events for files a session holds open.
- Agent sessions should not open documents, except that tsserver-based servers may need a few files open to resolve imports.

```rust
// sketch
struct DocState {
    holders: HashMap<SessionId, i32>, // session -> that session's last version
    owner: SessionId,                 // whose content the server currently has
    server_ver: i32,                  // version sequence the server sees
    content_hash: u64,
    dirty: bool,
}
```

#### Configuration merge

Never re-initialize to apply configuration. Push changes with `workspace/didChangeConfiguration` and serve the merged view when the server asks via `workspace/configuration`. Send a change only when the merged result actually differs: gopls rebuilds its internal workspace model on any option change, and redundant notifications have caused real performance regressions. Restart a server only for settings shown by testing to need it; keep that list per server.

Classify each setting with a policy:

```rust
// sketch; setting names from memory, verify against each server's docs ⚠
enum Policy { RouteKey, Union, Strongest, Authority }
// RouteKey: changes what the analysis means; a mismatch means a separate server, never a merge
// Union:    request-driven features; enable if any session wants it
// Strongest: background work pushed to all sessions; the most thorough setting wins
// Authority: default; the attached VS Code session wins, else its last persisted answers
let rust_analyzer = [("cargo.features", RouteKey), ("cargo.target", RouteKey), ("cargo.cfgs", RouteKey),
                     ("inlayHints.*", Union), ("check.command", Strongest), ("checkOnSave", Strongest)];
let gopls = [("build.buildFlags", RouteKey), ("build.env", RouteKey),
             ("ui.inlayhint.hints", Union), ("ui.diagnostic.staticcheck", Strongest)];
```

- Log unknown keys and apply `Authority` to them.
- Persist the authority session's last answers so a session with only Claude Code attached stays consistent.
- **[OPEN]** when a RouteKey setting changes mid-session. Default: move only the changing session to a matching or new instance, replaying its open documents there.

#### Readiness gate

After the watcher flushes a batch to a server, hold that server's incoming requests until it reports being idle, with a bounded timeout:

- rust-analyzer: its `experimental/serverStatus` notification with `quiescent: true`.
- Other servers: end of `$/progress`, then a cheap probe query. ⚠ Unverified per server.

This exists because servers still indexing return silently incomplete reference results. A reported case: 3 references returned cold, versus 13 across 4 files warm.

#### Crash recovery

Respawn the server, redo the handshake, replay `didOpen` for every open document with its current contents, and fail in-flight requests so sessions retry. The server will send its watcher registrations again on its own.

### 4.3 Watcher and coalescer

**Unprivileged (default): inotify.**

- One watch per directory. Scan each new directory as soon as it appears, to close the race where files land before the watch exists.
- On `IN_Q_OVERFLOW`, rescan and diff against the last snapshot.
- When `max_user_watches` is exhausted, fail with a clear error naming the sysctl.
- A deleted file's delete event arrives only after every open descriptor on it closes.
- Exclude build and dependency output: `target/`, `node_modules/`, `dist/`, `__pycache__/`, `.venv/`, and `.git/` apart from `index.lock`.

**Privileged mode: fanotify.**

- At startup, try `fanotify_init` with `FAN_REPORT_FID` and set a `FAN_MARK_FILESYSTEM` mark (available since kernel 4.20; directory events since 5.1). Events include the writer's process ID.
- On `EPERM`, fall back to inotify and log which mode is active. Unprivileged fanotify cannot set filesystem marks or see other processes' IDs, so it offers no advantage over inotify.

**Coalescer.**

- Flush after a quiet window, with a maximum-wait cap so continuous writes can't hold events back forever.
- Merge events on the same path to their net effect: create then delete → drop; create then change → create; delete then create → change; otherwise take the latest.
- On overflow, the rescan diff replaces pending events.
- Pause flushing while `.git/index.lock` exists, to approximate "checkout in progress". ⚠ This is a heuristic.

```go
// sketch of the merge rule
switch {
case prev == Created && next == Deleted: drop
case prev == Created:                    Created
case prev == Deleted && next == Created: Changed
default:                                 next
}
```

**Routing.**

- Send each server one `didChangeWatchedFiles` per batch, containing only paths matching the glob patterns it registered, spelled the way that server spells its folders.
- Servers that register nothing receive nothing.
- gopls registers `**/*.{go,mod,sum}` plus explicit directories so it is told about directory deletions.
- rust-analyzer uses client-side watching only when the client advertises dynamic registration. Advertise it only once the full register-and-notify path works, or changes will be silently dropped.
- typescript-language-server lets tsserver watch files itself by default.

**Scope.** Network filesystems are unsupported: inotify on NFS sees only local writes. Run the watcher on the host that owns the files. macOS (FSEvents) and Windows are out of v1.

### 4.4 Indexer → DuckDB

```sql
-- sketch
CREATE TABLE symbol (
  id TEXT PRIMARY KEY, lang TEXT, kind TEXT, file TEXT,
  start_line INT, end_line INT, signature TEXT, module TEXT
);
CREATE TABLE call_site (
  site_id BIGINT PRIMARY KEY, caller TEXT, file TEXT, line INT, ordinal INT,
  receiver_text TEXT, declared_target TEXT,
  dispatch TEXT,   -- static | virtual | dynamic | unknown
  source TEXT      -- scip@<sha> | lsp@<ver> | treesitter | callgraph
);
CREATE TABLE call_target (
  site_id BIGINT REFERENCES call_site,
  target TEXT,
  method TEXT      -- declared | cha | rta | vta | jelly | di-binding | name-match
);
```

| Language | Declared targets | Dispatch / gap filling | Known gaps |
|---|---|---|---|
| Go | scip-go for symbols and references | Call edges from `golang.org/x/tools/go/callgraph`: VTA; CHA for libraries without `main`. Its graph is keyed by call site, and polymorphic calls produce multiple edges per site. | One project reports call edges derived from SCIP enclosing ranges are only partial for Go, which is why x/tools supplies the edges. Reflection is not covered. |
| Rust | `rust-analyzer scip` | Expand trait and `dyn` calls to implementations via `is_implementation`. | Needs a build from 2025-11-28 or later; enclosing ranges were added then (rust-lang/rust-analyzer#21141). Filter `local N` symbols, which are exactly the discarded local scope. Test proc-macro expansion on the target crates. |
| TypeScript | scip-typescript | Union with Jelly. | tsc resolved 81% of call sites on idiomatic TypeScript versus 11% on JavaScript-style code in one analyzer project. A Jelly failure there silently cut the graph by about 81%, so make every provider failure loud. scip-typescript skips files over 1 MB unless `--max-file-byte-size` is raised. |
| Python | scip-python (Pyright-based) | Record everything Pyright can't resolve as name-match candidates, labeled with a candidate count. | 0.6.6 omits symbol info for `@dataclass` classes and many stdlib symbols, which crashed SCIP's converter on one large project. PyCG was archived in 2023; do not depend on it. |

**Rules.**

- Attribute each reference to the innermost definition whose `enclosing_range` contains it.
- Interface calls resolve to the interface method. Write the implementations as separate `call_target` rows, labeled with the method that produced them.
- Also capture references to functions used as values (callbacks, `executor.submit(self.process)`), not only call expressions.
- Record control context per call site (enclosing conditional, loop, try, transaction, `await`), plus literal arguments that name things (topics, tables, routes, event types).
- **[OPEN]** framework rules for decorator routes, DI registrations, and message publish/handle edges. Default: out of v1.
- Pin SCIP bindings. After any indexer or binding upgrade, compare edge counts per file: stale bindings once read scip-go output as zero edges with no error.
- Every indexer needs a buildable environment: dependencies installed, codegen run.

**Validation output.** Report the unresolved rate per language and per module after every run. Treat any sudden drop between runs as a provider failure, not a code change. Modules with high unresolved rates are where fan-out agents should read full source.

### 4.5 Views

#### Static tree (P2)

Materialize from DuckDB to a directory **outside every workspace**. Inside a repo, language servers would load view files as real code, VS Code would index them, and the watcher would loop on its own output.

```text
/tmp/codeviews/<repo>/                # sketch
  README.md                           # layout, facets, quoting rules; agents read this first
  files/ordering/aggregate.py.skel    # signatures + ordered call sites with file:line and provenance
  modules.dot                         # module graph with call-count edge weights
  unresolved/ordering.txt
```

- Use non-source extensions (`.skel`, `.calls`) and make the tree read-only (`chmod -R a-w`). Aider found weaker models sometimes try to edit code shown in its repo map.
- Start every file with this header:
  ```
  # VIEW of <source path> @ <short sha>, indexed <UTC time>; read-only, edit the source file
  ```
- Mark each edge's provenance so agents can judge freshness.

#### FUSE and tagma (P5, gated on P2 results)

- **Query engine:** tagma, <https://github.com/benjaminabbitt/tagma>. It is a dependency-free Rust core with a postfix wire form that is `/`-delimited, with implicit AND through the leftover-stack fold. The path under `q/` *is* the query, and every prefix of a valid postfix query is itself valid, so each directory can be walked into.
- **Prior art:** BATFS, <https://github.com/benjaminabbitt/BATFS>, for its path grammar and its collision, orphan, and trash ideas. Do not reuse its code.
- **Division of labour:** DuckDB stays the source of truth. The FUSE daemon asks tagma which ids match a path, then renders each file's contents from DuckDB.
- **Finite traversal:** listing a query directory returns result files only. Facet and operator names resolve on lookup but are never listed, so `grep -r`, `rg`, and `find` terminate. BATFS's listings were infinite.
- **Canonical names:** use dotted names (`ordering.OrderRepo.save`). A path component can never contain `/`, and Rust's `::` collides with tagma's reserved `:`.
- **Subtree queries:** tagma v1's `~` operator has no repetition, so tag every ancestor instead, using multi-valued keys (`module=ordering`, `module=ordering.service`).
- **Meta-configuration:** hide provenance with `tagma.hide:"prov:*"=true` (queries that name it still match it). Declare `kind`, `line`, and `caller` scalar with `tagma.arity`.
- **Rebuilds:** tagma's reference core has no untag operation. Default: rebuild the tagma index per change batch. **[OPEN]** adding a remove-item operation to tagma's spec instead. Benchmark the rebuild against the debounce window first.
- **Caching:** keep kernel entry and attribute cache timeouts near zero, or invalidate cached entries via libfuse's notifications (⚠ API details not re-verified), so agents never read stale views.
- **Mounting:** unprivileged, via `fusermount3`. A daemon crash yields "Transport endpoint is not connected" only under the mount, never in the working copy.
- **Shell hazards:** the root README must tell agents to single-quote query paths (`*`, `<`, `>`, `~` are shell-significant). Prefer `+` over `*` where the meaning allows.
- **[OPEN]** read-only views versus BATFS-style writes as agent annotations stored in DuckDB. Default: read-only.

Ingest example in tagma's `<id> <tag> <tag> ...` line format (*sketch*):

```text
sym.ordering.OrderService.place kind=method lang=rust module=ordering module=ordering.service calls=ordering.OrderRepo.save emits=OrderPlaced boundary=db prov:resolution=scip
site.4412 kind=callsite caller=ordering.OrderService.place target=ordering.OrderRepo.save line=58 dispatch=virtual prov:source=scip prov:candidates=6
_config tagma.hide:"prov:*"=true tagma.arity:kind=scalar tagma.arity:line=scalar tagma.arity:caller=scalar
```

### 4.6 Evaluation

- **[OPEN] which repo.** It must be one whose architecture the human knows well enough to grade answers.
- Write about 20 architectural questions with reference answers. A small set is enough early, when changes have large effects.
- Run each question under these conditions:
  - **A:** source only.
  - **B:** source plus the static view tree.
  - **C:** plus FUSE and tagma queries. Build this condition only if B beats A.
- Record correctness (graded by the human), tokens, tool calls, and wall time per run in `eval/results/*.jsonl`.

---

## 5. Phases

| Phase | Deliverable | Done when |
|---|---|---|
| P0 | Rust workspace, `justfile`, CI, `docs/verification.md` | `just check` exits 0 |
| P1 | Indexer for all four languages → DuckDB (§4.4) | Resolution-rate report produced for a sample repo in each language; edge-count regression check in CI |
| P2 | Static view materializer (§4.5) + evaluation harness (§4.6) | Conditions A and B run end to end on the eval repo; results committed. **Gate for P5.** |
| P3 | Proxy fork: routing key, handshake, server-to-client requests, document ownership, configuration merge (§4.2) | Integration tests: VS Code-style and Claude Code-style scripted sessions share one rust-analyzer and one gopls; no panic on multi-root |
| P4 | Watcher + coalescer + readiness gate (§4.3) | Files written by `sed -i`, `git checkout`, and a formatter all reach the server; find-references after an external edit matches a cold-start ground truth |
| P5 | FUSE views on tagma (§4.5), only if P2 shows B > A | Condition C runs; `rg -r` over the mount terminates; rebuild time is within the debounce budget |

## 6. Operating rules

1. **Sources of truth:** this brief, then tests, then code. On a conflict, stop and report it. Do not resolve **[OPEN]** items yourself.
2. **TDD.** Write a failing test first. A task is complete only when `just check` exits 0.
3. **Commits:** one task per commit, with conventional-commit messages.
4. **Verification:** check every ⚠ fact before code depends on it; record the outcome and source in `docs/verification.md`.
5. **Performance:** no optimization without a benchmark that fails its target first.
6. **Safety:** never bind TCP beyond loopback; never run a language server as root.

## 7. Known quirks from issue trackers

| Where | Issue |
|---|---|
| Claude Code LSP client | Omits the watched-files capability and sends nothing for files changed outside its own Write/Edit (#76870, #85225). Sends the same version on every `didChange` (reported in #64239, citing #30622). Returns an edit's result without waiting for diagnostics (#93321). |
| Serena | Watched-files notification was defined but never sent, so a warm server answered from pre-edit state (#1718). |
| gopls | Any option change creates a new internal workspace model (View); was patched to ignore unchanged configuration. |
| scip bindings | Stale bindings that predated the typed range fields read scip-go output as zero references and dropped every call edge. |
| lspmux | Drops server-to-client requests; multi-root handshakes hit an assert (⚠ read in 0.2.6 source). |

## 8. References

- LSP 3.17 specification: <https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/>
- lspmux: <https://codeberg.org/p2502/lspmux>. ra-multiplex 0.2.6 client source: <https://docs.rs/crate/ra-multiplex/0.2.6/source/src/client.rs>
- Claude Code plugins reference: <https://code.claude.com/docs/en/plugins-reference>
- Claude Code issues: <https://github.com/anthropics/claude-code/issues/76870>, <https://github.com/anthropics/claude-code/issues/85225>, <https://github.com/anthropics/claude-code/issues/64239>, <https://github.com/anthropics/claude-code/issues/93321>
- Serena issue: <https://github.com/oraios/serena/issues/1718>
- SCIP: <https://github.com/scip-code/scip>. rust-analyzer enclosing ranges: <https://github.com/rust-lang/rust-analyzer/pull/21141>
- scip-python gaps: <https://github.com/sourcegraph/scip-python/issues/223>
- Go call graphs: <https://pkg.go.dev/golang.org/x/tools/go/callgraph>
- Jelly: <https://github.com/cs-au-dk/jelly>
- typescript-language-server watch events PR (⚠ merge status): <https://github.com/typescript-language-server/typescript-language-server/pull/1057>
- fanotify_init(2): <https://man7.org/linux/man-pages/man2/fanotify_init.2.html>
- tagma: <https://github.com/benjaminabbitt/tagma>. BATFS: <https://github.com/benjaminabbitt/BATFS>
