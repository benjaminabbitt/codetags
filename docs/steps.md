# codetags step vocabulary

This vocabulary is a frozen interface (PLAN.md §0.3). Features under
`features/` may use only these steps. Adding or changing a step is a
`[SPEC]` task: update this file in the same commit as the step definition in
`crates/codetags-bdd`.

`{string}` is a double-quoted cucumber-expression string. `{int}` is a
decimal integer.

## Commands

```gherkin
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

The Linux FUSE spike (P0b S1) is the first backend, and the macOS NFS
loopback spike (P0b S2) the second. The mount steps that mount or unmount are
defined on Linux and macOS only; tag scenarios that use them `@mount` and with
the OS. The mount is unmounted, if still mounted, when the scenario ends.

```gherkin
Given the hello filesystem is mounted on an empty directory
      # the spike's read-only, one-file filesystem, mounted unprivileged on a
      # new directory in the scenario's scratch directory: through fusermount3
      # on Linux (S1); on macOS (S2) served by an in-process NFSv3 server on
      # 127.0.0.1 and mounted with /sbin/mount_nfs, with actimeo=0
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
