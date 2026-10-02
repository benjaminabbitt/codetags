# M3 stages 1 and 2: one rust-analyzer for Claude Code and VS Code

Stages 1 and 2 of M3 (PLAN.md D14-D22, `docs/proxy-zero-change.md` §3.5 and
§7) run this repo's rust-analyzer once, shared by Claude Code and VS Code,
through **upstream lspmux, unmodified**, pinned at `18861f9` (D19). The
BSD-3 parts are ours: the shim (`codetags-lsp serve`), its wrappers,
`codetags lsp setup` and the doctor's checks. Stage 0, which recorded Claude
Code's real LSP traffic first, is `docs/lsp-stage0.md`.

```
Claude Code ─stdio─> plugin wrapper ─> codetags-lsp serve --role agent  ─┐
VS Code     ─stdio─> tools/vscode/rust-analyzer ─> … --role editor     ─┴─ lspmux client ─ socket ─> lspmux server ─> rust-analyzer
```

## 1. Set it up (this repo)

Linux and macOS:

```sh
just setup-lspmux        # builds the pinned lspmux into .codetags/local/bin (about a minute, once)
just lsp-setup           # builds, then runs tools/lsp/lsp-setup.sh: copies the shim to
                         # .codetags/local/bin, records this toolchain's rust-analyzer, then
                         # runs `codetags lsp setup`, which shows the change to lspmux's
                         # config and asks before writing it
just doctor              # lspmux's revision, its config, the socket directory, and
                         # "lsp wiring": whether this checkout's clients use the shim
```

`tools/lsp/lsp-setup.sh CODETAGS_LSP CODETAGS [options]` wires the checkout
it lives in, with the given binaries, and passes any options to `codetags
lsp setup`; the wiring scenarios (§4) run it in a scratch checkout.

Then restart Claude Code in this checkout (`/plugin` lists
`codetags-lsp@codetags-local`, enabled, and the official `rust-analyzer-lsp`
disabled, by `.claude/settings.json`), and reload VS Code: this repo's
`.vscode/settings.json` points `rust-analyzer.server.path` at
`${workspaceFolder}/tools/vscode/rust-analyzer` (V118). Both attach to the
same rust-analyzer; `.codetags/local/bin/lspmux status` shows one instance
with two clients.

Windows: the same three commands, from Git Bash. VS Code and Claude Code
cannot start a script there (V119), so `just lsp-setup` also installs
`rust-analyzer.exe` beside each sh wrapper: a copy of `codetags-lsp` with its
settings in `rust-analyzer.codetags-lsp.toml` (both gitignored). lspmux has
no Unix socket on Windows, so the daemon listens on loopback TCP, which any
local user can reach (C-3; `codetags doctor` says so).

Until setup has run, both wrappers run rust-analyzer from `PATH` directly,
as before. They also run the recorded rust-analyzer directly, without the
shim, when `.codetags/local/bin/codetags-lsp serve --help` fails (a stage-0
build from `just lsp-record-setup` has no `serve`; D25) or no lspmux is
found. Each fallback says why on stderr, with the fix. `codetags doctor`
reports the same on its `lsp wiring:` line, for the checkout in its working
directory: through the shim, or without it and why (`run: just lsp-setup` or
`run: just setup-lspmux`). On Windows that line checks the `.exe` wrappers
and their settings instead. It is information, never a failure, since the
fallback is deliberate.

## 2. What each part does

**`codetags lsp setup`** is the only writer of lspmux's config file (D20),
which lspmux reads from one fixed per-user path (V116). It writes:

- `listen` and `connect`: on Linux and macOS a socket in a 0700 directory,
  `$XDG_RUNTIME_DIR/codetags/lspmux.sock` (else `$XDG_STATE_HOME/codetags/…`,
  else `~/.local/state/codetags/…`); on Windows `127.0.0.1:27631` (D17).
  `--listen` takes another loopback address or socket path; anything else is
  refused;
- `pass_environment = ["CODETAGS_KEY_*"]`: only the routing-key variables
  the shim sets, never raw `PATH` (R8, R9);
- `instance_timeout = 300`.

It prints the difference from the current file, asks (`--yes` skips the
question), and keeps the old file as `config.toml.bak-<UTC time>`. A running
daemon keeps its old settings until it restarts.

**`codetags doctor`** reports lspmux's path and revision (from cargo's
`.crates.toml`), whether the config is exactly what setup writes (drift
fails), whether lspmux would even parse it (a typo makes lspmux silently use
its defaults, V36), whether `lspmux config` loads it (V124), and whether the
listen address keeps other users out.

**`codetags-lsp serve --role editor|agent --server <rust-analyzer>`** (the
shim):

- runs a non-LSP invocation (`--version`, `--help`, a subcommand) on the real
  server directly, without lspmux (R2; VS Code checks `--version`, V118);
- refuses a multi-root `initialize` with LSP error -32602 itself, so lspmux's
  `assert!` is never reached (R12, V4);
- normalizes the root to the project root, for rust-analyzer the outermost
  Cargo workspace containing the crate (brief §4.2), **spelled canonically**
  (D26, superseding D23): symlinks resolved, Windows short names expanded,
  no `\\?\` prefix. That root goes in `workspaceFolders[0]`, `rootUri` and
  `rootPath`, and `CODETAGS_KEY_ROOT` is set to it (R10, R11), so every
  spelling of one checkout shares one server;
- **rewrites URIs both ways** (D26, `codetags_lsp::uri`) when the client's
  spelling of the root differs from the canonical one. In every message to
  the server, any `file:` URI under the client's root, as a JSON string or
  object key anywhere under `params` or `result`, is respelled under the
  canonical root, and in every message to the client the reverse. The match
  is percent-decoded and ignores a drive letter's case. The rest of each URI
  keeps its own spelling, and URIs outside the root (`~/.cargo`, the
  standard library) are left alone. When the spellings agree, messages pass
  as they came. So documents' URIs and the server's own file watcher agree:
  on macOS a checkout reached through a symlink otherwise never sees a
  change on disk (V132, V133). lspmux sees only canonical URIs, so its
  per-client open files (it tracks them by URI) agree across clients too;
- takes `workspace.didChangeWatchedFiles` out of the client capabilities, and
  drops every client's own `workspace/didChangeWatchedFiles`: rust-analyzer
  then watches files itself (V122) until the stage-3 watcher client feeds it
  (brief §4.2: the proxy's watcher is authoritative);
- starts `lspmux server` if nothing answers, detached, under a lock so two
  shims never start two (D21, V117), with its log at `lspmux.log` beside the
  socket. On Windows the daemon inherits no handles (V125), so that log holds
  only the shim's start lines;
- runs `lspmux client --server-path <server>` and relays the session through
  it message by message;
- **agent role** (D16): drops the client's `textDocument/didOpen`,
  `didChange`, `didSave` and `didClose`, so the server reads the agent's files
  from disk (V120); and answers `workspace/configuration` itself with `null`
  per item, because lspmux may send it to any client and an agent refuses it,
  which would leave the server waiting (V5);
- **the readiness gate** (P3.12, `codetags_lsp::ready`): an agent's
  requests wait until the server has loaded the workspace, because an agent
  reads an answer given before then ("No symbols found") as a fact (V128,
  V141). Every shim adds `experimental.serverStatusNotification` to the
  `initialize` it sends, since lspmux initializes the server with the first
  one (V151), and rust-analyzer then reports `experimental/serverStatus`
  whenever its status changes (V150). The shim passes those notifications on
  only to a client that asked for them itself. For a server whose
  `initialize` answer names it `rust-analyzer`, an agent's requests are held
  until a status says `quiescent: true`, at most for the bound: 300 s by
  default (V152), `--ready-timeout SECONDS` or `CODETAGS_LSP_READY_TIMEOUT`
  to change it, 0 to turn the gate off. When the bound expires the held
  requests go on anyway, and stderr says so. A session that joins a server
  that has finished loading sees no status, since rust-analyzer reports only
  changes, so after 2 s without one it checks a lock file in the daemon's
  state directory (`ready-<hash>.lock`), which every shim that last saw that
  server not quiescent holds shared; if none does, the session is not held.
  A later "not quiescent" (a reload after `Cargo.toml` changes) closes the
  gate again, with a new bound. Editors are never held, and notifications
  and responses never are. `shutdown` releases whatever is held first;
- `shutdown` is answered by lspmux, which detaches only this session (V121);
  the shim then waits for `exit` or to be killed, as Claude Code does (V115).

`--log <file>` writes the session in the recorder's format, with dropped and
locally answered messages marked (`"shim": …`), so `codetags-lsp analyze`
reads it.

Decided here, for review: the full fixed-capability rewrite (R15) is not
built, since rust-analyzer sent none of the requests Claude Code refuses
(V110); only the watched-files capability is removed. The first session's
`initialize` therefore still sets rust-analyzer's capabilities for everyone.

## 3. Not built yet

- **Stage 3:** the watcher as an extra lspmux client (R40-R42); until then
  servers watch for themselves. `TODO(stage 3)` in `codetags_lsp::policy`.
- **Readiness for other servers:** the gate (P3.12) knows only
  rust-analyzer's status. And if every session leaves while the server is
  still loading, the next one to join sees neither a status nor a lock, and
  is not held (V151).
- **Configuration merge** (P3.6, R23, R34): an agent's shim answers `null`;
  `TODO(P3.6)` in `codetags_lsp::policy::from_server`.
- **Crash recovery** (P3.7, R39): if lspmux's session ends unexpectedly the
  shim exits, and the client restarts the server; `TODO(P3.7)` in
  `codetags_lsp::serve`.
- Root rules for Go, TypeScript and Python (`TODO(P3.2)` in
  `codetags_lsp::root`), and the `CODETAGS_KEY_TOOLCHAIN` and settings keys
  (R9, R35).
- **Follow-ups:** automate a real VS Code session (only a scripted VS
  Code-style client is tested); check that VS Code and Claude Code on Windows
  start the `.exe` wrappers (V119).

## 4. Tests

- `features/lsp/setup.feature`: setup and doctor, each scenario in its own
  home; `@lspmux` ones check against the real lspmux.
- `features/lsp/shim.feature`: the shim against the BDD runner's fake
  language server through the real lspmux (`@lspmux`, run by CI's `check`
  job on all three OSes after `just setup-lspmux`): sharing one server
  process, the agent's dropped document messages, the editor's forwarded
  ones, `--version`, multi-root, the daemon on demand and started once, an
  agent killed without `exit`, root normalization to the canonical root with URIs rewritten both ways (through a symlink on Linux and macOS, and the identity everywhere), `workspace/configuration`,
  the readiness gate (an agent held until quiescence or the bound, a late
  joiner to a ready or a loading server, editors never held, the status
  passed on only to a client that asked), a scripted VS Code-style session
  through a wrapper, and the recorded Claude
  Code session (`tests/fixtures/lsp`) replayed as the agent.
- `features/lsp/wiring.feature` (`@lspmux @providers`, CI's `providers` job
  on all three OSes, P3.14, D25): §1 as a person runs it, in a scratch
  checkout of the Rust fixture with this repo's wiring. `lsp-setup.sh` runs
  there; the plugin's wrapper is started as Claude Code starts it and VS
  Code's through Node as the extension does (`tests/support/
  spawn-like-vscode.mjs`, V118); both share one real rust-analyzer through
  lspmux. It also covers the fallbacks before setup and with a stage-0 shim,
  doctor's `lsp wiring:` line, and V126's on-disk changes reaching
  rust-analyzer with no client telling it, with an editor-opened document as
  the control, and the same changes through a symlinked checkout (D26).
- `features/lsp/claude-client.feature`, rule "Through the shim and lspmux"
  (`@claude @linux`, `just test-claude`): Claude Code itself through the
  plugin, the shim and lspmux to rust-analyzer.
