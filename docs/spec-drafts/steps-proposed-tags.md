# Proposed steps for the tagging features (P6.1, draft)

**Status:** draft for human review, 2026-09-30. These steps are used by
`docs/spec-drafts/tags/*.feature` and are not in `docs/steps.md` yet. On
approval they move there, in the same commit as their definitions in
`crates/codetags-bdd` (PLAN.md §0.3).

Conventions are those of `docs/steps.md`: `{string}` is a double-quoted
cucumber-expression string (single quotes allowed, to hold `"`), `{int}` a
decimal integer. A `{string}` holds its characters literally; a backslash is
a backslash, except `\"` (or `\'`) inside the quotes. Paths in steps are
project-relative and `/`-separated on every OS.

## Questions for review

1. **The existing step `When codetags is run with {string}` changes
   meaning slightly:** once a scenario has a project, it runs in the
   project's root. Without a project it runs where it runs today (the
   scenario's scratch directory). OK, or a separate step?
2. **Byte-exact docstrings.** Gherkin strips a docstring's indentation and
   its final newline. `the tags file is:` therefore compares the file with
   the docstring plus one `\n`, and an empty docstring with the empty file.
   `the tags file's bytes are {string}` exists for the cases a docstring
   can't express (CRLF, no final newline, empty); it is the only step whose
   string interprets `\n`, `\r` and `\\`. Acceptable, or one mechanism?
3. **Git in the BDD runner.** The merge scenarios need `git` on `PATH` in
   every CI job that runs `just bdd` (it is, on all three hosted runners).
   The steps pin the identity, `merge.conflictStyle=merge`, `core.autocrlf
   =false` and `init.defaultBranch`, so the output is byte-stable. Should
   these scenarios carry a capability tag (e.g. `@git`) instead?
4. **Concurrency step.** "concurrently" means all children are spawned before
   any is awaited. It can't force interleaving; the lock scenarios prove no
   lost update under contention, not a particular schedule. Enough?
5. **Lock-holder child.** `another process holds the tags lock` re-runs the
   test executable as a child that takes `tags.lock` with the same locking
   call as the product (V32), as the DuckDB steps already do. The "killed
   mid-write" step needs a test-only hook in the product to stop between
   writing the temporary file and renaming it. Proposed: the child links
   `codetags-tags` and calls its writer with a test callback; no hook in the
   shipped binary. OK?

## Project and files

```gherkin
Given a project with the files {string}
      # whitespace-separated paths; a fresh scratch directory that is a git
      # work tree (identity, conflictStyle, autocrlf pinned; branch "main"),
      # holding `.codetags/` with only its §2.3 `.gitignore`, and each named
      # file (empty; parent directories created). codetags runs in its root.
Given the project also has the file {string}
      # one path, which may hold spaces or quotes; created empty
Given a git work tree with the files {string} and no .codetags directory
Given a directory with the files {string} that is not in a git work tree
      # created outside any git work tree; GIT_CEILING_DIRECTORIES is set
      # so a parent repository is never found
Given the project config holds:              # docstring → .codetags/config.toml
When the file {string} is deleted
When the file {string} is renamed to {string}
When the directory {string} is renamed to {string}
      # plain filesystem renames, not `git mv`
Then the file {string} holds:                # docstring + "\n", byte-exact
```

## The tags file

```gherkin
Given the tags file holds:                   # docstring + "\n" → .codetags/tags, byte-exact
Given the tags file's bytes are {string}     # \n, \r and \\ are interpreted
Given there is no tags file                  # removes .codetags/tags if present
Then the tags file is:                       # .codetags/tags == docstring + "\n"; "" docstring = empty file
Then the tags file's bytes are {string}
Then the tags file is unchanged              # byte-equal to its content before the scenario's first When
Then there is no tags file
```

## Commands

```gherkin
When codetags is run in {string} with {string}
      # as `codetags is run with`, in that project subdirectory
When codetags is run with the arguments:
      # a one-row table; each cell is one argument, kept whole (spaces,
      # quotes). Gherkin cell escapes apply: `\n` newline, `\|`, `\\`.
When codetags is run concurrently, once with each of:
      # a one-column table; each row is an argument string split on
      # whitespace; every child is spawned before any is awaited
Given the environment variable {string} is {string}
      # set for every codetags process the scenario starts
Then every run exits with status {int}
Then stderr matches {string}                 # Rust regex, unanchored; `(?m)` for line anchors
Then stdout is:                              # docstring + "\n", byte-exact
Then stdout is empty
Then stderr is empty
```

## Lock

```gherkin
Given another process holds the tags lock
      # a child holds .codetags/tags.lock until the scenario ends
Given another process holds the tags lock for {int} ms
      # the child releases it after that many milliseconds
Given a writer process was killed after writing its temporary file and before renaming it
      # the child takes the lock, writes its temporary file, and is killed
      # (SIGKILL / TerminateProcess) before the rename
Then git reports no untracked files under {string}
      # `git status --porcelain --untracked-files=all -- <dir>` prints nothing
```

## Git merges

```gherkin
Given the project is committed on branch {string}
      # commits every file in the project on that branch
Given on a new branch {string} from {string}, codetags is run with {string} and the result committed
      # checks out a new branch from the second, runs codetags (must exit 0),
      # commits `.codetags/tags`, and checks out the branch it started on
When branch {string} is merged into {string}
Given branch {string} is merged into {string}
      # checks out the second, runs `git merge --no-edit <first>`; a conflict
      # is not a step failure
Then the merge succeeds                      # git merge exited 0
Then the merge stops with a conflict in {string}
      # git merge exited 1 and `git diff --name-only --diff-filter=U` lists the path
Given the project uses the codetags merge driver for {string}
      # @pending-merge-driver only: writes the .gitattributes line and the
      # .git/config driver entry (merge.feature Q1)
```

## Scenario tags

| Tag | Meaning |
|---|---|
| `@pending-D11` | Placeholder awaiting D11/D12 review; the runner must not run it, and `ci/expected-skips.txt` lists nothing for it, because it has no scenarios. |
| `@pending-merge-driver` | Awaits merge.feature Q1. If Q1 is declined, the scenario is deleted, not skipped. |

No new capability tag is proposed for tagging; every scenario runs in the
`bdd` recipe, with `@linux @macos` on the two that need `"` or `\` in a file
name (Windows forbids `"`; `\` is its separator).
