# codetags step vocabulary

This vocabulary is a frozen interface (PLAN.md §0.3). Features under
`features/` may use only these steps. Adding or changing a step is a
`[SPEC]` task: update this file in the same commit as the step definition in
`crates/codetags-bdd`.

`{string}` is a double-quoted cucumber-expression string. `{int}` is a
decimal integer.

## Commands

```gherkin
Given the environment variable {string} is {string}
      # set for processes that `codetags is run with` starts in this scenario
When codetags is run with {string}      # args split on whitespace; captures exit status, stdout, stderr
Then it exits with status {int}
Then stdout matches {string}            # Rust `regex` syntax, unanchored unless the pattern anchors itself
```

## DuckDB model layer

"Another process" steps re-run the test executable as a child process. That
proves cross-process behaviour, not just behaviour between connections in one
process.

```gherkin
Given a DuckDB database with a table {string} holding the rows {string}
      # table name: [a-z_]+; rows: whitespace-separated values in one TEXT column `v`;
      # the writer closes the file before the step ends
When another process opens it read-only and lists table {string}
When another process opens it read-only and inserts {string} into table {string}
Then that process reads exactly {string}         # space-joined `v` values, sorted
Then that process's write is refused
```

## tagma queries

The wording matches tagma's own conformance steps (tagma `docs/steps.md`).

```gherkin
Given an item {string} tagged {string}       # id; whitespace-separated tags, parsed with tagma's Tag::parse
When the postfix query {string} is run
Then it matches exactly {string}             # whitespace-separated ids, compared as sorted sets; "" = none
Then the query fails
```

## Live mounts

The Linux FUSE spike (P0b S1) is the first backend, the macOS NFS loopback
spike (P0b S2) the second, and the Windows WinFsp spike (P0b S3) the third.
The mount steps that mount or unmount are defined on Linux, macOS and Windows;
tag scenarios that use them `@mount` and with the OS, and on Windows also
`@winfsp`. The mount is unmounted, if still mounted, when the scenario ends.

On Windows the mount point is a directory that does not exist yet: WinFsp
creates it on mount and removes it on unmount, so there "the mount directory
is empty again" also holds when the directory is gone.

```gherkin
Given the hello filesystem is mounted on an empty directory
      # the spike's read-only, one-file filesystem, mounted unprivileged on a
      # new directory in the scenario's scratch directory: through fusermount3
      # on Linux (S1); on macOS (S2) served by an in-process NFSv3 server on
      # 127.0.0.1 and mounted with /sbin/mount_nfs, with actimeo=0; on
      # Windows (S3) through WinFsp
Given a fusermount3 that is not setuid root comes first on PATH
      # Unix only: puts a fake, non-setuid fusermount3 first on PATH and
      # unsets FUSERMOUNT_PATH, for processes that `codetags is run with` starts
When the mount is unmounted
Then the mount lists exactly {string}                # whitespace-separated names, compared as sorted lists; "" = empty
Then reading {string} through the mount gives {string}   # file name in the mount; content, trailing whitespace trimmed
Then the mount directory is empty again
```

macOS only (S2). These steps measure the NFS client's caches: the server
cannot push an invalidation, so a change shows only when the client asks
again. "Missing" means `stat` of the name in the mount's root fails with
`ENOENT`.

```gherkin
Given the hello filesystem is mounted on an empty directory with the extra mount option {string}
      # as above, with one more mount_nfs `-o` option, e.g. "nonegnamecache"
Given {string} is missing from the mount
      # a lookup through the mount, which the client may cache as a negative entry
When the filesystem gains {string} holding {string}
      # the server adds a file to the root in process; the root's mtime is unchanged
When the filesystem gains {string} holding {string} and bumps the directory's mtime
      # as above, and the root's mtime becomes the current time
Then {string} is still missing from the mount after {int} seconds
      # looks the name up every 100 ms for that long; fails if it ever appears
Then {string} appears in the mount within {int} second(s)
      # looks the name up every 50 ms; prints how long it took on stderr
```

## Generation store

The index directory is a fresh `.codetags/index/` per scenario. The files a
generation "records" are the paths in its `file` table. "Another process" and
"a writer process" run in a child process, as in the DuckDB steps above, so
`Then that process reads exactly {string}` applies to them too.

```gherkin
Given an index with one complete generation per file in {string}
      # whitespace-separated paths; generation k records only the k-th path
Given an index whose newest complete generation has schema version {int}
Given a reader has opened the newest complete generation   # held by this process
Given a writer has begun a generation                      # held by this process, with write.lock
Given a writer process exits before completing a generation recording the files {string}
      # the child dies mid-write: no close, no completion marker
When another process writes a generation recording the files {string}
      # whitespace-separated paths; the child prints the generation number it completed
When another process opens the newest complete generation and lists its files
      # the child prints the sorted, space-joined paths
When the reader switches to the newest complete generation
When the index is garbage-collected
Then that process completes generation {int}
Then that process fails with an error matching {string}    # Rust `regex` syntax, against stderr
Then the reader reads the files {string}                   # space-joined paths, sorted
Then the generation the reader held before the switch reads the files {string}
Then the index holds only the generations {string}
      # space-joined numbers, ascending: exactly these are complete, and no
      # other generation has any file left in the index directory
```

## Names

`{word}` in the item-id steps is `file` or `symbol`. A `{string}` may be
single-quoted to hold `"`. In the Windows-name steps, `{U+XXXX}` inside the
expected string stands for that one code point.

```gherkin
When the canonical name of the SCIP symbol {string} is taken
Then the canonical name is {string}
Then the symbol has no canonical name                # a local, a parameter, or a Rust impl block
Given these SCIP symbols:                            # table with column: symbol
When their canonical names are assigned
Then the assigned names are:                         # table with columns: symbol | name; exact set, after the +field policy
Then the collisions are:                             # table with columns: name | symbol; exact set
Then there are no collisions

When the name {string} is mapped for Windows         # the private-use mapping, D15
Then the Windows name is {string}
Then the Windows name maps back to {string}

When the item id for {word} {string} is written     # file → file:<path>, symbol → sym:<name>
Then the written id is {string}
Then the written id reads back as {word} {string}

Given a directory holding these entries:            # table with columns: name | id
When collision suffixes are applied
Then the entries are named:                         # table with columns: id | name
Then applying them to the entries in reverse order gives the same names
```

## SCIP providers

Index features are tagged `@providers` and run in `just test-providers`.
Fixtures are `tests/fixtures/<name>`. A provider writes its index and any
build output (`CARGO_TARGET_DIR`) under the scenario's scratch directory,
never into the fixture. A fixture's index is deterministic, so one process
indexes each fixture once and its scenarios share the result.

A symbol `{string}` is named by its SCIP descriptors: the symbol without its
scheme, manager, package and version, e.g. `charge/double().` or
``charge/impl#[`Charge<T>`][Apply]apply().``. Lines are 1-based.

```gherkin
When the Rust provider indexes the fixture {string}          # `rust-analyzer scip` on PATH
When the Rust provider indexes a directory whose Cargo.toml is {string}   # a fresh directory holding only that manifest
When the Rust provider indexes a directory that does not exist
Then the provider run succeeds
Then the provider run fails with stderr matching {string}
      # the error carries the provider's stderr; Rust `regex` syntax, against that stderr
Then the index documents are exactly {string}                # whitespace-separated relative paths, compared as sorted sets
Then every definition of a function or method has an enclosing range
      # symbols of kind Function, Method or TraitMethod; fails if there are none
Then the definition of {string} encloses lines {int} to {int}
Then {string} occurs in {string} on line {int}                # a non-definition occurrence of the symbol in that document
Then no symbol in the index has a relationship
Then the document {string} has occurrences of local symbols  # `local N`
Then filtering local symbols keeps every other occurrence in {string}
Then every document has a reference within a definition's enclosing range
      # a non-local, non-definition occurrence inside some definition's enclosing range (brief §4.4: zero edges is a failure)
Then the reference to {string} on line {int} of {string} lies within the definition of {string}
      # the innermost definition in that document whose enclosing range contains the reference
Then the symbol {string} implements {string}                 # an `is_implementation` relationship
Then the definition of {string} has the canonical name {string}   # codetags-names (D15)
```

The Go provider runs `scip-go` and `gocallgraph` (`tools/gocallgraph`) from
`PATH`, over the packages `./...` unless a step names a pattern. A call-graph
edge's site is where the called name starts; columns are 1-based bytes. Caller
and callee are named as go/ssa names them, e.g.
`(example.com/shop/internal/pay.Card).Charge`, and a function literal is
`<enclosing>$<n>`.

```gherkin
When the Go provider indexes the fixture {string}
When the Go provider indexes the packages {string} of the fixture {string}   # one package pattern, relative to the fixture
When the Go provider indexes a module whose main.go is:     # docstring; a fresh module holding a go.mod and that main.go
When the Go provider indexes a directory that does not exist
Then the call-graph edges on line {int} of {string} are:
      # table with columns: caller | callee | column | kind | algorithm; exactly the edges whose site is on that line
Then every call site in the call graph, except a function literal's, starts an occurrence in the index
      # a site whose source text starts with `func` is a function literal's
```

TypeScript (P1.6): scip-typescript and Jelly. *That project* is the
scenario's scratch project from the last `Given`. The run steps above (`the
provider run succeeds`, `... fails with stderr matching`) check whichever tool
ran last. scip-typescript and Jelly log their errors to stdout, so for them
the error's stderr section holds both streams. A *location* `{string}` is
`<path>:<line>`, the path relative to the root with `/` separators; a
function's location is the line it starts on. Location lists are
whitespace-separated and compared as sorted sets; `""` is the empty set.

```gherkin
Given a TypeScript project whose {string} is {string}
      # the fixture "ts"'s tsconfig.json and package.json, plus that file with that one-line content
Given a TypeScript project whose {string} declares a function in {int} MB
      # as above; the file declares one function and is padded with comment lines to that size
Given a directory without a tsconfig.json whose {string} is {string}
When the TypeScript provider indexes the fixture {string}    # `scip-typescript` on PATH
When the TypeScript provider indexes that project
When the TypeScript provider indexes that project with a file-size limit of {string}
      # scip-typescript's --max-file-byte-size, e.g. "1mb", instead of the runner's raised default
When Jelly analyzes the fixture {string}                     # `jelly` on PATH
When Jelly analyzes that project
When Jelly analyzes a directory that does not exist
Then the definition of {string} has no enclosing range
Then the definitions of {string} enclose lines {string}
      # every definition occurrence, in index order, as whitespace-separated `<first>-<last>` line ranges
Then {string} implements {string}                            # an is_implementation relationship
Then no global symbol occurs in {string} on line {int}       # only `local N` occurrences, if any
Then the call graph files are exactly {string}               # whitespace-separated relative paths, sorted sets
Then the calls on line {int} of {string} reach exactly {string}
      # the functions (not modules) called from every call site starting on that line
Then the call graph has an edge from {string} to {string} at {string}
      # caller, callee and call-site locations
Then line {int} of {string} loads the module {string}        # an import or require edge to that file's module
```

The Python provider runs `scip-python` and a Python interpreter (`python3`,
or `python` on Windows) from `PATH`. A project's root holds a
`pyproject.toml` naming the project. An unresolved reference is a call or
attribute name (every `x.name` read, and every bare name that is called or
is a decorator) at which scip-python wrote no occurrence (`no-occurrence`), or
only a `local N` the document never defines (`unnamed-local`). Columns count
UTF-16 code units, as scip-python does.

```gherkin
When the Python provider indexes the fixture {string}
When the Python provider indexes a project whose main.py is:   # docstring; a fresh project holding a pyproject.toml and that main.py
When the Python provider indexes a directory that does not exist
Then every definition of a method descriptor has an enclosing range
      # every non-local definition whose symbol ends in `().`; fails if there are none
Then the symbol {string} has symbol information        # in some document's symbols or the external symbols
Then the symbol {string} has no symbol information     # nowhere, though the symbol occurs in the index
Then the unresolved references are:
      # table with columns: file | line | column | name | called (yes, no) | reason; exactly the unresolved references
```

## Watcher and coalescer

A *changes* `{string}` is a whitespace-separated list of `<kind>:<path>`
tokens, where `<kind>` is `created`, `changed` or `deleted` and `<path>` is
project-relative with `/` separators; it is compared as a set, and `""` means
none. `{word}` in the coalescer steps is one of those kinds.

The coalescer steps drive the coalescer directly with a fake clock; `at {int}
ms` is milliseconds after the scenario's start, and must not go backwards.

```gherkin
Given a coalescer with a quiet window of {int} ms and a maximum wait of {int} ms
      # the default index-lock cap
Given a coalescer with a quiet window of {int} ms, a maximum wait of {int} ms and an index-lock cap of {int} ms
When at {int} ms the watcher sees {string} {word}
When at {int} ms git takes its index lock
When at {int} ms git releases its index lock
When at {int} ms a rescan finds {string}       # changes; replaces the pending events
Then at {int} ms the coalescer flushes exactly {string}   # changes
      # polls the coalescer; fails if it flushes nothing (unless "") or anything else
Then at {int} ms the coalescer flushes nothing
Then the flushed batch is marked as a rescan
```

The watcher steps run a real watcher, with the default coalescer settings,
over a project directory in a fresh scratch directory. Files are written with
the content `<path> <n>`, where `n` counts writes in the scenario, so every
write changes the size or the content.

```gherkin
Given a project directory holding the files {string}
      # whitespace-separated project-relative paths, created with their parents
Given a project directory holding {int} files in {string}
      # <dir>/f001.rs, <dir>/f002.rs, …
Given the project is being watched           # Watcher::start on the project directory
When the file {string} is written            # created, or truncated and rewritten in place; parents created
When the file {string} is replaced the way sed -i does it
      # writes a temporary file in the same directory, then renames it over the target
When a formatter rewrites every file in {string}   # in place, in name order, back to back
When the file {string} is deleted
When the directory {string} is created holding the files {string}
      # creates the directory and its parents, then writes the files (names relative to it) at once
When the directory {string} is deleted       # recursively
When git takes its index lock                # creates .git/index.lock
When git releases its index lock             # renames .git/index.lock to .git/index
When the watcher is told its events overflowed     # Watcher::rescan, as notify's Flag::Rescan does
Then within {int} seconds the watcher reports exactly {string}   # changes
      # merges the batches received since the last assertion with the coalescer's rule;
      # passes once they equal the changes and stay so for one more second
Then within {int} seconds the watcher reports {int} changed files in {string} and nothing else
Then the watcher reports nothing for {int} seconds
Then a batch the watcher reported was marked as a rescan
      # any batch since the watch started
Then the watcher watches with {string}       # Watcher::mode's source: "notify", or the helper's backend ("fanotify")
```

## Privileged helper

The helper steps run `codetags-privhelper` from the same target directory as
the `codetags` binary (`just test-privileged` builds it). Steps that need
root run `sudo -n`, so they work only where sudo needs no password: CI's
`privileged-linux` job. The scenario itself, its watcher and its helper
clients run as the unprivileged user. Anything a step creates as root is
removed with `sudo -n` when the scenario ends, and the helper stops then.

```gherkin
Given no privileged helper is listening
      # the watcher steps then try the helper at a socket path in the scratch
      # directory that does not exist
Given the privileged helper is running
      # sudo -n codetags-privhelper --socket <new scratch dir>/privhelper.sock;
      # waits for the socket; the watcher steps then use that socket
Given the directory {string} in the project is readable only by root
      # sudo -n: created owned by root, mode 0700
Given a helper client is subscribed to the project
      # connects to the running helper, subscribes to the project directory,
      # and collects the events it sends
When a helper client subscribes to {string} in the project   # a fresh client; the reply is kept
When another process writes the file {string}
      # a child `sh` writes it, as the unprivileged user; its PID is kept
When root writes the file {string}            # sudo -n sh, writing the project-relative path
Then within {int} seconds the helper reports a write to {string} by that process
      # an event on that path whose writer PID is the last "another process"
Then the helper reported nothing under {string}
      # no event so far on a path strictly inside that project-relative directory
Then the helper refuses the subscription
```

### Ingest into a generation (P1.3)

The fixture's provider run (shared, as above) is ingested into a new
generation of the scenario's index, the same `.codetags/index/` the
generation-store steps use, so their steps apply to it. "The generation" is
the newest complete one. Symbols and call-site callers and targets are named
by canonical name (D15); a `descriptors` cell is the SCIP symbol's
descriptors, as above. Booleans are `yes` or `no`; lines are `<first>-<last>`;
an empty cell is NULL. Tables compare as sorted multisets.

```gherkin
Given the fixture {string} has been ingested into a new generation
When the fixture {string} is ingested into a new generation
Then the generation's symbols include:
      # table: name | descriptors | kind | file | lines | module | external
      # (any subset of columns after name); each name names exactly one symbol
Then the symbols defined in {string} are exactly:      # table: name; symbols whose file is that path
Then the ingest reports no canonical-name collisions
Then the call sites in {string} on line {int} are:
      # table: caller | target | kind | dispatch | external (the target's)
Then there are no call sites in {string} on line {int}
Then no symbol or call site in the generation comes from a local symbol
Then every call site has exactly one call target, its declared target, with the method {string}
Then every call site's source matches {string}               # Rust `regex` syntax
Then the generation holds one succeeded run of {string}      # the run's provider
Then the run's edge counts are:                              # table: file | edges; all of run_file
Then the ingest report counts:
      # table: what | count; what is one of: files, symbols, call sites,
      # local occurrences, operator references, non-callable references,
      # references outside a definition
```

`codetags index` end to end. The scenario's directory is its scratch
directory, whose `.codetags/index/` the generation-store steps read. Processes
started there get `CARGO_TARGET_DIR` inside it, so a provider writes nothing
into the copied project.

```gherkin
Given the scenario's directory holds a copy of the fixture {string}
When codetags is run in the scenario's directory with {string}
      # as `codetags is run with`, with the scenario's directory as the working directory
Then the file {string} in the scenario's directory holds the lines {string}
      # whitespace-separated; blank lines and `#` comments are ignored
```

### Resolution report and edge-count regression (P1.8)

`codetags report` run in the scenario's directory reads the index the ingest
steps write. A generation's edge counts are its `run_file` rows.

```gherkin
Given the edge count of {string} in generation {int} is changed to {int}
      # rewrites that file's `run_file` rows in that complete generation, as a
      # provider that silently lost (or gained) edges would have written them
Given the file {string} in the scenario's directory holds:   # docstring, written as is
Then the JSON resolution report counts:
      # parses the last process's stdout (`report --json`); table: scope | language |
      # package | module | call sites | resolved | unresolved, where scope is module,
      # language or total; an empty cell is null; compared with every count in the report as a set
Then the file {string} in the scenario's directory holds the JSON:
      # docstring; compared as parsed JSON values
```

## LSP recorder (M3 stage 0)

`codetags-lsp record` (PLAN.md D18) between a scripted client and a fake
language server: the BDD runner's own executable in child mode `fake-lsp`.
The fake server prints `fake-lsp 9.9.9` for `--version`. Otherwise it
answers `initialize` with a response of the same id (its headers in the
order `Content-Type`, `Content-Length`), then sends a `window/logMessage`
notification and a `client/registerCapability` request with id 900; it
answers `shutdown` and every other request with a `null` result, and stops
at `exit` or end of input. A `fake/askConfiguration` request is answered
with the result of a `workspace/configuration` request (one item) that it
sends the client first. It always exits with the status its `Given` step
names. Under the shim (below) it also appends a JSON line to a log as it
starts and for each message it receives, which the shim's `Then` steps read.

```gherkin
Given a fake language server                          # exits with status 0
Given a fake language server that exits with status {int}
When a scripted LSP session runs through codetags-lsp record
      # the client writes initialize (id 1, with a Content-Type header and
      # non-ASCII text), initialized, a response with id 900, shutdown (id 2)
      # and exit, closes its end, and captures stdout and the exit status;
      # bodies are spaced as no serializer would write them
When codetags-lsp record runs the fake server with the arguments {string}
      # args after `--`, split on whitespace; stdin is empty; captures exit
      # status, stdout, stderr
Then the fake server received exactly the bytes the client sent
Then the client received exactly the bytes the fake server sent
Then the recording holds {int} messages               # log lines with a `from` field
Then the recording shows {string} sent by the {word}
      # exactly one message from `client` or `server`: a method name, or
      # "response <id>" for a response with that JSON id
Then no recording was written                         # the --log file does not exist
```

`it exits with status {int}` and `stdout matches {string}` (Commands) apply
to the recorder's run.

## Claude Code's LSP client (M3 stage 0)

`features/lsp/claude-client.feature` (`@claude`) drives Claude Code headless
(`claude -p --model claude-haiku-4-5-20251001`, PLAN.md D16-D18) with the
recorder plugin (`tools/claude-plugins/codetags-lsp-recorder`) loaded through
`--plugin-dir`; docs/lsp-stage0.md explains the flags. Each `Claude Code …`
step is one user turn of one session (`--input-format stream-json`), and
records a step mark named in the comment, for `codetags-lsp analyze --marks`.
A step fails if Claude did not use the tools it names. The session ends,
and its recording is analyzed, after the scenario's last action.

Within one test run, scenarios whose `Given` and `When` steps (Background
included) are the same share one live session: the first runs it, the rest
replay its outcome, or its failure. The runner's `before` hook gives each
scenario its plan. Tag such features `@serial`, so no two scenarios start a
session at once.

Recordings are sanitized before they are analyzed: the project directory
becomes `/project`, and `$HOME` becomes `/home/user`. The `Then` steps read
the analysis, so they apply alike to a live session and to a recorded one.

```gherkin
Given a scratch Rust project from the fixture {string} with the LSP recorder plugin
      # copies tests/fixtures/<name> into the scratch directory as a git repo
      # (one commit), and gives it its own recorder setup in .codetags/local:
      # a copy of codetags-lsp and this toolchain's rust-analyzer path
When Claude Code reads {string} with the Read tool                     # mark: Read
When Claude Code asks the LSP for the definition of {string} in {string}   # symbol, file; mark: definition
When Claude Code asks the LSP for references to {string} in {string}       # mark: references
When Claude Code asks the LSP to hover on {string} in {string}             # mark: hover
When Claude Code asks the LSP for implementations of {string} in {string}  # mark: implementations
When Claude Code asks the LSP for the call hierarchy of {string} in {string}
      # prepareCallHierarchy, incomingCalls, outgoingCalls; mark: call hierarchy
When Claude Code appends {string} to {string} with the Edit tool, then hovers on {string}   # mark: Edit
When Claude Code creates {string} holding {string} with the Write tool, then lists its symbols
      # documentSymbol; mark: Write
When Claude Code runs {string} through Bash, then hovers on {string} in {string}
      # the command is allowed exactly (split on `&&`); mark: its program,
      # plus the subcommand for git ("sed", "git checkout")
When codetags-lsp analyzes the recorded session {string}
      # tests/fixtures/lsp/<name>.jsonl, with <name>.marks.jsonl if present
Then Claude Code loaded the plugin {string}           # from the stream's system/init event
Then Claude Code did not load the plugin {string}
Then the recorder wrote {int} recording(s)            # one per language-server process
Then the client's initialize rootUri is the project root    # "file:///project"
Then the client's initialize rootUri is null
Then the client's initialize workspaceFolders are only the project root
Then the client capability {string} is absent         # dotted path in initialize's capabilities
Then the client capability {string} is {string}       # expected value as JSON, compared parsed
Then the client answered {string} with error {int}    # every answer to that server request
Then the client answered {string} with the result {string}   # every answer, JSON compared parsed
Then the server sent no {string} request
Then after the {string} the client's document notifications were {string}
      # the step mark's didOpen/didChange/didSave/didClose/didChangeWatchedFiles,
      # in order, as "didOpen src/a.rs, didSave src/a.rs"; "none" if there were none
Then after the {string} the client's first request was {string}   # "none" if there were none
Then the client's last request was {string}
Then the client sent no {string}                      # a notification method, at any time
Then every didChange the client sent carried the full text   # and it sent at least one
Then the recording has no end-of-stream or exit record
      # the recorder was killed before either side closed its stream
```

## lspmux setup and doctor (M3 stages 1 and 2)

`codetags lsp setup` and the proxy checks of `codetags doctor`
(`features/lsp/setup.feature`, PLAN.md D20). Every scenario has its own home:
`HOME`, `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `XDG_DATA_HOME`, `USERPROFILE`,
`APPDATA` and `LOCALAPPDATA` point into the scratch directory, and on Linux
and macOS `XDG_RUNTIME_DIR` into a short private temporary directory (a
socket path is at most 103 bytes on macOS), for every process the scenario
starts. `CODETAGS_LSPMUX` names the lspmux binary when one is found
(`$CODETAGS_LSPMUX`, this checkout's `.codetags/local/bin/lspmux`, or PATH).
lspmux's config file is the one `ProjectDirs("", "", "lspmux")` names in that
home (V116).

```gherkin
Given an isolated home for lspmux
      # fails if `codetags lsp setup`, unconfirmed, names a config file
      # outside the home: the scenario would touch the real one
Given codetags lsp setup has run                 # `codetags lsp setup --yes`; must exit 0
Given lspmux is set up in an isolated home
      # both of the above; on Windows with --listen 127.0.0.1:<a free port>
When codetags is run with {string} and the answer {string}
      # like `codetags is run with`, with the answer and a newline on stdin
Given lspmux's config file holds {string}        # the text plus a newline
Given lspmux's config file has {string} replaced by {string}
Then lspmux's config sets {string} to {string}   # key; the value as TOML, compared parsed
Then lspmux's config listens on a socket in a private directory
      # listen = connect = <XDG_RUNTIME_DIR>/codetags/lspmux.sock, in a 0700 directory
Then a backup of lspmux's config holds {string}  # exactly one config.toml.bak-*, trailing newline ignored
Then no backup of lspmux's config exists
Then lspmux's config file still holds {string}   # trailing newline ignored
Then lspmux's config file does not exist
When lspmux prints its effective config          # `lspmux config` in the home; captures its output
Then lspmux's effective config listens where codetags lsp setup wrote
      # the printed config's `listen` is the file's
Given no lspmux daemon is answering
Then no lspmux daemon is answering
Then an lspmux daemon is answering               # the home's socket (Windows: its port) accepts
Then the lspmux daemon was started {int} time(s) # start lines in the daemon's log
```

## The LSP shim (M3 stages 1 and 2)

`codetags-lsp serve` (and wrappers that run it) between scripted sessions and
the fake language server above, through a real lspmux
(`features/lsp/shim.feature`; PLAN.md D14-D22). The sessions need an isolated
home with lspmux set up (above). The shim runs with `--server <the fake
server> --lspmux <lspmux>` in the scratch project (`project`, holding
`src/lib.rs`), and the fake server logs to the scenario's own file. A session
reads the shim's output on its own thread and waits up to 60 s for each
answer. Sessions are killed, and the daemons the shim started (named in the
daemon's log) with their servers, when the scenario ends.

```gherkin
Given the project is a Cargo workspace with the member crate {string}
      # Cargo.toml with [workspace] members = [<crate>], and the crate's own
      # Cargo.toml; sessions then pass --root cargo
When codetags-lsp serve runs the fake server as the {string} with the arguments {string}
      # role; args after `--`, split on whitespace; stdin empty; captures
      # exit status, stdout, stderr
When an {string} session {string} starts through codetags-lsp serve
      # role, name: initialize (id 1) naming the project as rootUri, rootPath
      # and only workspace folder, with empty capabilities; waits for its
      # answer, which must be a result; then initialized
When an {string} session {string} starts through codetags-lsp serve in {string}
      # the same, for a directory under the project
When the {string} sessions {string} and {string} start at once through codetags-lsp serve
      # both shims start and send initialize before either is answered
When an {string} session {string} sends an initialize with {int} workspace folders
      # waits for the answer, closes the input, waits for the shim to exit
When session {string} sends {string} for {string}
      # a notification for a project file: textDocument/didOpen, didChange,
      # didSave or didClose, or workspace/didChangeWatchedFiles
When session {string} asks {string} for {string}
      # a request (textDocument and position params) for a project file;
      # waits for its answer
When session {string} shuts down and is killed without exit
      # shutdown, its answer, then a kill, as Claude Code does (V115)
Given a wrapper {string} that runs the fake server as the {string}
      # `codetags-lsp install-wrapper` into the scratch directory
When the wrapper {string} runs with the arguments {string}
      # started by its path without an extension, as an editor would (V119);
      # stdin empty; captures exit status, stdout, stderr
When a VS Code-style session {string} runs through the wrapper {string}
      # initialize with VS Code-like capabilities (configuration,
      # workDoneProgress, watched-files dynamic registration), initialized,
      # didOpen, didChange, didChangeWatchedFiles, a hover request, shutdown,
      # exit; waits for the wrapper to exit
When the recorded Claude Code session {string} replays through codetags-lsp serve as the {string}
      # the client messages of tests/fixtures/lsp/<name>.jsonl, in order, with
      # /project respelled as the scratch project; waits for the answer to
      # its shutdown, then kills the shim
Then session {string} got the error {int} for its initialize
Then session {string}'s answer to {string} was {string}   # the result, JSON compared parsed
Then session {string} exited with status {int}
Then the fake server was started {int} time(s)   # fake-server processes, from its log
Then the fake server received {string} {int} time(s)
Then the fake server received no {string}
Then the fake server received no document notifications   # no didOpen, didChange, didSave or didClose
Then the fake server's initialize named the project root
      # rootUri and workspaceFolders[0].uri are the project's URI
Then the fake server's initialize did not advertise {string}   # a dotted capability path
```

`it exits with status {int}` and `stdout matches {string}` (Commands) apply
to `codetags-lsp serve` and wrapper runs.

## Scenario tags

Tags gate where a scenario runs (PLAN.md §3). They are read from the feature,
the rule, and the scenario.

| Tag | Scenario runs only… |
|---|---|
| `@linux`, `@macos`, `@windows` | on the named OSes; with no platform tag, on every OS |
| `@mount`, `@winfsp`, `@providers`, `@privileged`, `@slow`, `@claude`, `@lspmux` | when the job lists that capability in `CODETAGS_BDD_CAPABILITIES` (comma-separated); `@lspmux` needs the pinned lspmux (`just setup-lspmux`) |
| `@serial` | (no gate) alone: cucumber runs no other scenario at the same time |

Each scenario that doesn't run is reported on stderr as `bdd: not run here: …`.
Skipped *steps* (steps with no definition) fail the run.

When `CODETAGS_BDD_RAN` names a file, the runner appends one line for each
scenario it runs: `<feature file path relative to features/> :: <scenario
name>`, with `/` separators. CI collects these from every job, and
`ci/bdd-coverage.py` fails if a scenario ran in no job and is not listed in
`ci/expected-skips.txt` (PLAN.md §0.11). Scenario names must therefore be
unique within a feature file.
