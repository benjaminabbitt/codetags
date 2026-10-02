# Notes for an lspmux PR: server requests that are never answered

P3.11, D29. **These are notes, not PR text.** A human writes the PR and
the issue in their own words. The maintainer has said that pasting
machine-written output is against Codeberg's terms (Codeberg #21), and that
he is "not personally interested in debugging LLM tools" (#12). So the
motivation below is editor use: gopls behind lspmux, with VS Code or
Neovim. Nothing here needs agents to make sense, and nothing should mention
them.

Everything refers to lspmux at `18861f9` (main, 2026-03-11) unless it says
otherwise. `dev` restructures the core into an actor (`49878a8`), so check
whether the PR should target `dev` before writing code; see "Before
writing" at the end.

## 1. The problem

lspmux handles a few server-to-client requests and drops the rest without
answering them. The server then waits forever for an answer.

What happens to each server-to-client request, in `stdout_task`
(`src/instance.rs`):

| Request | Handling | Where |
|---|---|---|
| `window/workDoneProgress/create`, `workspace/{codeLens,semanticTokens,inlayHint,inlineValue,diagnostic}/refresh` | broadcast to every client, tagged `Drop`; lspmux answers the server with `null` itself | `instance.rs:908-935` |
| `workspace/configuration` | sent to one arbitrary client (`one_client`, the first in a `HashMap`), tagged `Forward`; the client's answer is relayed | `instance.rs:937-950`; relay in `client.rs:391-397` |
| `client/registerCapability`, `client/unregisterCapability` | cached, broadcast, answered with `null` | `instance.rs:952-1003` |
| **everything else** | `debug!("ignoring unknown server request")`, never answered | `instance.rs:1005-1010` |

The code already says so: the last arm carries `// TODO workspace/applyEdit
request` and `// TODO workspace/workspaceFolders request`
(`instance.rs:1007-1008`), and the README says lspmux "drops any requests
from the server" (`README.md:33-39`).

Two smaller gaps in the same path, worth knowing about even if the PR
leaves them alone:

- With no client attached, `workspace/configuration` is not answered
  either (`instance.rs:947-949`).
- An error *response* from a client is logged and dropped, never relayed
  (`client.rs:406-408`). So a forwarded request the client refuses still
  leaves the server waiting. Any fix that forwards more requests has to
  relay errors too, or it moves the hang rather than removing it.

The requests that matter in practice:

- **`workspace/applyEdit`.** gopls (checked at `134264d` during our
  analysis; re-check against a current release) sends it from
  `applyChanges` in `gopls/internal/server/command.go` (around lines
  1013-1027) with no timeout and without checking the client's
  `workspace.applyEdit` capability. Commands that go through it include
  add import, remove dependency, the go.mod upgrade commands, extract to
  new file, move type, add test, change signature, implement interface,
  modify tags and move declaration. Through lspmux, the editor's
  `workspace/executeCommand` for any of these never completes; the rest of
  gopls keeps working. Quick fixes that VS Code resolves through
  `codeAction/resolve` do not use it, which may be why nobody has
  reported it. Upstream #13 ("gopls not working", unresolved) may be the
  same thing; worth reading before filing.
- **`window/showMessageRequest`.** gopls sends it for its telemetry and
  vulncheck prompts, with a 15 s timeout (`prompt.go`, around 351-375), so
  it only loses the prompt. rust-analyzer sends it only with
  `open_server_logs` set and handles the answer asynchronously. It does
  not hang anything, but the same change fixes it.
- `workspace/workspaceFolders`: no server we use sends it; it is in the
  TODO comment, so the PR may cover it in passing or leave it.

## 2. A minimal reproduction

The aim is something the maintainer can run in two minutes with an editor
he already uses. Two variants; the first is the most convincing.

**Neovim (or Helix) with gopls through lspmux:**

1. `go install golang.org/x/tools/gopls@latest`; start `lspmux server`.
2. A module with one file that uses a package it does not import:

   ```go
   package main

   func main() { fmt.Println("hi") }
   ```

3. Point the editor's gopls at `lspmux client --server-path gopls`, as
   the README shows for rust-analyzer.
4. On `fmt`, run the code action that adds the import, or call
   `:lua vim.lsp.buf.execute_command({command = "gopls.add_import", ...})`.
   Which actions go through `executeCommand` + `applyEdit` depends on the
   gopls version; pick one from the list above and confirm it on the
   version you test with.
5. Direct gopls: the import is added. Through lspmux: nothing happens; the
   `executeCommand` request stays pending, and `RUST_LOG=debug lspmux
   server` logs `ignoring unknown server request` with
   `method: "workspace/applyEdit"`.

**Scripted, for the PR's test section:** a tiny fake server that answers
`initialize`, then on any `workspace/executeCommand` sends
`workspace/applyEdit` and answers the command only after it gets the
edit's response. Run a client through lspmux and send `executeCommand`:
it never gets an answer. This shows the bug without gopls, and is what the
tests below automate.

## 3. Proposed design

Keep it to the existing pattern: `workspace/configuration` already sends a
request to one client with the `Forward` tag and relays the answer. The
change generalizes that arm to "forward the request to one suitable client
and relay its answer or its error".

**Which client.** In order:

1. Restrict to clients that can handle the method. For
   `workspace/applyEdit`, those whose `initialize` had
   `capabilities.workspace.applyEdit: true`; for
   `window/showMessageRequest`, any client (the capability only refines
   action items); for anything unknown, any client. lspmux does not keep a
   client's capabilities today: `Client` is an id and a channel
   (`client.rs:87-90`), and only the first client's `InitializeParams`
   reach the server (`instance.rs:680-700`). Keeping the
   `capabilities` value from each client's `InitializeParams`
   (`client.rs`, in `connect`, around 222-260) on `ClientData` is a small
   addition.
2. Among those, prefer the client that most recently sent a
   `workspace/executeCommand` the server has not answered yet. JSON-RPC
   has no causal link from a server request to the client request that
   triggered it, but for `applyEdit` the triggering `executeCommand` is
   almost always still in flight. Tracking it means remembering, per
   client, the ids of its pending `executeCommand` requests (set when the
   request is tagged in `client.rs:384-389`, cleared when the response
   goes back in `stdout_task`, `instance.rs:865-905`).
3. Otherwise the most recently active client, or simply the first, as
   `workspace/configuration` does now.

**Relaying the answer.** Tag the request `Forward`, as now. Relay a
success response as `client.rs:391-397` already does, and change
`client.rs:406-408` to relay an error response with a `Forward` tag the
same way, instead of logging and dropping it.

**No capable client.** Answer the server at once with an error:
`RequestFailed` (-32803) or `InternalError` (-32603) with a message such
as "lspmux: no connected client can handle workspace/applyEdit". For
`workspace/applyEdit` a success with `{"applied": false, "failureReason":
"..."}` is the spec's own way to refuse and may be kinder to servers.
Either way the server stops waiting. Apply the same to
`workspace/configuration` with no client attached (today never answered),
answering `null` per item, which the spec allows.

**The chosen client goes away.** If the client disconnects before
answering, `cleanup_client` (`instance.rs:266-285`) should answer that
client's outstanding forwarded requests with an error, so the server does
not wait for a client that no longer exists. This needs a small map of
forwarded request ids per client.

**Timeouts.** lspmux has none for client answers today. A server that
sends `showMessageRequest` with its own timeout (gopls) is fine without
one. For `applyEdit`, a person may take a while to confirm an edit, so a
timeout in lspmux is debatable; suggest leaving it out of the first PR
and relying on the disconnect cleanup above. Mention it as an open
question for the maintainer.

**Size.** Our estimate is a few dozen lines plus tests, mostly in
`stdout_task`'s last arm, `ClientData`, and the error-response arm in
`client.rs`.

## 4. Edge cases to cover or decide

- Two editors attached, only one advertises `workspace.applyEdit`: the
  edit must go to that one.
- Both advertise it, and only one has an `executeCommand` in flight: that
  one.
- Both have one in flight (two people running commands at once): the most
  recent; say in the PR that this is a heuristic.
- The chosen client answers with an error: the server gets the error
  (today it would wait forever).
- The chosen client disconnects before answering: the server gets an
  error.
- No client attached at all (only possible between the last client
  leaving and the instance timing out): immediate error, or `applied:
  false`.
- Request id collisions: the server's ids are tagged `forward:...` as
  now, so a client's own ids cannot collide; the relay strips the tag.
- `showMessageRequest` broadcast vs one client: one client only, or two
  editors would both pop up the same prompt and the server would get two
  answers.
- The edit names files another client has open with unsaved changes:
  out of scope, as lspmux already leaves document ownership to the
  clients; mention it, do not solve it.
- `dev`'s actor refactor: the same logic moves into the actor's message
  handling; the design does not depend on the locking structure.

## 5. Test plan in lspmux's style

lspmux's tests are small `#[test]` functions next to the code, mostly
`serde_json::json!` round trips (`src/lsp/jsonrpc.rs:195-253`,
`src/lsp/ext.rs:224-291`, `src/lsp.rs:270-311`, `src/config.rs:178`).
There are no async or integration tests in the tree. So:

- **Pure selection function, unit-tested.** Factor the choice into
  something like `fn pick_client(method: &str, clients: &[ClientView]) ->
  Option<usize>`, where `ClientView` holds the id, whether it advertised
  `workspace.applyEdit`, and the time of its newest pending
  `executeCommand`. Test the cases in §4 with plain `#[test]`s.
- **Capability parsing, unit-tested.** `InitializeParams` with and
  without `capabilities.workspace.applyEdit`, as the existing
  `deserialize_InitializeParams_workspace_folders` test does.
- **Tag round trip for error responses.** A `Forward`-tagged id on a
  `ResponseError` untags to the original id, like the existing
  `ext.rs` tag tests.
- **Manual check, described in the PR:** the Neovim reproduction above,
  before and after, with the `RUST_LOG=debug` line gone.
- Offer, without insisting, an integration test with a fake server
  process and `tokio::test`, if the maintainer wants one; it would be the
  first of its kind in the repo.

## 6. Before writing

- Check `dev` (`f37d5cf` or newer): if the actor refactor is about to land
  on `main`, write against `dev`, and ask in the issue first.
- Re-check the gopls file and line references against the gopls release
  you test with; ours are from `134264d`.
- Search Codeberg issues for `applyEdit` and re-read #13.
- Keep the PR to forwarding plus error relay. Leave timeouts and
  `workspaceFolders` as questions.

For codetags, until it merges: gopls commands that use `applyEdit` do not
work through the shim (D29, `docs/proxy-zero-change.md` C-1, C-2).
