#!/bin/sh
# Verifies the prebuilt libduckdb archive that libduckdb-sys downloaded against
# the pin in duckdb.sha256 (PLAN.md R8; V68, V69). Run by `just duckdb-verify`
# after a build has run libduckdb-sys's build script.
#
# Usage: ci/duckdb-verify.sh [TARGET]
#   With no TARGET, checks the host target's cache, which cargo puts in
#   <target-dir>/duckdb-download/. With TARGET (a build run with --target),
#   checks <target-dir>/TARGET/duckdb-download/.
#
# POSIX sh: it runs under Git's sh on Windows. Hashing uses the first of
# sha256sum (GNU coreutils: Linux, Git for Windows), shasum -a 256 (macOS
# base system) or openssl that exists.
set -eu

pins=duckdb.sha256
me=duckdb-verify

fail() {
    echo "$me: $*" >&2
    exit 1
}

# DUCKDB_LIB_DIR takes precedence over the download in libduckdb-sys
# (V29), so nothing was downloaded and there is no archive to check.
if [ -n "${DUCKDB_LIB_DIR:-}" ]; then
    echo "$me: DUCKDB_LIB_DIR is set, so no archive was downloaded; nothing to verify"
    exit 0
fi

[ -f "$pins" ] || fail "$pins not found; run from the repository root"
[ -f Cargo.lock ] || fail "Cargo.lock not found; run from the repository root"

# DuckDB version from the locked libduckdb-sys: crate 1.MMmmpp.x is DuckDB
# MM.mm.pp, as libduckdb-sys's build.rs derives it.
crate_version=$(tr -d '\r' <Cargo.lock | awk '
    $0 == "name = \"libduckdb-sys\"" { found = 1; next }
    found && /^version = / { gsub(/"/, "", $3); print $3; exit }
')
[ -n "$crate_version" ] || fail "libduckdb-sys not found in Cargo.lock"
encoded=$(echo "$crate_version" | cut -d. -f2)
case "$encoded" in
    '' | *[!0-9]*) fail "cannot derive the DuckDB version from libduckdb-sys $crate_version" ;;
esac
encoded=$(expr "$encoded" + 0)
version="$((encoded / 10000)).$((encoded / 100 % 100)).$((encoded % 100))"

if [ $# -ge 1 ] && [ -n "$1" ]; then
    target=$1
    cache_root="${CARGO_TARGET_DIR:-target}/$target/duckdb-download"
else
    target=$(rustc -vV | tr -d '\r' | sed -n 's/^host: //p')
    [ -n "$target" ] || fail "cannot read the host target from rustc -vV"
    cache_root="${CARGO_TARGET_DIR:-target}/duckdb-download"
fi

# Archive names as in libduckdb-sys's build.rs (LibduckdbArchive::for_target).
case "$target" in
    *apple-darwin) archive=libduckdb-osx-universal.zip ;;
    x86_64-unknown-linux-gnu) archive=libduckdb-linux-amd64.zip ;;
    aarch64-unknown-linux-gnu) archive=libduckdb-linux-arm64.zip ;;
    x86_64-pc-windows-msvc) archive=libduckdb-windows-amd64.zip ;;
    aarch64-pc-windows-msvc) archive=libduckdb-windows-arm64.zip ;;
    *) fail "libduckdb-sys has no prebuilt archive for target $target" ;;
esac

expected=$(tr -d '\r' <"$pins" | awk -v v="$version" -v a="$archive" '
    $1 == v && $2 == a { print $3; exit }
')
[ -n "$expected" ] || fail "no pin for DuckDB $version $archive in $pins; see its header for how to add one"

path="$cache_root/$target/$version/$archive"
[ -f "$path" ] || fail "$path not found. Build first; if the library is cached without its archive, run: cargo clean -p libduckdb-sys"

if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$path" | cut -d' ' -f1)
elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$path" | cut -d' ' -f1)
elif command -v openssl >/dev/null 2>&1; then
    actual=$(openssl dgst -sha256 -r "$path" | cut -d' ' -f1)
else
    fail "no SHA-256 tool found (need sha256sum, shasum or openssl)"
fi
# sha256sum on Windows may prefix the hash with '\' for unusual paths.
actual=$(echo "$actual" | tr -d '\\\r' | tr 'A-F' 'a-f')

if [ "$actual" != "$expected" ]; then
    echo "$me: SHA-256 MISMATCH for $path" >&2
    echo "$me:   expected $expected (pinned in $pins)" >&2
    echo "$me:   actual   $actual" >&2
    echo "$me: do not use this library. Delete $cache_root/$target/$version and run: cargo clean -p libduckdb-sys" >&2
    exit 1
fi
echo "$me: $archive (DuckDB $version, $target) matches the pinned SHA-256"
