# codetags — Implementation Plan

Plan for building the system described in `docs/BRIEF.md`, as amended by §1
below. It is written to be driven by coding agents, following the conventions
of tagma's `PLAN.md`:

- Every task has explicit inputs, a machine-checkable done-condition, and a tag:
  - **[MECH]**: mechanical.
  - **[CORE]**: implementation whose algorithm is given here or in the brief.
  - **[SPEC]**: needs judgment. Produce Gherkin for human review, and escalate if it is ambiguous.
- Markers are as in the brief:
  - ⚠ = unverified; record the check in `docs/verification.md`.
  - **[OPEN]** = pending with the human; build only the stated default.
  - *sketch* = illustrative only.

---

## 0. Operating rules

1. **Sources of truth, highest first:**
   1. The human decisions in §1.1.
   2. `docs/BRIEF.md`.
   3. Approved `features/`.
   4. The rest of this plan.
   5. Code.

   On a conflict, stop and report it. Do not resolve **[OPEN]** items yourself.
2. **BDD outside, TDD inside.** Behaviour is specified in Gherkin under `features/` (the outer loop); Rust unit tests are the inner loop. Never write implementation code without a failing scenario or test first.
3. **Frozen step vocabulary.** Steps come from the frozen vocabulary in `docs/steps.md`. Extending it is a [SPEC] task.
4. **Human review of [SPEC] features.** A [SPEC] task that writes features ends at human review. Implementation tasks that depend on those features start only after approval.
5. **`just` is the only entry point.** Recipe names are frozen (Appendix A). A missing flag is fixed in the justfile as its own task, not with an ad-hoc command.
6. **Definition of done.** A task is complete only when `just check` exits 0 on the Linux, macOS, and Windows CI runners.
7. **Commits:** one task per commit, as a conventional commit naming the task ID, e.g. `feat(views): P2.4 tagma bridge`.
8. **Verification:** verify every ⚠ before code depends on it (brief §6.4).
9. **Performance:** no optimization without a benchmark that fails its target first.
10. **Safety:**
    - TCP listeners bind loopback only: the macOS NFS loopback backend (§2.8), and upstream lspmux's default (D14).
    - Never run a language server or indexer as root or admin.
    - The privileged helper never executes project code.
11. **No silent skips.** Every scenario must run in at least one CI job, or be listed in `ci/expected-skips.txt`; this is enforced across jobs:
    - When env `CODETAGS_BDD_RAN` names a file, the BDD runner appends one line for each scenario it runs: `<feature file path relative to features/> :: <scenario name>`.
    - Every CI job that runs BDD sets that variable and uploads the file as an artifact.
    - A final `bdd-coverage` job `needs` every BDD job and runs whether they pass or fail. It downloads every record and runs `ci/bdd-coverage.py`, which parses `features/**/*.feature` for scenario names (a Scenario Outline counts once) and fails, listing every scenario that ran in no job.
    - The exception is an entry `scenario: <path> :: <name>  # reason` in `ci/expected-skips.txt`. An entry that names no scenario, or names one that ran, also fails the job.
    - Other CI checks a platform cannot yet perform are listed there as `<platform>: <check>`, e.g. `windows: assert-unprivileged` (V18). The file's header documents the format.
12. **Licence boundaries** (D9):
    - `just license-check` (cargo-deny) must pass.
    - Dependencies of the BSD-3 crates must be permissively licensed. No GPL, AGPL, LGPL or EUPL.
    - EUPL-1.2 is allowed only in `lspx`, and nothing outside `lspx` may depend on it.
    - GPL-3.0 is allowed only for `winfsp` and `winfsp-sys`, and only beneath `codetags-mount-winfsp` (D9). A Windows release that includes it ships a GPL-3.0 notice.
    - Exceptions need human approval.

## 1. Decisions and amendments (2026-09-30)

### 1.1 Human decisions

| ID | Decision | Effect on the brief |
|---|---|---|
| D1 | v1 targets **Linux, macOS, and Windows** on every track, and CI covers all three from P0. | Resolves §2 [OPEN] (platforms); lifts §4.3's "out of v1" |
| D2 | Live mounts are **not gated** on the evaluation. P5 starts once the view core exists. Conditions A, B and C still run, for measurement. | §4.5, §4.6, §5 |
| D3 | **Windows:** the unprivileged baseline is the static view tree plus the `codetags` CLI. WinFsp live mounts are an optional upgrade, used when an admin has installed WinFsp. | §2 |
| D4 | The repo is **public**, at `github.com/benjaminabbitt/codetags`. | none |
| D5 | **Persistent data lives in the project** (`.codetags/`, §2.3), not in a per-user cache. | §4.4, §4.5 |
| D6 | **Users and agents can tag files**, and those tags are queryable alongside derived facts. | Resolves §4.5 [OPEN] (read-only vs writes): tags are writable, content stays read-only |
| D7 | **Functionality is described in Gherkin** and run with cucumber. | §6 |
| D8 | **All views go through a plugin engine** built here. That includes `files/`, cards, `README`, `FACETS`, graphs, and result-set renderings in `q/`. **Architecture diagrams (Mermaid)** are generated from the code as virtual files, as a plugin. | §4.5 (views become plugins; adds Mermaid beside `modules.dot`) |
| D9 | **Licensing:** codetags is **BSD-3-Clause**, and `crates/lspx` stays **EUPL-1.2**, as lspmux requires. Hosting is not a concern, because this is developer tooling. No BSD crate or binary may link `lspx` code, since that would make it EUPL; `lspx` may depend on BSD crates. **GPL-3.0 is tolerated for the WinFsp backend** when it's the better choice, and it is (V28). The `winfsp` crate is allowed only under `codetags-mount-winfsp`, so Windows binaries built with that backend are distributed under GPL-3.0. Source and the Linux/macOS binaries stay BSD-3. | none (resolves O-1) |
| D10 | **Dogfood:** run codetags on this repo as soon as each piece can (milestones M1–M3, §6). | none |
| D11 | **View and edit by tags, BATFS-style** (§2.10). **DRAFT, for review.** Query directories show the real source files; reads and writes go to those files, and filesystem operations in query directories add and remove tags. | Reverses brief §1's non-goal "editing code through views" for source files reached through tags. Derived views (`.skel`, cards, diagrams) stay read-only. Supersedes the O-6 and O-7 defaults. |
| D12 | **Recursion guard** (§2.11). **DRAFT, for review.** Every mutating operation goes through one serialized queue, deletes are buffered, and a decaying counter detects recursive operations and holds them. | none |
| D13 | **Change notifier** (§2.12). **DRAFT, for review.** Watcher events become tagged items. Per-consumer filters, written as tagma postfix queries over file name, extension, metadata and tags, decide what each language server is sent. | brief §4.3 "Routing" |
| D14 | **Zero changes to lspmux, if at all possible** (2026-10-01). Use upstream lspmux unmodified, as an installed and pinned tool rather than a vendored fork, and build our pieces around it: shim, watcher and config. Its transport stays as upstream has it: loopback TCP by default, which is fine. If lspmux can't meet a requirement unmodified, the requirement becomes an upstream PR or an accepted gap. A local change is the last resort, only when we must; lspmux is mature enough that it probably won't come to that. P3 is being re-planned from an analysis (`docs/proxy-zero-change.md`). | Overrides brief §2 "Proxy base: fork or extend lspmux" and "local transport only: Unix socket", C1, and D9's lspx carve-out (no EUPL code in this repo). Brief §2's "never listen on a non-loopback TCP port" still holds, through upstream's default config. |
| D15 | **No escaping in names or paths** (2026-10-01). Paths use raw tagma syntax on every OS; values that need it are quoted the tagma way (`"..."`). The only character no filesystem allows in a name, `/`, is kept out of values by data design, not escaped. On Windows, the characters Win32 forbids go through the Cygwin/MSYS2/WSL private-use mapping, which the WinFsp backend and the Windows static tree decode and encode (⚠ verified in S4). | Supersedes §2.7's percent-encoded path profile, C6 and O-4. |
| D16 | **Agent sessions never own documents** (2026-10-01). For an agent session such as Claude Code, the shim drops `textDocument/didOpen`, `didChange` and `didClose`, so the shared server reads agent-visible files from disk, and the watcher keeps it current through `didChangeWatchedFiles`. Only editor sessions (VS Code) send buffer contents. This answers lspmux question (a), and sidesteps Claude Code's reported stale-`didChange` bug. | brief §4.2 "Document ownership": agent sessions should not open documents |
| D17 | **lspmux listens on a Unix socket in a 0700 directory on Linux and macOS**, and on loopback TCP on Windows (2026-10-01). This is a config choice; lspmux is unchanged (D14). It closes the "any local user can make the daemon run programs" hole (V36, O-20) everywhere except Windows. | refines D14 and C1 |
| D18 | **The Claude Code integration is project-scoped:** a plugin in this repo, enabled only here; the official `rust-analyzer-lsp` plugin is disabled for this project and nowhere else. **Stage 0 comes first:** record Claude Code's real LSP traffic for one session, before the shim's design depends on the reported client bugs. | M3 |
| D19 | **lspmux is pinned at git rev `18861f9`**, installed with `cargo install --locked --git https://codeberg.org/p2502/lspmux --rev 18861f9` (2026-10-01). crates.io 0.3.0 breaks pyright and typescript-language-server (V35). The pin is bumped deliberately. | answers lspmux question (e) |
| D20 | **`codetags lsp setup` is the only writer of `~/.config/lspmux/config.toml`** (2026-10-01). lspmux can't be pointed elsewhere. The command writes the D17 socket, a `pass_environment` allowlist and the instance timeout; it backs up any existing file, shows a diff, and never runs implicitly. `codetags doctor` reports drift. | answers lspmux question (d) |
| D21 | **The shim starts `lspmux server` on demand** when its socket is missing, the same way on every OS; nothing is installed as a service (2026-10-01). | §3.4 of `docs/proxy-zero-change.md` |
| D22 | **VS Code joins in stage 2,** alongside Claude Code (2026-10-01). This repo's `.vscode/settings.json` points `rust-analyzer.server.path` at the shim, and a scripted VS Code-style session is added to the tests. | M3 |

### 1.2 Consequences adopted by this plan (review these)

The planning agent derived these from D1–D22 and from research. Each is a default the human may overturn.

- **C1. Transport.** *Superseded by D14:* upstream lspmux's transport, loopback TCP by default. Cross-machine use goes through SSH TCP port forwarding (`ssh -L`), which works between any of the three OSes.
- **C2. Watcher.**
  - The unprivileged baseline everywhere is `notify` (inotify / FSEvents / ReadDirectoryChangesW).
  - Privileged accelerators run through the optional helper: fanotify on Linux, the USN change journal on Windows.
  - macOS needs no accelerator.
- **C3. Live-mount backends.**
  - Linux: FUSE (`fuser`, pure-Rust mount via `fusermount3`).
  - macOS: an in-process NFSv3 loopback server, mounted with the built-in `mount_nfs`. No kext, no install.
  - Windows: WinFsp (optional, D3).
  - macFUSE, FUSE-T, ProjFS and Dokany are not v1 backends (O-10; §2.6 has the reasons).
- **C4. DuckDB index as immutable generations** (§2.2). DuckDB allows either one read-write process or many read-only processes, never both at once (V7).
- **C5. User tags live in a committed text file, not in DuckDB.**
  - The file is line-oriented, in tagma's `<id> <tag>...` format, so tags diff, merge, and review in git (D5, D6).
  - The DuckDB file is also in the project but gitignored. It is binary, regenerated, and unmergeable.
  - The brief's non-goal "database as source of truth" still holds. DuckDB is derived from the source code alone, and user tags, the only primary data, live in the text file.
- **C6. Path profile.** *Superseded by D15:* raw tagma syntax in paths, quoting rather than escaping, and the private-use mapping on Windows (§2.7).
- **C7. Views and mounts stay outside every workspace** (brief §4.5). Only `.codetags/`, which holds data and never views, lives in the project.
- **C8. Plugin kinds** (§2.9).
  - D8 puts every view behind a `ViewPlugin`.
  - The core keeps only tree routing, `q/` evaluation, and tagging.
  - v1 has two kinds of plugin:
    - **built-in** (Rust, compiled in);
    - **declarative** (a manifest plus a SQL query plus a template), sandboxed, and safe to load from a committed `.codetags/plugins/`.
  - Plugins that execute code (WASM or external processes) are O-14.
  - Mermaid is the first declarative plugin. It proves the architecture.

### 1.3 Brief claims contradicted by verification

Details are in `docs/verification.md`. These change the work:

- **V1.** lspmux's default transport is loopback TCP.
- **V2.** `pass_environment` defaults to *all* variables.
- **V3.** lspmux shares one server across differently spelled paths, because it compares device and inode.
- **V5.** Some server-to-client requests are already handled; the rest get no response at all, so the server hangs.

---

## 2. Architecture

### 2.1 Processes

```
per user ────────────────────────────────────────────────────────────────────────────────
Claude Code ─stdio─> lspx shim ─┐
VS Code     ─stdio─> lspx shim ─┴─ local socket ─> lspx daemon ─> language servers
                                                    ▲ codetags-watch
codetags CLI ─── local socket ──> codetagsd
                                  ├─ codetags-watch ─> coalescer ─> provider runs ─> .codetags/index/gen-N
                                  ├─ tag store <── .codetags/tags   (committed text, C5)
                                  └─ view core: ViewFs over (generation N + tag overlay) + tagma index
                                       ├─ static tree     all OSes (Tier 0)
                                       ├─ FUSE            Linux
                                       ├─ NFSv3 loopback  macOS
                                       └─ WinFsp          Windows, optional (D3)
optional, admin-installed: codetags-privhelper (fanotify | USN journal) ─> event stream to both daemons
```

- **Two daemons.** `lspx` (proxy track) and `codetagsd` (index/views track) are separate processes. This keeps the tracks independent (brief §3), and a crash in a mount cannot kill LSP sessions.
- **One watcher each.** Each daemon runs its own watcher from the shared `codetags-watch` crate. Merging the two watchers is an optimization and needs a benchmark first.
- **The daemon is optional for batch work.** `codetags index` and `codetags views materialize` run standalone, and in CI, against the generation store.

### 2.2 Generation store (DuckDB)

- **Writing.** An indexing run writes a new file `.codetags/index/gen-<N>.duckdb` that no other process has open, then writes `gen-<N>.complete` last. A completed generation is immutable.
- **Reading.**
  - Readers open the highest completed generation read-only, and any number of processes may do so (V7).
  - A reader switches generations by opening the new file and swapping an `Arc`.
  - Old handles keep serving the old generation until they are released, so a file opened during generation N reads entirely from N.
- **Incremental runs** may `ATTACH` the previous generation read-only and copy unchanged rows. Full rebuild is the P1 default; do this only when a benchmark justifies it.
- **Retention.**
  - Keep the previous generation for the brief's edge-count regression diff.
  - GC deletes older generations. On Windows, deleting a file that is still open fails; retry at the next GC.
- **Per-generation metadata:** a `run` table recording provider, version, args, start and finish times, status, per-file edge counts, and the source tree identity (the git tree SHA when available, otherwise a content hash).

### 2.3 Project persistence (`.codetags/`, D5)

```text
<project>/.codetags/
  .gitignore     # "index/", "*.lock", "local/" — self-contained; the project's own .gitignore is untouched
  config.toml    # committed: languages, provider args, excludes, extra reserved keys
  tags           # committed: user/agent tags (C5), one line per item, sorted
  plugins/       # committed: project-specific declarative view plugins (§2.9); defaults are embedded in the binary
  index/         # ignored: gen-N.duckdb, gen-N.complete, provider logs
  tags.lock      # ignored: cross-process writer lock
```

- **What the watcher sees.** It excludes `.codetags/index/`, its own output. It does watch `tags` and `config.toml`, so a `git pull`, a checkout, or a hand edit reloads them.
- **How `tags` is written.**
  - Rewritten atomically (temp file, then rename, with a retry on Windows sharing violations) while holding `tags.lock`.
  - Lines are sorted, with the tags sorted within each line. Diffs stay minimal, and a merge conflicts only when both sides tagged the same item.
- **Line format** (*sketch*):

  ```text
  file:src/billing/charge.rs owner=billing risk=high
  "file:docs/My Notes.md" area=docs
  ```

  - Item ids use `<kind>:` prefixes: `file:` and, later, `sym:`.
  - An id containing whitespace or `"` is quoted the tagma way (D15). tagma's `add_line` keeps a quoted id's quotes and doesn't decode them (V21), so codetags decodes ids itself.
  - codetags parses the file itself: it splits with `tagma_core::token::split_unquoted_whitespace`, parses each tag with `Tag::parse`, and calls `add_item`.
- **[OPEN] O-2:** a local-only personal tag file (`.codetags/local/tags`, ignored) alongside the shared one. Default: the shared file only.

### 2.4 Items, facets, and user tags

| Item kind | Id (*sketch*) | Stable across generations | User-taggable in v1 |
|---|---|---|---|
| file | `file:<project-relative POSIX path>` | yes, until a rename | **yes** (D6) |
| symbol | `sym:<canonical dotted name>` | until a rename | no (O-12) |
| call site | `site:<n>` | no | no |

- **Derived facets** follow the brief's ingest sketch (`kind`, `lang`, `module`, `calls`, `caller`, `target`, `line`, `dispatch`, `prov:*`, …) and are rebuilt with every generation. The full list is generated into `docs/facets.md`.
- **Reserved names.** Users may not write:
  - any null-namespace key used by a derived facet;
  - the namespaces `prov`, `ct`, `fs` (change-notifier facts, §2.12), and `tagma.*`;
  - any extra key listed in `config.toml`.

  Writing one is rejected, and the error names the reserved set.
- **User tags** accept any other key or namespace, e.g. `owner=billing`, `risk=high`, `review:status=needs-audit`. Queries mix them freely with derived facets: `q/owner=billing/kind=function`.
- **Orphans** (from BATFS): a tagged item whose file no longer exists in the current generation is listed in `orphans.txt`, never silently dropped. `codetags tags mv <old> <new>` re-points its tags after a rename.
- **The tagma index for a (generation, tags revision) pair:**
  1. Derived items are built once per generation.
  2. On a tag write, the derived index is cloned (`Index: Clone`) and the user tags are applied. That is the default until **O-3** (tagma `remove_item`) is decided.
  3. P6.3 benchmarks write-to-visible latency against O-8.

### 2.5 View core

- **`ViewFs`** is a platform-neutral trait (*sketch*: `lookup`, `readdir`, `getattr`, `read`, `create_tag`, `generation`). Every frontend is a thin adapter over it: the static materializer, the CLI, FUSE, NFS, and WinFsp.
- **File sizes.** Contents are rendered from DuckDB plus the tag overlay, deterministically per (generation, tags revision). `getattr` needs the size before the read, so rendering happens at lookup and goes into an LRU cache.
- **One contract for all frontends.** `features/views/` is the single view contract. The same scenarios run against the in-memory walker, the materialized tree, and each live mount (P5.4).
- **Plugins produce all file contents** (§2.9). The view core owns only the tree shape and the lookup rules.

### 2.6 Platform matrix

| Capability | Linux | macOS | Windows |
|---|---|---|---|
| Static tree (Tier 0) | yes | yes | yes |
| `codetags` CLI (`q`, `show`, `tag`) | yes | yes | yes |
| Live mount | FUSE via `fusermount3` (needs the fuse3 package) | NFSv3 loopback + `mount_nfs`, no install | WinFsp if installed (admin once, then unprivileged) |
| Staleness control | `fuser` `Notifier` inval + short TTLs | no server push: `actimeo=0`, directory mtime bump per generation | WinFsp notify ⚠ |
| Watcher (unprivileged) | inotify | FSEvents | ReadDirectoryChangesW |
| Privileged accelerator | fanotify | none needed | USN change journal |
| Proxy transport | loopback TCP (upstream lspmux, D14) | loopback TCP | loopback TCP |
| CI | ubuntu | macos (arm64) | windows, with and without WinFsp |

Why the other candidates are not v1 backends:

- **macFUSE (kext):** needs Reduced Security mode and a reboot.
- **macFUSE (FSKit backend):** mounts only under `/Volumes`, and has no invalidation.
- **FUSE-T:** commercial use needs a paid licence.
- **ProjFS:** hydrates real files onto disk and trips EDR rules.
- **Dokany:** stale.
- **WebDAV:** deprecated.

### 2.7 Names and paths (D15)

- **Canonical symbol names** are dotted (brief §4.5), built from SCIP descriptors, and otherwise keep their **real characters**:
  - Descriptor separators and Rust `::` become `.`. An overload disambiguator `(+N)` becomes `+N`.
  - Generics, `$`, `#` and the like stay as they are, e.g. `billing.Charge<T>.apply`.
  - Package paths containing `/` (Go, TS) are rendered dotted (`github.com.acme.billing`). The exact SCIP symbol stays in DuckDB, so a canonical name only needs to be unique; it is never decoded.
  - **Invariant:** no canonical name contains `/` or a control character. Two symbols with the same canonical name are a reported collision, never silently merged.
  - **Defaults, pending review** (P1.3c, from dogfooding this repo; V96–V98):
    - **Impl blocks are containers.** rust-analyzer's symbol for an impl block itself (`impl#[Tree]`) has no canonical name, and is neither a `symbol` row nor a caller. Its methods keep `Type.method` and `Type.Trait.method`.
    - **Value-namespace suffixes.** Rust keeps types (modules, structs, enums, traits) and values (functions, consts, statics, fields) in separate namespaces. Only on a collision: a type keeps the plain name and each value gets its kind's suffix, `+fn`, `+field`, `+const` or `+static` (the function `checker()` beside the module `checker/` is `checker+fn`). With no type in the collision, a function keeps the plain name, so a getter's field is `export_path+field`. `+` and letters cannot be mistaken for `+N`. Two values of one kind that share a name are still a reported collision.
    - **Rust names start with the crate,** as Rust's full paths do: the SCIP package name with `-` mapped to `_` (`codetags_model.store.GenerationStore`; the crate root `crate/` is `codetags_model`), always, not only on a collision. Go, TypeScript and Python are unchanged: their descriptors already carry the import or module path.
    - **The standard library and dependencies are prefixed with their crate** the same way (`core.iter.traits.iterator.Iterator.map`, `alloc.boxed.Box<T>.new`). A symbol's module ancestors are canonical names too (`billing.charge`); the crate root is not one, so an item at the root has no module.
- **Values never contain `/`, by design.**
  - Modules and packages are dotted.
  - File locations are faceted as ancestor segments (`fs:dir=src`, `fs:dir=billing`, multi-valued like `module`) and as the file name (`fs:name=charge.rs`).
  - Path-shaped matching uses tagma's `~`, where `.` matches any single character, `/` included: `file~src.billing.charge.rs`.
  - Files are browsed as nested directories under `@files/`, never as a single path component.
- **Paths are raw tagma.** Each `q/` component is exactly one tagma postfix element, as typed: `'q/prov:source=scip/line>50'`. A value containing tagma syntax or spaces is quoted the tagma way: `'q/calls="billing.Charge<T>.apply"'`. Agents single-quote whole query paths for the shell (brief §4.5).
- **Linux and macOS** allow everything except `/` and NUL in a name, so nothing more is needed. Finder displays `:` as `/`, which is cosmetic.
- **Windows (⚠ verified in S4).** Win32 forbids `" * : < > ? |`. Cygwin, MSYS2 (Git Bash) and WSL map those characters to private-use code points (U+F000 plus the ASCII code) when calling Windows.
  - The WinFsp backend decodes those code points back to ASCII on input and encodes them on output.
  - The Windows Tier 0 writer stores them the same way.
  - Agents in Git Bash or WSL therefore type and see the real characters. Native PowerShell and Explorer users see private-use glyphs and use the `codetags q` CLI instead.
  - Device names (`CON`, `NUL`, …) and trailing `.` or space follow S4's findings.
- **Result groups** are directories whose names start with `@` (`@files/`, `@symbols/`, `@sites/`). `@` can never start a tagma token, so a result name can never be mistaken for a query element.
- **Case-folding collisions.** When the materialized tree lands on a case-insensitive filesystem (APFS/NTFS default), only the colliding names get a deterministic suffix, `~<6 hex of id hash>`.

### 2.8 Security

- **Proxy transport (D14):** upstream lspmux, loopback TCP by default. Other local users can reach a loopback port, and language servers execute project code (brief §2), so on shared machines this is accepted exposure (O-20).
- **The macOS NFS loopback** also listens on TCP.
  - It binds 127.0.0.1 on an ephemeral port, with an unguessable export path.
  - The only operations allowed are reads and tag creation.
  - **Residual risk:** another local user who learns the port and token can read the views, because NFSv3 AUTH_SYS is spoofable. **[OPEN] O-5.** Default: accept, with these mitigations documented.
- **Privileged helper.**
  - It sends only an event stream.
  - Events are filtered to roots the requesting user can read, and the helper checks this itself.
  - It never executes project code and is never required.
- **Mutations through a mount** (D11) all go through the operation queue and recursion guard (§2.11). Views edit file contents and tags. They never create, delete, or rename source files (O-16).
- **Plugins never run repo-supplied code** unless the user has trusted it on their own machine (O-14).
  - Declarative plugins run on a read-only DuckDB connection with external access disabled and configuration locked (V22).
  - Their templates have no filesystem or network access.
  - A plugin from a freshly cloned repo is therefore data, not code.

### 2.9 View plugins (D8)

**The `ViewPlugin` contract** (*sketch*):

```rust
// sketch
trait ViewPlugin: Send + Sync {
    fn manifest(&self) -> &Manifest;                 // name, version, mounts (below), output extension
    fn list(&self, ctx: &Ctx, scope: &Scope) -> Result<Vec<Entry>>;   // finite
    fn render(&self, ctx: &Ctx, scope: &Scope, entry: &str) -> Result<Vec<u8>>;
}
// Ctx:   generation handle (read-only SQL), tag overlay, tagma index, config
// Scope: Root | Query(ids)       — a plugin may mount at the root, inside every q/ result set, or both
```

**Rules:**

- Output must be deterministic per (generation, tags revision, plugin version); `getattr` needs the size before the read, and the cache relies on it.
- Every output starts with the brief's VIEW header, in the output format's comment syntax (`%%` for Mermaid).
- A plugin failure renders an error file (`<name>.error`). It never takes the mount down, and it is counted in `codetags doctor`.

**Where plugins mount:**

- **Root:** `<views root>/<repo>/<plugin>/…`, e.g. `arch/modules.mmd`.
- **Result-set scoped:** `q/<query>/@views/<plugin>.<ext>`. This renders the plugin over just that query's items, e.g. `q/owner=billing/@views/callgraph.mmd` shows the architecture of the billing-owned code only.

**Declarative plugins** live in `.codetags/plugins/<name>/`:

- `plugin.toml`: the manifest.
- `*.sql`: queries over a documented, versioned view schema (`docs/plugin-schema.md`), not the raw tables, so internal schema changes don't break plugins.
- `*.j2`: `minijinja` templates.

**Built-in plugins:**

- `skel` (files view)
- `card` (symbol and site cards)
- `dot` (`modules.dot`)
- `readme`
- `facets`

**The first declarative plugin is `mermaid`:**

- `arch/modules.mmd`: a module-dependency flowchart, with edges weighted by call counts.
- `arch/<module>.mmd`: a drill-down, one per module.
- `@views/callgraph.mmd`: the call graph of a result set.
- `.md` twins wrapping each diagram in a ```` ```mermaid ```` fence, so GitHub and VS Code previews render it.

Mermaid has size limits (⚠ V23; believed to be 500 edges and 50,000 characters by default). The plugin aggregates to stay under them (O-15) and writes a comment saying what it elided, rather than emitting a diagram that won't render.

### 2.10 Editing by tags (D11, DRAFT for review)

Under a query directory `q/<Q>`, result groups become:

| Group | Contents | Writable |
|---|---|---|
| `@files/<path>` | the **real source file** at project path `<path>` (passthrough) | content: yes; tags: via the operations below |
| `@skel/<path>.skel` | the derived skeleton of that file | no |
| `@symbols/`, `@sites/`, `@views/` | as in §2.9 and Appendix B | no |

**Operation mapping** in `q/<Q>`. Here *A* is the set of definite atoms in *Q*: bare keys or `key=value`, optionally namespaced. If *Q* contains `or`, `not`, a quantifier, or a relational, `~` or `!=` element, its directory accepts no tag operations (EINVAL).

| Operation | Effect |
|---|---|
| read `@files/<path>` | reads the source file |
| write or truncate `@files/<path>` | writes the source file in place, through the queue |
| an editor's atomic save (write a temp file, then rename it over the target) | the temp file lives in a per-mount staging area, never in the project; the rename replaces the source file atomically (a temp file plus rename in the source file's own directory) |
| create, `touch`, `cp` or `ln` to `@files/<path>`, where `<path>` exists in the project | adds *A* to that file |
| create `@files/<path>`, where `<path>` does not exist | EPERM (O-16) |
| `mv q/<Q1>/@files/<p> q/<Q2>/@files/<p>` | retags: removes *A1* and adds *A2*, as one queued operation |
| `rm q/<Q>/@files/<path>` | **untags**: removes *A* from the file, never the file itself; buffered (§2.11) |
| `mkdir q/<Q>/<elem>` | succeeds if `<elem>` is a valid next element; every valid query directory already exists |
| `rmdir` of a query directory | EPERM. Removing a tag from every item is `codetags untag --all`, which is explicit and journaled. |

**Journal.** Every tag change is recorded with its time, actor and inverse operation in `.codetags/local/journal` (ignored). `codetags ops undo` reverses changes, and git is the history for the committed `tags` file. This replaces BATFS's Trash tag; nothing needs a reserved tag.

### 2.11 Operation queue and recursion guard (D12, DRAFT for review)

- **One queue.** Every mutating operation from every frontend (FUSE, NFS, WinFsp, the CLI) becomes an `Op` on one serialized queue per project, applied by one worker. That gives ordering, a single writer for `.codetags/tags`, the journal, and one place for policy. Reads never queue.
- **Buffered deletes.** Destructive operations (untag via `rm`, the removal half of a retag, `untag --all`) wait for a grace window (default 2 s) before they commit. Creates and content writes commit at once.
- **Decaying counter.** Each actor has a counter that decays on every destructive operation: *c* ← *c*·2^(−Δ*t*/*h*) + 1, with half-life *h* (default 1 s).
  - FUSE and WinFsp identify the actor by the requesting PID.
  - NFSv3 carries no PID, so on macOS the actor is the whole mount.
  - For the CLI, the actor is the invocation.
  - The breadth of distinct directories touched is tracked as well, to tell `rm -r`'s sweep apart from a few hand deletions.
- **Tripping.** When *c* crosses the threshold *T* (default 20), the actor trips:
  - its buffered deletes do not commit;
  - its further destructive operations fail with EPERM until it has been quiet (*c* < *T*/4);
  - the held batch appears in `codetags ops pending` and in a read-only `@pending` file at the view root.

  `codetags ops commit` and `codetags ops discard` resolve it.
- **Content writes** are counted and journaled but never held (O-18).

### 2.12 Change notifier (D13, DRAFT for review)

- **Events as items.** Each coalesced watcher batch becomes a small tagma index with one item per changed path. Each item is tagged with:
  - `fs:event=created|changed|deleted`, `fs:path=<project-relative path>`, `fs:name=<file name>`, and `fs:ext=<extension>`;
  - `fs:dir=<ancestor>` for **every** ancestor directory (multi-valued, like `module`);
  - `fs:kind=file|dir|symlink`, `fs:size`, `fs:mtime`, `fs:ignored` (gitignored), and `fs:generated` (under the build-output excludes);
  - plus the file's user tags and its derived facets (e.g. `lang=rust`) from the current generation.
- **Filters.** Every consumer has a filter, written as a tagma postfix query: each language-server instance, the reindex loop, and plugins. A language server is sent the events that match its filter **and** its registered globs (brief §4.3). A filter can only narrow what a server registered for, never widen it; servers that register nothing still get nothing.
- **Why not globs everywhere.** tagma v1's `~` is anchored and has only `.` as a wildcard, so server registrations stay globs (`globset`). User filters use `fs:ext`, `fs:name` and `fs:dir` instead.
- **Configuration** (*sketch*). Built-in defaults exist per server; `.codetags/config.toml` overrides them:

  ```toml
  [notify.rust-analyzer]
  filter = "fs:ext=rs/fs:name=Cargo.toml/or/fs:name=Cargo.lock/or/fs:dir=target/not/and"
  ```
- **Readiness gate.** Only servers that were actually sent events are held by the readiness gate (brief §4.2).

---

## 3. Repo layout

```
codetags/
  Cargo.toml  rust-toolchain.toml  justfile  PLAN.md  providers.toml
  docs/        BRIEF.md  verification.md  steps.md  path-profile.md  facets.md
  features/    names/ path-profile/ tags/ views/ index/ cli/ proxy/ watch/ mount/
  crates/
    codetags-model/         DuckDB schema, migrations, generation store
    codetags-names/         canonical names, path profile, collision suffixes (pure)
    codetags-ingest/        SCIP reader, provider runners, resolution report
    codetags-tags/          tags file: parse, write, lock, reserved keys, orphans
    codetags-views/         ViewFs, lookup rules, tagma bridge
    codetags-plugins/       ViewPlugin trait, registry, built-ins, declarative runtime + sandbox
    codetags-materialize/   Tier 0 frontend
    codetags-mount-fuse/    Linux
    codetags-mount-nfs/     macOS (the server compiles and unit-tests everywhere)
    codetags-mount-winfsp/  Windows
    codetags-watch/         notify watcher, coalescer, privhelper client
    codetags-ipc/           local sockets, peer checks
    codetags-privhelper/    optional privileged helper (bin)
    codetags/               CLI and codetagsd (bin)
    codetags-bdd/           cucumber runner and step definitions (test-only)
    codetags-lsp/           LSP session recorder and analyzer (M3 stage 0), later the shim (P3.4); no DuckDB (bin)
    lspx/                   lspmux fork, EUPL-1.2 with its own LICENSE (bin), P3.0. Nothing else depends on it (D9)
  plugins/mermaid/          first declarative plugin (plugin.toml, *.sql, *.j2), shipped as a default
  tools/claude-plugins/     local Claude Code marketplace, enabled in .claude/settings.json (D18)
  tools/gocallgraph/        Go program: x/tools/go/callgraph (VTA/CHA) → JSONL
  tests/fixtures/<lang>/    small repos with known edges
  tests/baselines/          committed edge counts per fixture
  eval/                     questions, runner, results/*.jsonl
  ci/expected-skips.txt
  .github/workflows/
```

- **The dogfood `.codetags/`.** codetags keeps one in its own repo. `just check` validates `.codetags/tags` there.
- **Gherkin tags for scenarios:**
  - Platform: `@linux`, `@macos`, `@windows`.
  - Capability: `@mount`, `@winfsp`, `@providers`, `@privileged`.
  - `@slow`.

  A job runs exactly the tags its capabilities allow, and rule 11 applies.

## 4. Toolchain and pinned dependencies

| Dependency | Pin | Notes |
|---|---|---|
| Rust | `rust-toolchain.toml` 1.96.x, edition 2024 | 1.96.1 on the dev host |
| tagma-core | git `benjaminabbitt/tagma` at the full SHA of `b1ae808`, the first BSD-3-Clause commit | Local override: `just dev-tagma PATH` adds a marked `[patch]` block to the committed `.cargo/config.toml`, and `just dev-tagma` removes it. `just override-check`, part of `check`, fails while it is present. |
| duckdb | `~1.10506` (DuckDB 1.5.6), default features (V29) | **Development and CI** link the prebuilt dynamic library (R1, V29):<br>• The committed `.cargo/config.toml` sets `DUCKDB_DOWNLOAD_LIB=1`, so `just`, plain cargo and rust-analyzer all use it.<br>• libduckdb-sys downloads the release library into `target/duckdb-download/`, where `just duckdb-verify` checks the archive against `duckdb.sha256` (R8), and copies it into `target/<profile>/deps`. Cargo puts that directory on the runtime library path for tests and `cargo run`.<br>• A binary that links DuckDB, run outside cargo, does not find the library: there is no rpath.<br>**Release builds** enable `codetags-model`'s `bundled-duckdb` feature (`duckdb/bundled`). It is a static build, so there is no runtime DLL, but it compiles DuckDB's C++ for about 12 minutes. On Windows it needs a short `CARGO_TARGET_DIR` and `+crt-static` (V8). |
| cucumber | 0.23 | Latest as of 2026-09-30; tagma pins 0.21.1 |
| fuser | 0.18 | V11 |
| `nfsserve` or `nfs3_server` | chosen in spike S2 | V13 |
| winfsp (winfsp-rs) | 0.13 (GPL-3.0, D9), features `full` | Chosen over `winfsp_wrs` (MIT), for reasons in V27 and V28:<br>• It builds without WinFsp installed, using its bundled import lib and headers. bindgen needs libclang on Windows build hosts ⚠.<br>• At runtime, `winfsp_link_delayload()` plus `winfsp_init()` give a graceful fallback.<br>• The `notify` feature gives real cache invalidation.<br>• It passes WinFsp's own test suite (through ntptfs).<br>• Risk: one main maintainer. Mitigation: the backend is confined to one crate, so swapping in `winfsp_wrs` is local work. |
| notify | 8.x; 9.0 once it is stable | V17 |
| interprocess | 2.4 | V15 |
| minijinja | 2.x stable (3.0 is in alpha) | Template engine for declarative plugins; no loader, so no filesystem access |
| SCIP bindings | pinned | Compare edge counts on every upgrade (brief §4.4) |
| Providers | rust-analyzer ≥ 2025-11-28, scip-go, scip-typescript, scip-python, Jelly, Go toolchain | Pinned in `providers.toml` and installed by `just setup-providers`. ⚠ Each must run on each OS (V20). |

## 5. CI (GitHub Actions)

- **`check`**, a matrix over Linux, macOS (arm64) and Windows with pinned runner labels (V19). Each runs `just setup && just check`. The Windows leg has no WinFsp installed and must show the fallback message and a working Tier 0.
- **`mount-linux`** (ubuntu-24.04).
  1. Delete the non-setuid `/usr/local/bin/fusermount3` shadow, only if it exists and is not setuid (V12). sudo is allowed for this setup.
  2. Make sure `fuse3` is installed.
  3. Assert the process is unprivileged, and run `just doctor`.
  4. Run `just test-mount` as the runner user, with no sudo.
- **`mount-macos`:** `just test-mount` with no sudo.
- **`mount-windows-winfsp`:** install WinFsp with `choco install winfsp`, then run `just test-mount` from a de-elevated process (V18).
- **Windows fallback.** winfsp-rs builds without WinFsp installed, so the Windows `check` leg builds the full binary, WinFsp backend included. That leg runs on a runner without WinFsp, which proves a release binary starts, prints the fallback message, and serves Tier 0.
- **`providers`**, a matrix over the three OSes: `just setup-providers && just test-providers && just baseline-check`.
- **`privileged-linux`:** the helper runs under sudo while the daemon runs as the runner user.
- **Every job that runs product code** has a step asserting the process is unprivileged: euid ≠ 0, or the Windows token is not elevated.
- **`bdd-coverage`:** needs every job that runs BDD, and checks that every scenario ran somewhere (rule 11).
- **Triggers:** pushes to `main`, `spike/**` and `agent/**` (so agents can iterate on CI from their own branches), pull requests, and `workflow_dispatch`.
- **Caching:** `Swatinem/rust-cache`. DuckDB is not compiled: development and CI link the prebuilt library (§4, R1). Each job runs `cargo clean -p libduckdb-sys` after restoring the cache, so the library is downloaded again (V29).

## 6. Phase graph

```
P0 ──┬─> P0b spikes (S1–S4) ─────────────────────────────────┐
     ├─> P1 index ──> P2 view core, static tree, CLI ──┬─> P5 live mounts ─┬─> P6.4 FS tagging
     │                                                 ├─> P6 tagging ─────┘
     │                                                 └─> P7 plugins + mermaid (P7.4 @views needs P5 to be seen live)
     └─> P3 proxy ──> P4 watcher, readiness, reindex loop (joins the index track)
```

- The index/views track (P1, P2, P5, P6, P7) and the proxy track (P3, P4) run in parallel.
- **Dogfood milestones** (D10):
  - **M1, static.** Once P1.4 (the Rust provider) and P2.5–P2.6 (materializer and CLI) are done, `just dogfood` indexes this repo and materializes its views. CI runs it, and development uses the views. This repo's own `.codetags/` is committed then.
  - **M2, live.** After P5.1, this repo's views are mounted while developing on Linux.
  - **M3, proxy.** After P3–P4, this repo's rust-analyzer runs through `lspx`.
  - **P1 order:** the Rust provider is built first, to reach M1 soonest.
- Phase numbers are labels, not order:
  - P6 needs P2 but not P5, except P6.4.
  - P7 needs P2.3's trait, and nothing else.

---

## 7. P0 — bootstrap

| ID | Tag | Task |
|---|---|---|
| P0.1 | MECH | `git init`. Create the public GitHub repo; the human confirms before the first push. Root `LICENSE` is BSD-3-Clause; `license = "BSD-3-Clause"` in every crate's manifest except `lspx` (D9). |
| P0.1b | MECH | `deny.toml` and `just license-check` (rule 12), wired into `just check`. |
| P0.2 | MECH | Workspace skeleton: the §3 crates as stubs, and `rust-toolchain.toml`. |
| P0.3 | MECH | justfile (Appendix A). |
| P0.4 | CORE | cucumber harness (`codetags-bdd`), `docs/steps.md`, and the first feature, `features/cli/version.feature`. |
| P0.5 | CORE | DuckDB smoke scenario on all three OSes: link the prebuilt library (R1); write a generation; read it read-only from a second process. |
| P0.6 | MECH | tagma-core git dependency, plus a smoke scenario: ingest two items and run a postfix query. |
| P0.7 | CORE | CI `check` job (§5), including the unprivileged assertion. |

**Done when** `just check` exits 0 on all three runners.

## 8. P0b — platform spikes

Because of D2, mount risk is retired early. Each spike:

- serves a one-file hello filesystem;
- has a scenario: "mount, list, read, unmount, as an unprivileged user";
- lands its code in the matching `codetags-mount-*` crate, as the seed of P5.

| ID | Tag | Spike |
|---|---|---|
| S1 | CORE | Linux FUSE with `fuser`. `codetags doctor` detects a non-setuid `fusermount3` that shadows the real one on PATH. |
| S2 | CORE | macOS NFSv3 loopback. Choose between `nfsserve` and `nfs3_server` and record why. Mount with an unprivileged `mount_nfs` onto a user-owned directory. Measure staleness under `actimeo=0`, including negative-name caching. |
| S3 | CORE | Windows WinFsp mount from a de-elevated process. A companion scenario without WinFsp checks the fallback message. |
| S4 | SPEC | Windows name probe through WinFsp: which tagma characters, device names, and trailing dots or spaces reach the filesystem. The results feed `docs/path-profile.md`. |

**Done when** each spike's scenario is green in its CI job and its findings are recorded.

## 9. P1 — index into DuckDB generations (brief §4.4, §2.2 here)

| ID | Tag | Task |
|---|---|---|
| P1.1 | CORE | `codetags-model`: the brief's schema plus `run` and `file` tables, and the generation store (write, complete, open-latest, GC). Scenario: a reader holds generation N while a writer completes N+1. |
| P1.2 | CORE | `codetags-names` (D15): the SCIP symbol parser and canonical names (dotted, real characters, no `/`, collisions reported), the Windows private-use mapping, item ids for the tags file (tagma quoting when needed), and collision suffixes. Property tests: names never contain `/` or control characters; the private-use mapping round-trips; suffixes leave no case-fold collisions. |
| P1.3 | CORE | SCIP ingest: attribute each reference to the innermost enclosing range; capture functions used as values; record control context and literal names (brief rules). **Split: P1.3b** takes control context and literal names, which SCIP does not carry. |
| P1.4 | CORE | Rust provider (`rust-analyzer scip`): filter `local N`, expand trait and `dyn` calls. |
| P1.5 | CORE | Go provider: `scip-go` plus `tools/gocallgraph`, joined by call site. |
| P1.6 | CORE | TypeScript provider: `scip-typescript` (raise the file-size limit) ∪ Jelly. Failures are loud. |
| P1.7 | CORE | Python provider: `scip-python`, plus name-match candidates with counts. |
| P1.8 | CORE | Resolution report per language and per module. Edge-count regression against the previous generation and `tests/baselines/`. Any provider failure exits non-zero. |

- Each provider task comes with a fixture and `features/index/<lang>.feature`, which lists the expected edges.
- **Done when** the brief's P1 condition holds on all three OSes. If a provider cannot run on an OS, that exception is recorded in `docs/verification.md` and needs human approval.

## 10. P2 — view core, static tree, CLI, evaluation

| ID | Tag | Task |
|---|---|---|
| P2.1 | SPEC | `features/views/*.feature`: layout, finite traversal, lookup rules, headers, and result groups (Appendix B). Human review. |
| P2.2 | CORE | The `ViewFs` trait, plus an in-memory walker harness that runs the view features. |
| P2.3 | CORE | The `ViewPlugin` trait (§2.9), and every renderer written as a built-in plugin through it: `README.md`, `FACETS.md` (facet names and counts, since facets are never listed), `files/**.skel`, `modules.dot`, `unresolved/`, `orphans.txt`, symbol cards. |
| P2.4 | CORE | tagma bridge: per-generation ingest (the brief's sketch plus `_config` meta tags) and `q/` evaluation through the path profile. Benchmark build and query on the largest fixture. |
| P2.5 | CORE | Materializer (Tier 0) into the per-user views directory, read-only. Build a sibling directory, then rename with retry; the per-file headers make any mix of generations detectable. |
| P2.6 | CORE | CLI: `codetags q <query>`, `codetags show <item>`, `codetags where`. |
| P2.7 | SPEC | Evaluation harness for conditions A and B; C is wired up in P5.5. The eval repo is still the brief's [OPEN]. |

**Done when:**

- the view features pass against the in-memory walker and the materialized tree on all three OSes;
- conditions A and B run end to end, with results committed.

This no longer gates anything (D2).

## 11. P3 — lspx proxy (brief §4.2, plus C1 and V1–V5)

| ID | Tag | Task |
|---|---|---|
| P3.0 | SPEC | **Re-planned by D14.** Analyse a zero-change integration of upstream lspmux, classifying each brief §4.1–§4.3 requirement as done upstream, achievable outside lspmux, or needing an upstream PR, in `docs/proxy-zero-change.md`. P3.1–P3.8 are rewritten from it after human review. |
| P3.1 | MECH | Transport: no change (D14). Verify that upstream's default listen address is loopback, and record it. |
| P3.2 | CORE | Routing key per the brief. Replace `pass_environment = ["*"]` with a curated allowlist (V2). Replace device+inode equality with spelled-path identity, warning on aliases (V3). |
| P3.3 | CORE | Fixed-capability handshake. A multi-root handshake returns an LSP error and never panics (V4). |
| P3.4 | CORE | Server-to-client requests per the brief's table. Every request gets a response (V5). |
| P3.5 | CORE | Document ownership. |
| P3.6 | CORE | Configuration merge. |
| P3.7 | CORE | Crash recovery. |
| P3.8 | CORE | Scripted-session features, VS Code-style and Claude Code-style, sharing one rust-analyzer and one gopls. |

**Done when** the brief's P3 condition holds on all three OSes.

## 12. P4 — watcher, coalescer, readiness gate, reindex loop

| ID | Tag | Task |
|---|---|---|
| P4.1 | CORE | `codetags-watch` on notify: scan new directories to close the creation race; on Rescan, diff against the last snapshot; apply the brief's excludes plus `.codetags/index/`; give clear limit errors (the inotify sysctl, the Windows buffer). |
| P4.2 | CORE | Coalescer: the brief's merge rule, a quiet window with a maximum-wait cap, and a pause while `.git/index.lock` exists. |
| P4.3 | CORE | Route each batch to servers by registered globs ∧ the server's notifier filter (§2.12); readiness gate. |
| P4.4 | CORE | Optional privhelper: fanotify on Linux, USN journal on Windows. It filters paths per user; the daemon logs which mode is active and falls back when the helper is absent. |
| P4.6 | SPEC | `features/watch/notify.feature`: the `fs:` facts, filter semantics, narrowing-only, and the per-server defaults. Human review. |
| P4.7 | CORE | Change notifier (§2.12): per-batch event index, filters from config, built-in defaults for rust-analyzer, gopls, pyright and the TS server. |
| P4.5 | CORE | Reindex loop in `codetagsd`: batch → affected providers → new generation → views swap. A change to the tags file reloads only the overlay. |

**Done when:**

- the brief's P4 condition holds on all three OSes;
- a scenario measures edit-to-view freshness and records the number (the target is O-8).

## 13. P5 — live mounts (not gated, D2)

| ID | Tag | Task |
|---|---|---|
| P5.1 | CORE | Linux FUSE over `ViewFs`: TTLs near zero, plus `Notifier` invalidation on a generation swap or a tag write. |
| P5.2 | CORE | macOS NFSv3 backend on `nfs3_server` (S2, V42): a table mapping (generation, node) to handles of at most 56 bytes; ESTALE after GC; `actimeo=0`. **Bump the directory mtime on every change.** macOS caches failed lookups until a directory's mtime changes, even under `actimeo=0` (V44); `nonegnamecache` is the alternative. |
| P5.3 | CORE | Windows WinFsp backend on `winfsp` (winfsp-rs):<br>• Delay-load the DLL; if `winfsp_init()` fails, fall back to Tier 0 (D3).<br>• Set case-sensitive search and case-preserved names.<br>• Use short info timeouts, plus `Notifier` invalidation on a generation swap or a tag write.<br>• Show the WinFsp attribution notice and repo link in `codetags --version`, `codetags doctor` and the README (V14).<br>• Windows release archives carry the GPL-3.0 licence text and a NOTICE (D9). |
| P5.4 | CORE | Run every `features/views` scenario against each live backend, parameterized by backend. Content must equal the in-memory rendering byte for byte. |
| P5.5 | CORE | Evaluation condition C. |

**Done when:**

- the brief's P5 condition holds (C runs, `rg` over the mount terminates, rebuild fits the budget) on Linux and macOS, and on Windows in the WinFsp job;
- the Windows job without WinFsp falls back cleanly.

## 14. P6 — user and agent tagging (D6)

| ID | Tag | Task |
|---|---|---|
| P6.1 | SPEC | `features/tags/*.feature`: tag, untag and list via the CLI; reserved names; orphans; re-pointing after a rename; the tags file format and its merge behaviour; plus the §2.10 operation mapping and the §2.11 guard. Human review. |
| P6.2 | CORE | `codetags-tags`: parse, write and lock the tags file. Commands: `codetags tag`, `untag`, `tags`, `tags check`, `tags mv`. |
| P6.3 | CORE | Apply the overlay to the tagma index and the renderers, so user tags appear in `.skel` headers and symbol cards. Benchmark write-to-visible latency (O-8). |
| P6.4 | CORE | Operation queue, journal, `codetags ops pending/commit/discard/undo`, and the recursion guard (§2.11). Scenarios: `rm -r` over a 100-file query directory untags nothing until commit; a single `rm` commits after the grace window. |
| P6.5 | CORE | BATFS-style tag operations through live mounts (§2.10), via the queue. Needs P5. |
| P6.6 | CORE | `@files` passthrough reads and writes, including editor atomic saves through the staging area. Needs P5. |

**Done when:**

- the tag features pass through the CLI on all three OSes;
- the §2.10 and §2.11 features pass on each live backend.

## 15. P7 — view plugins and Mermaid diagrams (D8)

| ID | Tag | Task |
|---|---|---|
| P7.1 | SPEC | `features/plugins/*.feature`: the plugin contract (determinism, header, error files, where plugins mount, sandbox refusals), and Mermaid output for the fixtures: expected `.mmd` text, and aggregation once past the limits. Human review. |
| P7.2 | CORE | Plugin registry and manifest loading: built-ins first, then `.codetags/plugins/`. Name clashes are an error naming both sources. |
| P7.3 | CORE | Declarative runtime: the versioned plugin schema (SQL views over a generation plus the tag overlay), a sandboxed DuckDB connection (V22), and minijinja with no loader. Scenario: a plugin that tries `read_csv`, `ATTACH`, or a template include is refused. |
| P7.4 | CORE | The `plugins/mermaid` plugin: `arch/modules.mmd`, the per-module drill-downs, `@views/callgraph.mmd`, the `.md` twins, and aggregation under the limits (O-15). The output must parse with the Mermaid CLI in CI (V23). |
| P7.5 | SPEC | Proposal for plugins that execute code, WASM or external process (O-14). It ends at human review; nothing is built. |

**Done when:**

- the plugin features pass on all three OSes through the in-memory walker and the materialized tree, and on each live backend;
- `just check` parses every Mermaid fixture output.

---

## 16. OPEN items

**Carried from the brief:**

- the full multi-root approach (§4.2);
- a RouteKey change mid-session (§4.2);
- framework rules (§4.4);
- the eval repo (§4.6);
- tagma `remove_item` (§4.5), now O-3.

**Resolved by §1:** v1 platforms (D1); read-only views vs writes (D6).

| ID | Question | Default |
|---|---|---|
| O-1 | **Resolved by D9.** tagma is now BSD-3-Clause, as of tagma `b1ae808` (2026-10-01). WinFsp's FLOSS exception does cover BSD-3 (V14), on two conditions:<br>• we show its attribution notice;<br>• we never link or distribute it with proprietary software.<br>So a proprietary downstream fork would lose the exception for the WinFsp backend. That is their concern, not ours. | resolved |
| O-2 | A local-only personal tag file. | shared file only |
| O-3 | Add `remove_item` to tagma (now on the tagging hot path). | clone-and-apply per write; revisit if the P6.3 benchmark fails |
| O-4 | Upstream the path profile into tagma's SPEC. | **obsolete (D15):** there is no path profile |
| O-5 | macOS NFS loopback is reachable by other local users. S2 (V43–V44) found the random export path is **not** secret: `mount` and `nfsstat -m` show it and the port to any local user, and both server crates hand out the export list. So the only real mitigations are read-only exports and single-user machines. | accept on single-user machines; document the exposure |
| O-6 | Untagging by unlinking an entry in a query directory. | **superseded by D11:** `rm` untags, guarded by §2.11 |
| O-7 | Should file items in `q/` show the `.skel` view or the raw source? | **superseded by D11:** raw source under `@files/`, skeletons under `@skel/` |
| O-8 | Targets: tag write → visible; source edit → view fresh; a cap on result listings. | 250 ms p95 on 1M items (proposed); measure freshness first; no cap |
| O-9 | Import lspmux by subtree, or keep a separate fork repo. | subtree |
| O-10 | macFUSE or FUSE-T as optional macOS backends when already installed. | not in v1 |
| O-11 | Cross-machine proxy with a Windows endpoint. | resolved by D14: SSH TCP port forwarding |
| O-12 | User tags on symbols, not just files. | files only in v1; the id scheme already allows symbols |
| O-13 | Incremental edges from the proxy's warm servers (`source=lsp@ver`) between SCIP runs. | not in v1 |
| O-14 | Plugins that execute code: WASM (sandboxed, cross-platform, any language) or external processes (simplest, but a repo-supplied command is code run on clone, so it needs a trust step like `direnv allow`). | not in v1; built-in and declarative only |
| O-15 | Mermaid aggregation policy (module depth, edge cap, how elisions are shown). | collapse to module depth 2 and keep the heaviest edges under the limit; list elided counts in a comment |
| O-16 | Creating, deleting or renaming *source files* through views. | refused: views edit contents and tags, never the shape of the tree |
| O-17 | Recursion-guard parameters: half-life, threshold, grace window, quiet level; and what a tripped actor sees. | 1 s, 20, 2 s, *T*/4; EPERM plus `@pending` and `codetags ops pending` |
| O-18 | Whether mass content writes (e.g. `sed -i` over a query directory) are held like deletes. | counted and journaled, never held |
| O-19 | D11–D13 are drafts: confirm the operation mapping, the guard, and the notifier before their [SPEC] features are written. | none; the human reviews |
| O-20 | Loopback TCP can be reached by other local users on shared machines. | **resolved by D17** on Linux and macOS; on Windows, accepted (D14) |

## 17. Risks

| ID | Risk | Mitigation |
|---|---|---|
| R1 | The bundled DuckDB build is slow, especially on Windows CI. | Development and CI never compile DuckDB. They link the prebuilt library through `DUCKDB_DOWNLOAD_LIB` (§4, V29), so a cold `just check` takes about 1.5 minutes on the dev host, with no C++ compile. Only release builds enable `bundled-duckdb`. CI runs `cargo clean -p libduckdb-sys` after restoring its cache, because rust-cache prunes the downloaded library (V29). Residual risks: the download needs network on a cold `target/`, and libduckdb-sys itself checks no checksum; `just check` verifies the archive afterwards (R8). |
| R2 | A provider does not run on some OS (e.g. scip-go, scip-python or Jelly on Windows). | V20; a per-OS exception approved by the human. |
| R3 | tagma is in-memory with String keys, so it may not scale in memory or rebuild time. | O-3. Alternative: compile postfix queries to SQL over a DuckDB tag table, validated by tagma's own conformance features. |
| R4 | The macOS NFS client caches despite `actimeo=0`, e.g. negative-name caching. | Spike S2 measures it. |
| R5 | Proving "unprivileged" on Windows runners, which run as admin. | V18. |
| R6 | Runner images drift; the `fusermount3` shadow is an example. | `codetags doctor` plus pinned runner labels. |
| R7 | Deep query paths hit Windows' 260-character MAX_PATH and case-insensitive name handling on Windows and macOS. | Collision suffixes; tests at the limits; the `codetags q` CLI as the fallback |
| R8 | libduckdb-sys downloads the prebuilt DuckDB archive over HTTPS without checking a checksum (V29), so a tampered or replaced release asset would be linked into development and CI builds. | `duckdb.sha256` pins the SHA-256 of each target's archive, taken from the GitHub release's asset digests (V68). `just duckdb-verify` hashes the cached archive for the current target and fails on a mismatch or a missing pin (V69). `just check` runs it after `lint`, which runs the build script, and before `test` and `bdd` load the library. Residual risks: the build script has already linked the library when the check runs, so a bad archive fails the check rather than the build; and the pins trust the asset GitHub held when they were taken. |

---

## Appendix A — justfile recipe contract (names frozen)

| Recipe | Does |
|---|---|
| `setup` | Rust toolchain components; per-OS notes |
| `setup-providers` | Installs the pinned providers from `providers.toml` |
| `check` | `override-check`, `fmt-check`, `lint`, `duckdb-verify`, `license-check`, `test`, `bdd`, `tags-check` |
| `duckdb-verify [TARGET]` | Checks the cached prebuilt libduckdb archive for the host (or TARGET) against its pin in `duckdb.sha256` (R8) |
| `override-check` | Fails while `.cargo/config.toml` holds a local override (a `dev-tagma` block or any `[patch]`) |
| `license-check` | `cargo deny check licenses bans` (rule 12) |
| `fmt` / `fmt-check` / `lint` / `test` | cargo equivalents; clippy with `-D warnings` |
| `lint-cross` | `lint` for the `x86_64-pc-windows-msvc` and `aarch64-apple-darwin` targets from any host; fails with the `rustup target add` hint if a target is missing. Not part of `check` (CI lints natively) |
| `bdd` | Features that need no mount, provider, or privilege |
| `tags-check` | Validates this repo's `.codetags/tags` |
| `test-mount` | View, plugin, and mount-tagging features against this OS's live backend |
| `test-providers` | Index features against the fixtures |
| `baseline-check` | Edge-count regression: indexes each ingested provider fixture and diffs its per-file edge counts against `tests/baselines/` |
| `baseline-update` | Rewrites `tests/baselines/` from fresh fixture indexes; deliberate, after reviewing the `baseline-check` diff |
| `test-privileged` | Helper features (CI only) |
| `test-claude` | M3 stage 0 (D16-D18): the `@claude` features, Claude Code's LSP client observed live, headless (needs `claude`, logged in or `ANTHROPIC_API_KEY`) |
| `bench` | criterion |
| `eval CONDITION` | Evaluation condition A, B or C |
| `doctor` | Environment report: mount backend, helper, providers |
| `dogfood` | Index this repo and refresh its views (M1) |
| `lsp-record-setup` | M3 stage 0 (D18): installs `codetags-lsp` as the recorder this repo's Claude Code plugin runs rust-analyzer through, and prints the runbook steps (`docs/lsp-stage0.md`) |
| `dev-tagma PATH` | Local tagma override (a marked block in `.cargo/config.toml`; no PATH removes it) |

## Appendix B — view tree and tag-write rules

These are defaults. Once `features/views` and `features/tags` are approved, the features are normative.

```text
<views root>/<repo>/               # per-user directory, outside every workspace (C7)
  README.md  FACETS.md             # read first: layout, path profile, quoting, facets with counts
  files/<path>.skel                # brief §4.5
  modules.dot  unresolved/<module>.txt  orphans.txt
  arch/modules.mmd  arch/modules.md  arch/<module>.mmd     # mermaid plugin, root mount (P7)
  q/<elem>/<elem>/...              # live mounts only; each <elem> is one postfix element
      @files/<path>                # results, grouped by kind; only non-empty groups are listed
                                   # @files are the real source files (D11); @skel/<path>.skel the skeletons
      @symbols/<canonical>.sym
      @sites/<n>.site
      @views/callgraph.mmd         # result-set plugins (P7); listed only if the result set is non-empty
```

**Lookup in `q/<query>`.**

- A name starting with `@` is a result group, or `@views`.
- Any other name is appended to the query, as typed, as the next postfix element (on Windows, after private-use decoding; D15):
  - If the result is a valid prefix, it is a directory.
  - If it is invalid (e.g. operator underflow), the lookup returns ENOENT.
- `readdir` lists only result groups, so traversal is finite.
- `q/` itself is the empty query and lists nothing.
- The static tree has no `q/`. Its README points agents to `codetags q`.

**Tag writes and edits (live mounts).** See §2.10 for the operation mapping and §2.11 for the queue and recursion guard.
