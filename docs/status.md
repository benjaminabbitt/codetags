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

**Reviews that gate work:**
1. **D11–D13** (PLAN.md §2.10–§2.12): BATFS-style editing, the recursion guard, the change notifier. This gates P6.4–P6.6 and P4.6–P4.7.
2. **Spec drafts**, in `docs/spec-drafts/`, each with its questions at the top:
   - `views/` (P2.1): gates all of P2 and P5, and dogfood milestone M1.
   - `plugins/` (P7.1): gates P7.
   - `tags/` (P6.1): gates P6. Its merge questions are answered by D39
     (a per-item merge driver).

**Waiting on your action:**
- Add the `ANTHROPIC_API_KEY` repository secret (D30), then say so: the
  `claude-client` job's expected-skip entries go in the same change.
- Write the upstream lspmux PR once the P3.11 notes exist (D29).
- Sign off the privileged helper's threat model once it is written (D40).

**Decisions:** none open. The 15 that were listed here were decided on
2026-10-01 as D27–D41 (PLAN.md §1.1). The `p3-plan.md` draft is fully
settled (D16, D17, D19, D20, D23–D24, D26–D29).

## Remaining work

| Phase | Remaining | Blocked on |
|---|---|---|
| P1 | Ingest for Go, TS and Python (call-graph join, the Jelly union, name-match candidates); multi-language `codetags index`; the summary and drop check after every run (D33); trait expansion through `textDocument/implementation` (D31); P1.3b through tree-sitter, after a spike (D32) | ready |
| P2 | `ViewFs`, renderers as built-in plugins, the tagma bridge, the materializer, `codetags q`/`show`, the eval harness (on codetags itself, D41) | The P2.1 review |
| P3 | P3.12 the readiness gate; P3.13 live-test follow-ups; P3.6's configuration merge now includes the cross-session coordinator (D24); P3.4 routing-key injection, the minimal daemon environment and toolchain key (D27), and wrappers for gopls, pyright and the TS server; P3.5 the full fixed capability set; P3.6 configuration merge; P3.7 crash recovery; P3.8 the watcher client (gopls); P3.10 gopls, real VS Code automation and the Windows `.exe` wrappers; P3.11 the upstream PR notes, for you to write the PR from (D29) | ready |
| P4 | notify 9.0.0-rc.5 (D35); P4.3 routing (needs P3.8); P4.4 Windows USN; P4.5 reindex loop; P4.6–P4.7 notifier; the helper's threat model (D40) | P3.8; the D13 review |
| P5 | Live mounts over `ViewFs` (FUSE, NFS, WinFsp), with the S2 rule of bumping directory mtimes | P2.2 |
| P6 | The tags file, the CLI, the overlay, the operation queue and guard, filesystem tagging | The P6.1 and D11/D12 reviews |
| P7 | The plugin registry, the declarative runtime and sandbox, Mermaid | The P7.1 review |
| Milestones | **M1** static dogfood (needs P2); **M2** live (P5.1); **M3** usable with rust-analyzer after `just setup-lspmux` and `just lsp-setup`, polish in P3.12 and P3.13 | as above |

## Housekeeping

- **Disk.** `/home` is at about 96–97%; most of that is outside this project. Each agent worktree has its own `target/`, of about 5 GB.
- **Agents.** Implementation agents use `.claude/agents/codetags-implementer.md` (Opus, medium effort).
- **Licence warnings.** `cargo deny` prints two SPDX parse warnings, because `winfsp` and `winfsp-sys` use the deprecated id `GPL-3.0`. They are harmless; a `[[licenses.clarify]]` entry would silence them.
- **Verification numbering.** `docs/verification.md` numbers are assigned in blocks to parallel agents, so there are gaps (V40–41, V45–47, V53–55, V59–61, V67, V101–103 and so on). The gaps are expected.
