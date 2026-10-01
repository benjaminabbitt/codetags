#!/usr/bin/env bash
# Asserts that the job runs product code unprivileged (PLAN.md §5).
# Linux/macOS: not root. Windows: not elevated, unless that check is listed as
# pending in ci/expected-skips.txt (PLAN.md §0.11, V18).
set -euo pipefail

skips="$(dirname "$0")/expected-skips.txt"

case "$(uname -s)" in
  Linux|Darwin)
    if [ "$(id -u)" -eq 0 ]; then
      echo "assert-unprivileged: running as root" >&2
      exit 1
    fi
    echo "assert-unprivileged: uid $(id -u), not root"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    # `net session` succeeds only from an elevated process.
    if net session >/dev/null 2>&1; then
      if grep -qx 'windows: assert-unprivileged' "$skips"; then
        echo "assert-unprivileged: elevated; listed as pending in ci/expected-skips.txt (V18)"
      else
        echo "assert-unprivileged: elevated, and not listed in ci/expected-skips.txt; in CI, run it through ci/run-deelevated.ps1 (V18)" >&2
        exit 1
      fi
    else
      echo "assert-unprivileged: not elevated"
    fi
    ;;
  *)
    echo "assert-unprivileged: unknown platform $(uname -s)" >&2
    exit 1
    ;;
esac
