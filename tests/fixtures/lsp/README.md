# Recorded Claude Code LSP sessions

Replay fixtures for the shim's tests (P3.4), which will play them back as a
fake client. They come from the live feature
`features/lsp/claude-client.feature` (M3 stage 0, PLAN.md D16-D18), one
directory per Claude Code version: `claude-code-<version>/`.

| file | session |
|---|---|
| `the-handshake.jsonl` | one hover on `Ledger` in `src/ledger.rs` (the rule "The handshake") |
| `document-sync.jsonl` | the ten-step stage-0 script (the rule "Document sync") |
| `*.marks.jsonl` | one line per scripted step: `{"step": …, "ts_ms": …}`, the Unix time its prompt was sent |

Each `.jsonl` is a `codetags-lsp record` log (format: docs/lsp-stage0.md,
"Log format") of rust-analyzer 1.96.1 under Claude Code, against a scratch
git copy of `tests/fixtures/rust`. Summarize one with:

```sh
codetags-lsp analyze tests/fixtures/lsp/claude-code-2.1.286/document-sync.jsonl \
    --marks tests/fixtures/lsp/claude-code-2.1.286/document-sync.marks.jsonl
```

## How they were made

```sh
CODETAGS_BDD_CLAUDE_SAVE=/some/dir just test-claude
cp /some/dir/claude-code-<version>/* tests/fixtures/lsp/claude-code-<version>/
```

`just test-claude` runs Claude Code headless with Haiku (docs/lsp-stage0.md
says how). With `CODETAGS_BDD_CLAUDE_SAVE` set, each live session's
recordings and marks are saved there, named after the feature's rule.

The steps sanitize every recording before saving or analyzing it:

- the scratch project directory becomes `/project` (so `rootUri` reads
  `file:///project`, and `cwd` `/project`);
- `$HOME` becomes `/home/user` (the rust-analyzer path, and standard-library
  locations in results);
- a recording that still holds `ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN`,
  or anything shaped like an Anthropic key fails the run instead of saving.

The replacements change string lengths, so a record's `bytes` field (the
original body's length) no longer matches its `msg`. A replay re-serializes
`msg` and frames it afresh. Process ids and times are left as recorded.

`features/lsp/claude-client-replay.feature` analyzes these files in every
CI job, so they stay parseable and keep saying what they said when they
were recorded. When a new Claude Code version behaves differently, add a new
`claude-code-<version>/` directory. Keep the old one: the shim must work
with both.
