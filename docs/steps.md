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
Then the symbol has no canonical name                # a local or a parameter
Given these SCIP symbols:                            # table with column: symbol
When their canonical names are assigned
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

## Scenario tags

Tags gate where a scenario runs (PLAN.md §3). They are read from the feature,
the rule, and the scenario.

| Tag | Scenario runs only… |
|---|---|
| `@linux`, `@macos`, `@windows` | on the named OSes; with no platform tag, on every OS |
| `@mount`, `@winfsp`, `@providers`, `@privileged`, `@slow` | when the job lists that capability in `CODETAGS_BDD_CAPABILITIES` (comma-separated) |

Each scenario that doesn't run is reported on stderr as `bdd: not run here: …`.
Skipped *steps* (steps with no definition) fail the run.

When `CODETAGS_BDD_RAN` names a file, the runner appends one line for each
scenario it runs: `<feature file path relative to features/> :: <scenario
name>`, with `/` separators. CI collects these from every job, and
`ci/bdd-coverage.py` fails if a scenario ran in no job and is not listed in
`ci/expected-skips.txt` (PLAN.md §0.11). Scenario names must therefore be
unique within a feature file.
