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

## Scenario tags

Tags gate where a scenario runs (PLAN.md §3). They are read from the feature,
the rule, and the scenario.

| Tag | Scenario runs only… |
|---|---|
| `@linux`, `@macos`, `@windows` | on the named OSes; with no platform tag, on every OS |
| `@mount`, `@winfsp`, `@providers`, `@privileged`, `@slow` | when the job lists that capability in `CODETAGS_BDD_CAPABILITIES` (comma-separated) |

Each scenario that doesn't run is reported on stderr as `bdd: not run here: …`.
Skipped *steps* (steps with no definition) fail the run.
