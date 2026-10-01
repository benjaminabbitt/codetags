# M3 stage 0: record Claude Code's LSP traffic

**For the human.** One interactive Claude Code session in this repo, with
rust-analyzer running through `codetags-lsp record`, which passes every byte
through unchanged and logs each LSP message as one JSON line. The recording
settles the reported client gaps (V110) before the shim's design (P3.4)
depends on them (PLAN.md D16–D18, `docs/proxy-zero-change.md` §3.5).

## 1. What is in the repo

- `tools/claude-plugins/`: a local marketplace, `codetags-local`, with one
  plugin, `codetags-lsp-recorder`. Its `lspServers.rust-analyzer` runs
  `tools/claude-plugins/codetags-lsp-recorder/scripts/rust-analyzer-recorder`
  for `.rs` files, with `workspaceFolder` = `${CLAUDE_PROJECT_DIR}`. Claude
  Code loads it in place from the main checkout (V108).
- `.claude/settings.json`: registers that marketplace, enables the recorder
  plugin, and disables `rust-analyzer-lsp@claude-plugins-official` **for this
  project only** (V109). Other projects keep your user settings.
- The wrapper runs `.codetags/local/bin/codetags-lsp record --server <real
  rust-analyzer> --log $CLAUDE_PROJECT_DIR/.codetags/local/lsp-{ts}-{pid}.jsonl`.
  Until `just lsp-record-setup` has run, it runs `rust-analyzer` from `PATH`
  unrecorded, so sessions (and agent worktrees) still get an LSP.

Differences from the official plugin, which runs `rust-analyzer` from `PATH`
with no other settings: the real server is this toolchain's rust-analyzer by
absolute path (the rustup proxy would pick the same one here), and
`workspaceFolder` is set, as the shim will set it. A non-null root in this
recording therefore does not refute "`rootUri: null`" for the official plugin.

## 2. Start the session

1. In the **main checkout** (not a worktree), run `just lsp-record-setup`. It
   builds `codetags-lsp`, copies it to `.codetags/local/bin/`, writes the real
   rust-analyzer's path to `.codetags/local/rust-analyzer.path`, and prints
   these steps.
2. Quit every Claude Code session in this repo, so the recording comes from
   one fresh session.
3. From the repo root, run `claude`. If Claude Code asks whether to trust the
   folder, accept: project marketplaces apply only in trusted folders.
4. Run `/plugin`. Check that `codetags-lsp-recorder@codetags-local` is listed
   and enabled, that `rust-analyzer-lsp@claude-plugins-official` is disabled,
   and that the **Errors** tab is empty. If the recorder is not listed, run
   `/reload-plugins` once, then check again.
5. Note the wall-clock time. Every log line has `ts_ms` (Unix milliseconds),
   so your notes can be matched to the log.

The language server starts when Claude first touches a `.rs` file, so step 1
of the script below starts it.

## 3. Scripted actions

Type each prompt as given, one at a time, and wait for Claude to finish.
Write down the time before each step. Do not commit anything; step 10 undoes
every change.

1. **Read.** `Read crates/codetags-model/src/store.rs with the Read tool and summarize it in one sentence.`
2. **Definition.** `Use the LSP tool (goToDefinition) on the use of GenerationWriter in crates/codetags-model/src/store.rs. Report the file and line.`
3. **References.** `Use the LSP tool (findReferences) on GenerationStore::open_newest in crates/codetags-model/src/store.rs. List the files.`
4. **Hover.** `Use the LSP tool (hover) on StoreError where it is declared in crates/codetags-model/src/error.rs. Quote the hover text.`
5. **Implementations.** `Use the LSP tool (goToImplementation) on the trait std::error::Error as named in crates/codetags-model/src/error.rs, and on StoreError. Report what it returns.`
6. **Call hierarchy.** `Use the LSP tool for the call hierarchy of GenerationStore::gc in crates/codetags-model/src/store.rs: prepareCallHierarchy, then incomingCalls and outgoingCalls.`
7. **Edit.** `With the Edit tool, add the line "// stage0: edit tool" at the end of crates/codetags-model/src/reader.rs. Then use the LSP tool (hover) on Generation in that file.`
8. **Write a new file.** `With the Write tool, create crates/codetags-model/src/stage0_scratch.rs containing "pub fn stage0() -> u32 { 1 }". Then use the LSP tool (documentSymbol) on it.`
9. **Change through Bash.** `Run this with the Bash tool: sed -i 's|// stage0: edit tool|// stage0: sed|' crates/codetags-model/src/reader.rs. Then use the LSP tool (hover) on Generation in crates/codetags-model/src/reader.rs again.`
10. **git checkout.** `Run with the Bash tool: git checkout -- crates/codetags-model/src/reader.rs && rm crates/codetags-model/src/stage0_scratch.rs. Then use the LSP tool (findReferences) on Generation in crates/codetags-model/src/reader.rs.`
11. **End.** Type `/exit`. Wait for Claude Code to quit; it shuts the server down (and kills it if shutdown times out).

Afterwards `git status` must be clean apart from `.codetags/local/`, which is
gitignored.

## 4. Where the log lands, and what to send back

- Logs: `.codetags/local/lsp-<UTC start time>-<recorder pid>.jsonl` in the
  project directory, one file per server process. More than one file means
  Claude Code restarted the server (or started a second one); keep them all.
- Summary: `.codetags/local/bin/codetags-lsp analyze <log>`. It prints the
  `initialize` params (`rootUri`, `workspaceFolders`, capabilities), every
  `didOpen`/`didChange`/`didSave`/`didClose`/`didChangeWatchedFiles` with its
  time, every server-to-client request with the client's answer, the client's
  requests with their latencies, and a verdict on each reported gap.
- Hand the log(s) and your timed notes to the agent doing stage 1; it
  records the findings in `docs/verification.md` (V110) and the shim design.

The log holds file contents Claude Code sent (`didOpen` text) and your
paths. It stays in `.codetags/local/`, which is never committed.

## 5. Log format

One JSON object per line; each has `ts_ms` (Unix ms) and `t_ms` (ms since
the recorder started).

| record | fields |
|---|---|
| start | `event: "start"`, `server`, `args`, `pid` (server), `recorder_pid`, `cwd`, `format: 1` |
| message | `from: "client" \| "server"`, `kind: "request" \| "response" \| "notification" \| "batch" \| "invalid"`, `method`, `id`, `bytes`, `msg` (the parsed body); `headers` if any besides `Content-Length`; `raw` and `parse_error` if the body is not JSON |
| malformed | `event: "malformed"`, `side`, `reason`, `raw`: the stream stopped being framed LSP; the rest is passed through unrecorded |
| eof | `event: "eof"`, `side`, `error`: that side closed its stream |
| exit | `event: "exit"`, `status`, `signal`: the server exited; the recorder exits the same way |

## 6. If something goes wrong

- **No log file.** Check `/plugin` → **Errors**. Check that
  `.codetags/local/bin/codetags-lsp` and `.codetags/local/rust-analyzer.path`
  exist in the main checkout; if not, the wrapper ran `rust-analyzer`
  unrecorded and said so on stderr.
- **The LSP tool says no server.** The server starts on the first `.rs`
  file Claude touches; repeat step 1.
- **Undo the setup.** Delete `.codetags/local/bin/` and
  `.codetags/local/rust-analyzer.path`; the plugin then runs rust-analyzer
  unrecorded. To opt out of the plugin on your machine only, set
  `"codetags-lsp-recorder@codetags-local": false` in
  `.claude/settings.local.json` (V109).
