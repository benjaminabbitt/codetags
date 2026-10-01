# Proposed steps for the P2.1 and P7.1 drafts

**Status:** DRAFT for human review (2026-09-30). These steps are used by
`docs/spec-drafts/views/` and `docs/spec-drafts/plugins/`. When the drafts are
approved they move into `docs/steps.md`, in the same commit as their
definitions in `crates/codetags-bdd`, and the features move into `features/`.

The drafts also reuse these existing steps from `docs/steps.md`:

```gherkin
Then it exits with status {int}
Then stdout matches {string}
```

## The backend under test

`features/views/` is the single view contract (PLAN.md §2.5), and
`features/plugins/` reuses it. Every scenario in both directories runs once
for each **view backend** the job enables:

| Backend | What serves the tree | Has `q/` | Enabled by |
|---|---|---|---|
| `walker` | the in-memory `ViewFs` walker (P2.2), called directly | yes | `just bdd` on every OS |
| `static` | the Tier 0 materializer (P2.5), read through `std::fs` | no | `just bdd` on every OS |
| `fuse`, `nfs`, `winfsp` | a live mount (P5.1 to P5.3), read through `std::fs` | yes | `just test-mount` on its OS |

The runner reads the list from `CODETAGS_VIEW_BACKENDS` (comma-separated;
`just bdd` sets `walker,static`, `just test-mount` sets the live backend).
Steps never name a backend. Two new scenario tags restrict where a scenario
runs:

| Tag | Scenario runs only on… |
|---|---|
| `@query-tree` | backends that have `q/`: `walker` and every live mount |
| `@static-tree` | `static` |
| `@case-insensitive` | `walker` and `static`, each told to name entries for a case-insensitive target |
| `@case-sensitive` | `walker`, and any other backend whose target directory is case-sensitive (detected by probing it) |
| `@pending-D11` | nowhere yet: D11 is a draft (PLAN.md §2.10). Listed in `ci/expected-skips.txt` until D11 is approved |
| `@mermaid-cli` | jobs whose `CODETAGS_BDD_CAPABILITIES` include `mermaid-cli` (the Mermaid CLI is on PATH, V23) |

A backend that a scenario's tags exclude reports `bdd: not run here: …` as
usual. The `CODETAGS_BDD_RAN` record stays one line per scenario run, so a
scenario counts as covered once it has run on any backend.

## Paths

A **view path** is relative to the repo's view root, `<views root>/<repo>/`;
`""` is the root itself. View paths are written with their real characters
and raw tagma syntax (D15), single-quoted when they hold `"`:
`'q/calls="billing.Charge<T>.apply"'`. The step encodes them for the backend:
on Windows through the private-use mapping (PLAN.md §2.7). A trailing `/` is
allowed and ignored.

A **project path** is relative to the fixture's project directory, the
directory that holds `.codetags/`.

**Views are built lazily.** `Given` steps only prepare the project. The first
step that touches a view path builds the views (or mounts them) from the
project's newest complete generation and its `.codetags/` directory, so a
scenario may add plugins or config after loading a fixture.

## Fixtures

```gherkin
Given the view fixture {string}
      # copies tests/fixtures/views/<name>/ into a fresh project directory:
      # generation.sql becomes generation 1 (schema created, the file run,
      # the generation completed); tags becomes .codetags/tags; config.toml,
      # if present, becomes .codetags/config.toml. The repo name is the
      # fixture name.
Given a view fixture with only these files:
      # table with columns: path | language. Repo "shop", generation 1 at
      # commit a1b2c3d, indexed 2026-09-30T12:00:00Z, holding those files
      # and nothing else (no symbols, no call sites, no tags)
Given a view fixture whose module calls are:
      # table with columns: from | to | calls. Repo "shop", generation 1 at
      # commit a1b2c3d, indexed 2026-09-30T12:00:00Z. Each module m gets
      # one file src/<m with "." as "/">.rs holding one function m.f; each
      # row adds <calls> static, declared call sites from from.f to to.f
Given the project file {string} contains:
      # doc string; writes the file at that project path, creating parent
      # directories, with one trailing newline
Given the views target is case-insensitive
      # only in @case-insensitive scenarios: the walker and the materializer
      # name entries as for an APFS or NTFS default target (PLAN.md §2.7)
```

### The `shop` fixture

`docs/spec-drafts/fixtures/shop/` (on approval `tests/fixtures/views/shop/`):

- `generation.sql`: six Rust files in four modules (`api`, `billing`,
  `notify`, `ordering`), eleven symbols and seven call sites, one of them
  unresolved. Its header comment tabulates them.
- `tags`: user tags on three files, and one orphan (`src/legacy/old.rs`, not
  in the generation).

Every view of generation 1 carries the header text
`VIEW of <source> @ a1b2c3d, indexed 2026-09-30T12:00:00Z; read-only, edit the source file`.

**Facets the drafts assume** P2.4 derives (brief §4.5 ingest sketch). P2.4
owns the full list in `docs/facets.md`; the drafts depend only on these:

| Item | Id | Facets |
|---|---|---|
| file | `file:<path>` | `kind=file`, `lang`, `module` (of its symbols), `fs:dir` (each ancestor), `fs:name`, plus user tags |
| symbol | `sym:<canonical>` | `kind`, `lang`, `module`, `file=<path>`, `calls=<canonical>` (one per distinct target, dispatch expansions included) |
| call site | `site:<n>` | `kind=callsite`, `caller`, `target` (one per target), `line`, `dispatch`, `file`, `module` (the caller's), `prov:source=scip`, `prov:method` |

## Reading the views

```gherkin
Then listing {string} gives exactly {string}
      # view path; whitespace-separated names, directories with a trailing
      # "/", compared as sets; "" = an empty directory
Then {string} is a directory
Then {string} is a file
Then {string} does not exist
      # lookup fails with not-found: ENOENT, or ERROR_FILE_NOT_FOUND /
      # ERROR_PATH_NOT_FOUND on Windows
Then reading {string} gives:
      # doc string; the file's bytes equal the doc string plus one "\n"
Then the first line of {string} is {string}
Then {string} contains the line {string}
      # some whole line, without its "\n", equals the string
Then {string} contains {string}
Then {string} does not contain {string}
Then the content of {string} matches {string}
      # Rust `regex` syntax, multi-line mode, unanchored unless anchored
Then {string} is at most {int} bytes
Then {string} has the same bytes as {string}            # two view paths
Then reading {string} gives the bytes of the project file {string}
      # @pending-D11 only; the fixture then also needs its source files
```

## Whole-tree checks

```gherkin
When {string} is walked recursively
      # depth-first: readdir, then lookup and getattr of every entry,
      # following no symlinks; the step fails after 10,000 entries, which
      # is how a non-terminating walk shows up
Then the walk visits exactly:
      # doc string; one view path per line relative to the walked
      # directory, directories with a trailing "/", sorted bytewise
Then every view file under {string} starts with a VIEW header
      # the first line matches
      #   ^(# |// |%% |<!-- )VIEW of .+ @ [0-9a-f]{7}, indexed
      #    \d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ; read-only, edit the source file( -->)?$
      # and its comment syntax is the one the file's extension calls for;
      # @files/ (raw source under D11) is excluded
Then reading every file under {string} twice gives the same bytes
Then the size of every file under {string} equals the number of bytes read
      # getattr's size, taken before the read (PLAN.md §2.5)
When the views are rebuilt from the same generation
      # snapshots every file's bytes, then: walker, a new ViewFs; static,
      # materialize again; live, unmount and mount again
Then every file under {string} has the same bytes as before
Then every file under {string} is byte-identical to the in-memory rendering
      # compares with a walker over the same project (P5.4); trivially
      # true on the walker itself
Then no file under {string} contains {string}
```

## Read-only behaviour

Each `When` snapshots the tree first; the `Then` checks the error and that
the tree is unchanged.

```gherkin
When {string} is written with {string}
When the file {string} is created
When {string} is deleted
When the directory {string} is created
When {string} is renamed to {string}
Then the operation is refused as read-only
      # EROFS, EACCES or EPERM (ERROR_ACCESS_DENIED or ERROR_WRITE_PROTECT
      # on Windows), and the tree reads exactly as before
```

## Generations

```gherkin
Given {string} is held open
      # opened for reading, nothing read yet; held until the scenario ends
When generation {int} is published at commit {string}, indexed {string}, without the file {string}
      # writes and completes that generation from the current one with the
      # file's rows removed: its file row, its symbols, the call sites in it
      # and their targets
When the views switch to the newest generation
      # walker, swap the generation handle; static, materialize again
      # (sibling directory, then rename); live, wait until the daemon has
      # swapped (bounded, fails after 10 s)
Then the first line of the held file {string} is {string}
Then the held file {string} contains the line {string}
```

## Project files and the CLI

```gherkin
When codetags is run in the project with {string}
      # as `When codetags is run with`, with the fixture's project
      # directory as the working directory
Then no file {string} exists in the project or the working directory
      # project path; also checked relative to the views process's
      # working directory, where DuckDB resolves relative paths
```

## Mermaid

```gherkin
Then {string} parses with the Mermaid CLI     # @mermaid-cli only (V23)
```
