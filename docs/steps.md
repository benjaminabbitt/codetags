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

## Scenario tags

Tags gate where a scenario runs (PLAN.md §3). They are read from the feature,
the rule, and the scenario.

| Tag | Scenario runs only… |
|---|---|
| `@linux`, `@macos`, `@windows` | on the named OSes; with no platform tag, on every OS |
| `@mount`, `@winfsp`, `@providers`, `@privileged`, `@slow` | when the job lists that capability in `CODETAGS_BDD_CAPABILITIES` (comma-separated) |

Each scenario that doesn't run is reported on stderr as `bdd: not run here: …`.
Skipped *steps* (steps with no definition) fail the run.
