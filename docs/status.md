# Status

Snapshot after the first overnight run, 2026-10-01. `PLAN.md` is the plan and
`docs/verification.md` holds the evidence; this file says where things stand
and what is waiting on whom.

## Done

Everything listed is on `main`. The last CI run on these changes, 36819097543 on `56904b3`, passed all 11 jobs. CI covers Linux, macOS and Windows, with
cross-job BDD coverage, unless a row says otherwise.

| Phase | What exists |
|---|---|
| P0 | The Rust workspace and `justfile`, with frozen recipe names. Gherkin BDD through cucumber. A CI matrix over three OSes, and a check that every scenario runs in some job. The cargo-deny licence policy (D9). The prebuilt DuckDB (R1), which made a cold `just check` about 1.5 min instead of about 12, with its download checked against a pinned SHA-256 (R8). |
| P0b | All four platform spikes. **S1** Linux FUSE through `fusermount3`. **S2** macOS NFSv3 loopback with `nfs3_server`. **S3** Windows WinFsp with winfsp-rs; Windows CI runs de-elevated (V18 solved). **S4** the Win32 name probe, which also verified that Git Bash's private-use mapping supports D15. |
| P1 | **P1.1** the generation store. **P1.2** names (D15): canonical names, the Windows private-use mapping, item ids, collision suffixes. **P1.3** SCIP ingest into DuckDB generations, plus `codetags index`. **P1.3c** naming defaults; this repo indexes with 0 collisions. **P1.4–P1.7** provider runners and fixtures for Rust, Go (with gocallgraph), TypeScript (with Jelly) and Python, green on all three OSes. **P1.8** `codetags report` (resolution rates by language and module), edge-count regression against the previous generation, and `just baseline-check` against `tests/baselines/`; Rust has a baseline, and the other languages get one once ingest supports them. |
| P4 | **P4.1–P4.2** the watcher and coalescer (notify), the same on all three OSes. **P4.4** the optional fanotify helper (Linux), with its privileged paths proven in CI. |
| Dogfood | `just dogfood` indexes this repo (Rust), and CI fails on a provider failure or any collision. |
| Specs and analyses | For review, under `docs/spec-drafts/`: views (P2.1), plugins (P7.1), CLI tagging (P6.1), and a P3 re-plan. `docs/proxy-zero-change.md` analyses using lspmux with no changes. |

## Waiting on you

Each item has the default currently built or proposed.

**Reviews that gate work:**
1. **D11–D13** (PLAN.md §2.10–§2.12): BATFS-style editing, the recursion guard, the change notifier. This gates P6.4–P6.6 and P4.6–P4.7.
2. **Spec drafts**, in `docs/spec-drafts/`, each with its questions at the top:
   - `views/` (P2.1): gates all of P2 and P5, and dogfood milestone M1.
   - `plugins/` (P7.1): gates P7.
   - `tags/` (P6.1): gates P6.
   - `p3-plan.md`: gates the proxy track, P3.

**Decisions:**

| # | Decision | Default |
|---|---|---|
| 3 | The lspmux questions: (a) and the transport are **decided** (D16, D17); still open are (b) multi-user Windows scope, (c) who writes the upstream `applyEdit` PR, (d) `codetags lsp setup` writing lspmux config, (e) pin rev `18861f9` | (b) out of v1; (c) you, gap accepted until then; (d) only via that explicit command; (e) pin now |
| 4 | Security: the lspmux handshake names the program to run, so any local user who can reach the loopback port can run programs as you. A Unix socket in a 0700 directory avoids it on Unix, as a config choice. | — |
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

## Remaining work

| Phase | Remaining | Blocked on |
|---|---|---|
| P1 | Ingest for Go, TS and Python (call-graph join, the Jelly union, name-match candidates); multi-language `codetags index`; P1.3b; trait expansion | Decisions 6 and 11 for parts; the rest is ready |
| P2 | `ViewFs`, renderers as built-in plugins, the tagma bridge, the materializer, `codetags q`/`show`, the eval harness | The P2.1 review; the eval repo |
| P3 | The `codetags-lsp` shim, lspmux setup, initialize rewriting, crash handling, the watcher as an lspmux client, M3 | The P3 re-plan review; decision 3 |
| P4 | P4.3 routing and readiness gate (needs P3); P4.4 Windows USN; P4.5 reindex loop; P4.6–P4.7 notifier | P3; the D13 review |
| P5 | Live mounts over `ViewFs` (FUSE, NFS, WinFsp), with the S2 rule of bumping directory mtimes | P2.2 |
| P6 | The tags file, the CLI, the overlay, the operation queue and guard, filesystem tagging | The P6.1 and D11/D12 reviews |
| P7 | The plugin registry, the declarative runtime and sandbox, Mermaid | The P7.1 review |
| Milestones | **M1** static dogfood (needs P2); **M2** live (P5.1); **M3** proxy (P3, P4) | as above |

## Housekeeping

- **Disk.** `/home` is at about 96–97%; most of that is outside this project. Each agent worktree has its own `target/`, of about 5 GB.
- **Agents.** Implementation agents use `.claude/agents/codetags-implementer.md` (Opus, medium effort).
- **Licence warnings.** `cargo deny` prints two SPDX parse warnings, because `winfsp` and `winfsp-sys` use the deprecated id `GPL-3.0`. They are harmless; a `[[licenses.clarify]]` entry would silence them.
- **Verification numbering.** `docs/verification.md` numbers are assigned in blocks to parallel agents, so there are gaps (V40–41, V45–47, V53–55, V59–61, V67, V101–103 and so on). The gaps are expected.
