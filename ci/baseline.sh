#!/bin/sh
# Edge-count baselines of the provider fixtures (PLAN.md §9 P1.8; brief §4.4:
# pin SCIP bindings, and compare per-file edge counts after any indexer or
# binding upgrade). Run by `just baseline-check` and `just baseline-update`.
#
# Usage: ci/baseline.sh check|update FIXTURE...
#
# For each fixture, copies tests/fixtures/FIXTURE to a fresh temporary
# directory, indexes it there with `codetags index`, and then:
#   check   runs `codetags report --baseline tests/baselines/FIXTURE.json`,
#           which prints a diff and fails on any difference;
#   update  writes the fixture's edge counts to tests/baselines/FIXTURE.json.
# Every fixture is checked before the script fails, so one run shows every
# difference.
#
# Expects `cargo build --workspace --bins` to have run. POSIX sh: it runs
# under Git's sh on Windows, where libduckdb is found through PATH (as in
# `just doctor`).
set -eu

me=baseline
mode="${1:-}"
case "$mode" in
check | update) shift ;;
*)
    echo "usage: ci/baseline.sh check|update FIXTURE..." >&2
    exit 2
    ;;
esac
[ "$#" -gt 0 ] || {
    echo "$me: no fixtures given" >&2
    exit 2
}

repo="$PWD"
bin="${CARGO_TARGET_DIR:-target}/debug"
if command -v cygpath >/dev/null 2>&1; then
    bin="$(cygpath -ua "$bin")"
else
    case "$bin" in /*) ;; *) bin="$repo/$bin" ;; esac
fi
PATH="$bin/deps:$PATH"
codetags="$bin/codetags"
[ -x "$codetags" ] || [ -x "$codetags.exe" ] || {
    echo "$me: $codetags is not built" >&2
    exit 1
}

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT INT TERM

failed=""
for fixture in "$@"; do
    source="$repo/tests/fixtures/$fixture"
    baseline="$repo/tests/baselines/$fixture.json"
    [ -d "$source" ] || {
        echo "$me: no fixture tests/fixtures/$fixture" >&2
        exit 1
    }
    root="$scratch/$fixture"
    mkdir -p "$root"
    cp -R "$source/." "$root/"
    # The provider's build output stays out of the project copy.
    if ! CARGO_TARGET_DIR="$scratch/target-$fixture" "$codetags" index --root "$root" >"$scratch/$fixture.log" 2>&1; then
        cat "$scratch/$fixture.log" >&2
        echo "$me: indexing the fixture $fixture failed" >&2
        failed="$failed $fixture"
        continue
    fi
    if [ "$mode" = update ]; then
        "$codetags" report --root "$root" --write-baseline "$baseline" >/dev/null
        echo "$me: wrote tests/baselines/$fixture.json"
    elif [ ! -f "$baseline" ]; then
        echo "$me: tests/baselines/$fixture.json is missing; create it with: just baseline-update" >&2
        failed="$failed $fixture"
    elif "$codetags" report --root "$root" --baseline "$baseline"; then
        echo "$me: $fixture matches tests/baselines/$fixture.json"
    else
        failed="$failed $fixture"
    fi
done

if [ -n "$failed" ]; then
    echo "$me: failed for:$failed" >&2
    [ "$mode" = check ] && echo "$me: if a provider upgrade changed the counts on purpose, review them and run: just baseline-update" >&2
    exit 1
fi
echo "$me: $mode passed for: $*"
