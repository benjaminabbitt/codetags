# M3 stage 0: observing Claude Code's LSP client

Stage 0 (PLAN.md D16-D18, `docs/proxy-zero-change.md` §3.5) records Claude
Code's real LSP traffic before the shim's design (P3.4) depends on the
client gaps others reported. rust-analyzer runs through `codetags-lsp
record`, which passes every byte through unchanged and logs each message as
one JSON line. An automated test drives Claude Code and asserts what the
client does. The findings, with the Claude Code version, are in
`docs/verification.md` V110-V115.

## 1. Run it

```sh
just test-claude
```

It needs:

- `claude` on `PATH`, either logged in (`claude auth login`) or with
  `ANTHROPIC_API_KEY` set; the recipe checks both and says which is missing;
- this toolchain's rust-analyzer (`rustup component add rust-analyzer`), and
  `git`;
- Linux or macOS: the plugin's wrapper is a sh script.

Each run drives two Haiku sessions of a few minutes in all, at a small cost
each time. To keep the sanitized recordings, set
`CODETAGS_BDD_CLAUDE_SAVE=<dir>`. `tests/fixtures/lsp/README.md` explains how
they become replay fixtures.

In CI, the `claude-client` job runs `just test-claude` only when the
repository has the `ANTHROPIC_API_KEY` secret. Until then,
`ci/expected-skips.txt` lists the scenarios, and once the secret is added
those entries must go (the file says so). The job installs the latest Claude
Code, so a release whose client behaves differently fails it. Its
`claude-recordings` artifact holds the new recordings.

## 2. What the test does

`features/lsp/claude-client.feature` (`@claude`, `@serial`) is plain Gherkin,
run by the BDD runner; `docs/steps.md` ("Claude Code's LSP client") defines
its steps.

- **Setup.** `Given a scratch Rust project from the fixture "rust" with the
  LSP recorder plugin` copies `tests/fixtures/rust` into a scratch git repo
  (one commit). It gives the copy its own recorder setup in
  `.codetags/local/`: a copy of `codetags-lsp`, and the toolchain's
  rust-analyzer path in `rust-analyzer.path`. The plugin's wrapper uses a
  project's own setup in preference to the checkout's, and the logs go to
  that project's `.codetags/local/`.
- **Claude Code.** The first action starts one headless session in the
  project:

  ```sh
  claude -p --model claude-haiku-4-5-20251001 \
    --input-format stream-json --output-format stream-json --verbose \
    --plugin-dir tools/claude-plugins/codetags-lsp-recorder \
    --setting-sources project \
    --settings '{"enabledPlugins":{"rust-analyzer-lsp@claude-plugins-official":false}}' \
    --strict-mcp-config --tools Read,Edit,Write,Bash,LSP \
    --allowedTools Read Edit Write LSP 'Bash(<each scripted command>)' \
    --permission-mode dontAsk --max-turns 8 --max-budget-usd 1 \
    --no-session-persistence --append-system-prompt '<keep to the script>'
  ```

  Each `When Claude Code …` step sends one user turn and waits for its
  result. It notes the time as a step mark, and fails if Claude did not use
  the tools the step names. After the scenario's last action, stdin closes,
  Claude Code exits, and the recording is sanitized and then analyzed with
  `codetags-lsp analyze --marks`.
- **Facts.** Each `Then` step reads one fact from that analysis: the
  `initialize` parameters and capabilities, the client's answers to server
  requests, and the document notifications during each step. Every scenario
  is one fact, so a change in Claude Code fails exactly the facts it changes.
- **Sharing.** Every scenario of a rule needs the same session. Within one
  run, the first scenario runs it, and the others replay its outcome, or its
  failure. That gives two live sessions per run: "The handshake" (one hover)
  and "Document sync" (the ten-step script below).

### Choices that make it work headless (V111-V113)

- **Plugin by `--plugin-dir`.** `claude -p` never shows the trust dialog,
  and in an untrusted folder it ignores the repository's
  `extraKnownMarketplaces`, so this repo's `.claude/settings.json` route
  loads nothing headless. `--plugin-dir` loads the plugin for the one
  session. `--bare` cannot be used: it turns LSP off.
- **No user configuration.** `--setting-sources project` (the scratch
  project has no settings) and `--strict-mcp-config` keep the user's
  plugins, hooks, permissions and MCP servers out. Login still works:
  credentials are not settings. The account may still supply plugins of its
  own (V113), so the test asserts only that the recorder loads and the
  official rust-analyzer plugin does not.
- **Permissions per invocation.** `--permission-mode dontAsk` denies anything
  not pre-approved, and `--allowedTools` approves exactly the scripted Bash
  commands. Nothing weakens permissions beyond the one scratch session.
- **Nested sessions.** Run from inside Claude Code, `claude -p` starts
  anyway (V112). The steps still remove the parent session's variables:
  every `CLAUDE*` except `CLAUDE_CONFIG_DIR` and `CLAUDE_CODE_OAUTH_TOKEN`,
  and `AI_AGENT`.

### The script

The "Document sync" rule mirrors the manual runbook (appendix), on the
fixture. The step mark of each action is in parentheses.

1. (Read) Read `src/ledger.rs` with the Read tool.
2. (definition) goToDefinition on the use of `Ledger` in `src/lib.rs`.
3. (references) findReferences on `record` in `src/ledger.rs`.
4. (hover) hover on `Ledger` in `src/ledger.rs`.
5. (implementations) goToImplementation on `Apply` in `src/charge.rs`.
6. (call hierarchy) prepareCallHierarchy, incomingCalls, outgoingCalls on
   `settle` in `src/lib.rs`.
7. (Edit) Edit: append `// stage0: edit tool` to `src/ledger.rs`, then hover.
8. (Write) Write `src/stage0_scratch.rs`, then documentSymbol on it.
9. (sed) Bash: `sed -i 's|// stage0: edit tool|// stage0: sed|' src/ledger.rs`,
   then hover.
10. (git checkout) Bash: `git checkout -- src/ledger.rs && rm
    src/stage0_scratch.rs`, then hover.

### What it cannot automate

- **Interactive mode.** The test runs `claude -p`. The trust dialog, `/plugin`
  and `/reload-plugins`, and the reported interactive-only behaviour
  (#79744: no `didChange` after `didOpen`) need a person at a terminal. Use
  the appendix for that.
- **The model's choices.** Haiku decides how to use each tool. For example,
  it may hover again when the first hover returns nothing while
  rust-analyzer is still indexing. So no `Then` step counts requests;
  assertions are about document sync, which the client drives, and about
  the handshake.

## 3. Log format

One JSON object per line; each has `ts_ms` (Unix ms) and `t_ms` (ms since
the recorder started).

| record | fields |
|---|---|
| start | `event: "start"`, `server`, `args`, `pid` (server), `recorder_pid`, `cwd`, `format: 1` |
| message | `from: "client" \| "server"`, `kind: "request" \| "response" \| "notification" \| "batch" \| "invalid"`, `method`, `id`, `bytes`, `msg` (the parsed body); `headers` if any besides `Content-Length`; `raw` and `parse_error` if the body is not JSON |
| malformed | `event: "malformed"`, `side`, `reason`, `raw`: the stream stopped being framed LSP; the rest is passed through unrecorded |
| eof | `event: "eof"`, `side`, `error`: that side closed its stream |
| exit | `event: "exit"`, `status`, `signal`: the server exited; the recorder exits the same way |

Under Claude Code a recording has no `eof` or `exit` record: Claude Code
kills the server after `shutdown` (V115).

Step marks (`--marks`) are one JSON object per line, `{"step": <name>,
"ts_ms": <Unix ms>}`. A step lasts from its mark to the next one. The
analysis's "Client messages per step" section lists, for each step, the
document notifications in order and the distinct request and other
notification methods.

## Appendix: the manual session (fallback)

Use this to record an **interactive** session, which the automated test
cannot do.

### A.1 What is in the repo

- `tools/claude-plugins/`: a local marketplace, `codetags-local`, with one
  plugin, `codetags-lsp-recorder`. Its `lspServers.rust-analyzer` runs
  `tools/claude-plugins/codetags-lsp-recorder/scripts/rust-analyzer-recorder`
  for `.rs` files, with `workspaceFolder` = `${CLAUDE_PROJECT_DIR}`. Claude
  Code loads it in place from the main checkout (V108).
- `.claude/settings.json`: registers that marketplace, enables the recorder
  plugin, and disables `rust-analyzer-lsp@claude-plugins-official` **for this
  project only** (V109). Other projects keep your user settings.
- The wrapper runs `.codetags/local/bin/codetags-lsp record --server <real
  rust-analyzer> --log $CLAUDE_PROJECT_DIR/.codetags/local/lsp-{ts}-{pid}.jsonl`,
  using the project's own `.codetags/local` setup if it has one, else the
  main checkout's. Until `just lsp-record-setup` has run, it runs
  `rust-analyzer` from `PATH` unrecorded, so sessions (and agent worktrees)
  still get an LSP.

Differences from the official plugin, which runs `rust-analyzer` from `PATH`
with no other settings: the real server is this toolchain's rust-analyzer by
absolute path (the rustup proxy would pick the same one here), and
`workspaceFolder` is set, as the shim will set it. A non-null root in a
recording therefore does not refute "`rootUri: null`" for the official
plugin.

### A.2 Start the session

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
5. Note the wall-clock time before each step below. Every log line has
   `ts_ms` (Unix milliseconds), so your notes can be matched to the log; or
   write them as a marks file (section 3) for `codetags-lsp analyze --marks`.

The language server starts when Claude first uses the LSP tool, so step 2 of
the script below starts it.

### A.3 Scripted actions

Type each prompt as given, one at a time, and wait for Claude to finish. Do
not commit anything; step 10 undoes every change.

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
11. **End.** Type `/exit`. Claude Code shuts the server down.

Afterwards `git status` must be clean apart from `.codetags/local/`.

### A.4 Where the log lands

- Logs: `.codetags/local/lsp-<UTC start time>-<recorder pid>.jsonl` in the
  project directory, one file per server process. More than one file means
  Claude Code restarted the server, or started a second one; keep them all.
- Summary: `.codetags/local/bin/codetags-lsp analyze <log> [--marks <file>]`.
  It prints the `initialize` params and capabilities, every document
  notification with its time, every server-to-client request with the
  client's answer, the client's requests with their latencies, and a verdict
  on each reported gap.
- Compare it with V110, which holds the headless facts.

A log holds the file contents Claude Code sent (`didOpen` text) and your
paths. It stays in `.codetags/local/`, which is never committed; sanitize
it as `tests/fixtures/lsp/README.md` describes before sharing it.

### A.5 If something goes wrong

- **No log file.** Check `/plugin` → **Errors**. Check that
  `.codetags/local/bin/codetags-lsp` and `.codetags/local/rust-analyzer.path`
  exist in the main checkout; if not, the wrapper ran `rust-analyzer`
  unrecorded and said so on stderr.
- **The LSP tool says no server.** The server starts at the first LSP tool
  use; repeat step 2.
- **Undo the setup.** Delete `.codetags/local/bin/` and
  `.codetags/local/rust-analyzer.path`; the plugin then runs rust-analyzer
  unrecorded. To opt out of the plugin on your machine only, set
  `"codetags-lsp-recorder@codetags-local": false` in
  `.claude/settings.local.json` (V109).
