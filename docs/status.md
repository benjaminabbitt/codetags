# Status

Snapshot, updated 2026-10-01 (after the first overnight run, and M3 stages 0–2). `PLAN.md` is the plan and
`docs/verification.md` holds the evidence; this file says where things stand
and what is waiting on whom.

## Done

Everything listed is on `main`. CI run 36906588385 on `904aa79`, after all of
M3 stages 0–2, passed all 12 jobs. CI covers Linux, macOS and Windows, with
cross-job BDD coverage, unless a row says otherwise.

| Phase | What exists |
|---|---|
| P0 | The Rust workspace and `justfile`, with frozen recipe names. Gherkin BDD through cucumber. A CI matrix over three OSes, and a check that every scenario runs in some job. The cargo-deny licence policy (D9). The prebuilt DuckDB (R1), which made a cold `just check` about 1.5 min instead of about 12, with its download checked against a pinned SHA-256 (R8). |
| P0b | All four platform spikes. **S1** Linux FUSE through `fusermount3`. **S2** macOS NFSv3 loopback with `nfs3_server`. **S3** Windows WinFsp with winfsp-rs; Windows CI runs de-elevated (V18 solved). **S4** the Win32 name probe, which also verified that Git Bash's private-use mapping supports D15. |
| P1 | **P1.1** the generation store. **P1.2** names (D15): canonical names, the Windows private-use mapping, item ids, collision suffixes. **P1.3** SCIP ingest into DuckDB generations, plus `codetags index`. **P1.3c** naming defaults; this repo indexes with 0 collisions. **P1.4–P1.7** provider runners and fixtures for Rust, Go (with gocallgraph), TypeScript (with Jelly) and Python, green on all three OSes. **P1.8** `codetags report` (resolution rates by language and module), edge-count regression against the previous generation, and `just baseline-check` against `tests/baselines/`; Rust has a baseline, and the other languages get one once ingest supports them. |
| P4 | **P4.1–P4.2** the watcher and coalescer (notify), the same on all three OSes. **P4.4** the optional fanotify helper (Linux), with its privileged paths proven in CI. |
| Dogfood | `just dogfood` indexes this repo (Rust), and CI fails on a provider failure or any collision. |
| M3 (P3) | **Stage 0:** Claude Code's LSP client is described in Gherkin (`features/lsp/claude-client.feature`): observed live and headless, with replays on every push. **Stages 1–2:** lspmux pinned at `18861f9` and configured by `codetags lsp setup`; the `codetags-lsp` shim (agent sessions never own documents; multi-root rejected; daemon started on demand); a project-scoped Claude Code plugin and VS Code wiring. **Freshness:** rust-analyzer's own watcher keeps it current through the shim, so Rust needs no stage 3 (V126). The live unopened-file rule now passes too (V143: all 32 `@claude` scenarios, 2026-10-01). **P3.14** (`features/lsp/wiring.feature`, V131) drives these on all three OSes: `lsp-setup` itself, both clients sharing one real rust-analyzer, and V126's freshness. It found one gap: on macOS, a checkout reached through a symlink never sees changes on disk (V132), which D26 fixed: the shim now canonicalizes the root and rewrites URIs both ways (V133). Before that, the shim made it worse for VS Code there. It also adds the wrapper's serve check and doctor's `lsp wiring:` line. **On the dev host, setup ran on 2026-10-01.** Before that, the `codetags-lsp` there was a stage-0 build with no `serve` (V142). A fresh headless Claude Code now loads the plugin, and its sessions share one rust-analyzer through lspmux (V140). Their first answers were empty while this repo loaded, for over a minute (V141), which is P3.12's case. VS Code is on the shim once its window is reloaded. |
| Specs and analyses | For review, under `docs/spec-drafts/`: views (P2.1), plugins (P7.1), CLI tagging (P6.1), and a P3 re-plan. `docs/proxy-zero-change.md` analyses using lspmux with no changes. |

## Waiting on you

Each item has the default currently built or proposed.

**Reviews that gate work:**
1. **D11–D13** (PLAN.md §2.10–§2.12): BATFS-style editing, the recursion guard, the change notifier. This gates P6.4–P6.6 and P4.6–P4.7.
2. **Spec drafts**, in `docs/spec-drafts/`, each with its questions at the top:
   - `views/` (P2.1): gates all of P2 and P5, and dogfood milestone M1.
   - `plugins/` (P7.1): gates P7.
   - `tags/` (P6.1): gates P6.
   - `p3-plan.md`: most of it is settled. Questions (a), (d), (e) and 3 are
     decided (D16, D20, D19, D17), and 1, 2 and 6 are built as proposed (2
     with sh wrappers on Linux and macOS, executables on Windows, V119).
     Questions 4 and 5 are decided too: all three configuration policies
     are in v1 (D24), and the root is canonicalized with URIs rewritten (D26, superseding D23). Still
     open: (b) and (c) (decision 3), and 7 (the daemon's environment leaks
     into every server). These gate P3.4's toolchain key, not the rest of
     P3.

**Decisions:**

| # | Decision | Default |
|---|---|---|
| 3 | The lspmux questions: (a), (d), (e) and the transport are **decided** (D16, D17, D19, D20); still open are (b) multi-user Windows scope and (c) who writes the upstream `applyEdit` PR. rust-analyzer never sends `applyEdit`, so (c) only matters for gopls. | (b) out of v1; (c) you, gap accepted until then |
| 4 | **Decided (D17):** a Unix socket in a 0700 directory on Linux and macOS. Windows stays loopback TCP, which is (b) in decision 3. | — |
| 5 | Naming (PLAN §2.7): impl blocks are containers; `+fn`, `+field`, `+const` and `+static` suffixes only on a collision; Rust names start with the crate; the standard library and dependencies get their crate too; value precedence on a collision is fn, then const, static, field | as listed |
| 6 | Rust trait and `dyn` expansion: rust-analyzer emits no `is_implementation` relationships (V64), while Go, TS and Python do. Options: match by name, ask LSP for implementations, or accept the gap. | `declared` targets only |
| 7 | The Unlicense on `nfs3_types` and `nfs3_macros` (S2) | scoped exceptions in `deny.toml`; `nfsserve` is the fallback |
| 8 | Windows watcher overflow: notify 8.2 drops it (V56). Options: notify 9.0-rc, or a periodic reconcile. | neither yet |
| 9 | Git merges of the tags file conflict on *adjacent* lines too (V70). Options: a per-item merge driver, or accept git's behaviour. | — |
| 10 | Windows names with a trailing dot or space, and the `NUL` device-name note (S4) | — |
| 11 | P1.3b, control context and literal names per call site, needs a parser beyond SCIP (tree-sitter?) | split out, not started |
| 12 | Should `codetags index` print the resolution report after every run (brief: "after every run")? | — |
| 13 | Review the privileged helper's security model before anyone installs it (`codetags-privhelper` crate docs). Should writer PIDs reach watcher batches, for D12? | — |
| 14 | The eval repo for conditions A, B and C (brief §4.6, still [OPEN]) | — |
| 15 | scip-python 0.6.6 can't start on Windows (open upstream bug sourcegraph/scip-python#210). `just setup-providers` patches the installed package (`tools/patch-scip-python.js`), with guards, until the fix (#224) is released. The alternative is a Windows exception for Python (R2). | the patch |
| 16 | Add an `ANTHROPIC_API_KEY` repo secret, so the live Claude Code LSP test (`claude-client` job) runs in CI | not set; the job skips itself |

## Remaining work

| Phase | Remaining | Blocked on |
|---|---|---|
| P1 | Ingest for Go, TS and Python (call-graph join, the Jelly union, name-match candidates); multi-language `codetags index`; P1.3b; trait expansion | Decisions 6 and 11 for parts; the rest is ready |
| P2 | `ViewFs`, renderers as built-in plugins, the tagma bridge, the materializer, `codetags q`/`show`, the eval harness | The P2.1 review; the eval repo |
| P3 | P3.12 the readiness gate; P3.13 live-test follow-ups; P3.6's configuration merge now includes the cross-session coordinator (D24); P3.4 routing-key injection and wrappers for gopls, pyright and the TS server; P3.5 the full fixed capability set; P3.6 configuration merge; P3.7 crash recovery; P3.8 the watcher client (gopls); P3.10 gopls, real VS Code automation and the Windows `.exe` wrappers; P3.11 the upstream PR notes | (c) for P3.11; the rest is ready |
| P4 | P4.3 routing and readiness gate (needs P3); P4.4 Windows USN; P4.5 reindex loop; P4.6–P4.7 notifier | P3; the D13 review |
| P5 | Live mounts over `ViewFs` (FUSE, NFS, WinFsp), with the S2 rule of bumping directory mtimes | P2.2 |
| P6 | The tags file, the CLI, the overlay, the operation queue and guard, filesystem tagging | The P6.1 and D11/D12 reviews |
| P7 | The plugin registry, the declarative runtime and sandbox, Mermaid | The P7.1 review |
| Milestones | **M1** static dogfood (needs P2); **M2** live (P5.1); **M3** usable with rust-analyzer after `just setup-lspmux` and `just lsp-setup`, polish in P3.12 and P3.13 | as above |

## Housekeeping

- **Disk.** `/home` is at about 96–97%; most of that is outside this project. Each agent worktree has its own `target/`, of about 5 GB.
- **Agents.** Implementation agents use `.claude/agents/codetags-implementer.md` (Opus, medium effort).
- **Licence warnings.** `cargo deny` prints two SPDX parse warnings, because `winfsp` and `winfsp-sys` use the deprecated id `GPL-3.0`. They are harmless; a `[[licenses.clarify]]` entry would silence them.
- **Verification numbering.** `docs/verification.md` numbers are assigned in blocks to parallel agents, so there are gaps (V40–41, V45–47, V53–55, V59–61, V67, V101–103 and so on). The gaps are expected.
