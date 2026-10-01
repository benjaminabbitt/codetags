#!/usr/bin/env python3
"""Cross-job BDD coverage: no silent skips (PLAN.md §0.11).

Every CI job that runs BDD sets CODETAGS_BDD_RAN, and the runner appends one
line per scenario it runs:

    <feature file path relative to features/> :: <scenario name>

This script reads those files (from the directories or files given), parses
features/**/*.feature for scenario names, and fails, listing every scenario
that ran in no job and is not listed in ci/expected-skips.txt as

    scenario: <path> :: <name>  # reason

A Scenario Outline counts once. Its expanded names have their <placeholders>
filled in, so a placeholder matches any text.

It also fails on:
- an expected skip that is malformed, names no scenario, or ran anyway;
- a recorded line that names no scenario;
- two scenarios with the same name in one feature file.

Usage: bdd-coverage.py [--features DIR] [--skips FILE] RAN_PATH...
"""

import argparse
import re
import sys
from pathlib import Path

SCENARIO = re.compile(r"^\s*(Scenario Outline|Scenario Template|Scenario|Example):\s*(.*?)\s*$")
DOCSTRING = re.compile(r'^\s*("""|```)')
PLACEHOLDER = re.compile(r"<[^>]+>")
SKIP = re.compile(r"^scenario:\s*(?P<path>\S.*?) :: (?P<name>\S.*?)\s{2,}#\s*(?P<reason>\S.*)$")


def scenarios(features: Path) -> tuple[list[tuple[str, str]], list[str]]:
    """Every (path, name) under `features`, and any duplicate-name errors."""
    found: list[tuple[str, str]] = []
    errors: list[str] = []
    for file in sorted(features.rglob("*.feature")):
        path = file.relative_to(features).as_posix()
        seen: set[str] = set()
        in_docstring = None
        for line in file.read_text(encoding="utf-8").splitlines():
            fence = DOCSTRING.match(line)
            if fence:
                if in_docstring is None:
                    in_docstring = fence.group(1)
                elif in_docstring == fence.group(1):
                    in_docstring = None
                continue
            if in_docstring is not None:
                continue
            match = SCENARIO.match(line)
            if not match:
                continue
            name = match.group(2)
            if name in seen:
                errors.append(f"duplicate scenario name in one file: {path} :: {name}")
            seen.add(name)
            found.append((path, name))
    return found, errors


def matcher(path: str, name: str) -> re.Pattern[str]:
    """Matches the recorded lines of a scenario, or of each row of an outline."""
    parts = PLACEHOLDER.split(name)
    pattern = ".*".join(re.escape(part) for part in parts)
    return re.compile(re.escape(path) + " :: " + pattern + r"\Z", re.DOTALL)


def recorded(paths: list[Path]) -> set[str]:
    """Every line in the record files, searching directories recursively."""
    lines: set[str] = set()
    files: list[Path] = []
    for path in paths:
        if not path.exists():
            print(f"bdd-coverage: no such record path, so nothing recorded there: {path}")
            continue
        files.extend(sorted(p for p in path.rglob("*") if p.is_file()) if path.is_dir() else [path])
    for file in files:
        for line in file.read_text(encoding="utf-8").splitlines():
            if line.strip():
                lines.add(line.rstrip("\r"))
    print(f"bdd-coverage: read {len(files)} record file(s), {len(lines)} distinct line(s)")
    return lines


def expected_skips(file: Path) -> tuple[dict[tuple[str, str], str], list[str]]:
    """The `scenario:` entries of the skips file, and malformed-entry errors."""
    skips: dict[tuple[str, str], str] = {}
    errors: list[str] = []
    for number, line in enumerate(file.read_text(encoding="utf-8").splitlines(), 1):
        if not line.startswith("scenario:"):
            continue
        match = SKIP.match(line)
        if not match:
            errors.append(
                f"{file}:{number}: malformed; expected 'scenario: <path> :: <name>  # reason'"
            )
            continue
        skips[(match.group("path"), match.group("name"))] = match.group("reason")
    return skips, errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--features", type=Path, default=Path("features"))
    parser.add_argument("--skips", type=Path, default=Path("ci/expected-skips.txt"))
    parser.add_argument("ran", type=Path, nargs="+", help="record files or directories")
    args = parser.parse_args()

    all_scenarios, errors = scenarios(args.features)
    lines = recorded(args.ran)
    skips, skip_errors = expected_skips(args.skips)
    errors.extend(skip_errors)

    matchers = {key: matcher(*key) for key in all_scenarios}
    ran: set[tuple[str, str]] = set()
    for line in sorted(lines):
        hits = [key for key, pattern in matchers.items() if pattern.match(line)]
        if not hits:
            errors.append(f"recorded but not found in features/: {line}")
        ran.update(hits)

    known = set(all_scenarios)
    for key, reason in sorted(skips.items()):
        if key not in known:
            errors.append(f"expected skip names no scenario: {key[0]} :: {key[1]}")
        elif key in ran:
            errors.append(f"expected skip ran anyway; remove it: {key[0]} :: {key[1]}")
        else:
            print(f"bdd-coverage: expected skip: {key[0]} :: {key[1]}  # {reason}")

    missing = [key for key in all_scenarios if key not in ran and key not in skips]
    for path, name in missing:
        errors.append(f"ran in no job: {path} :: {name}")

    print(
        f"bdd-coverage: {len(all_scenarios)} scenario(s), {len(ran)} ran, "
        f"{len(skips)} expected skip(s), {len(missing)} not run"
    )
    sys.stdout.flush()
    for error in errors:
        print(f"bdd-coverage: {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
