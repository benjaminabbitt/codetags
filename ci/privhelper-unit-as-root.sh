#!/bin/sh
# Runs codetags-privhelper's unit tests as root, for the branches that only
# root takes (such as a root-owned socket directory accepted). `just test-privileged unit-as-root` calls this in CI's
# privileged-linux job, where sudo needs no password.
#
# The test binary is built as the calling user, so nothing in target/ ends up
# owned by root; only the finished binary runs under `sudo -n`. The crate's
# integration tests (tests/unprivileged.rs) are not run: they test the
# helper without privilege.
set -eu

exe="$(cargo test --workspace --bin codetags-privhelper --no-run --message-format=json \
    | grep '"test":true' \
    | grep '"name":"codetags-privhelper"' \
    | sed -n 's/.*"executable":"\([^"]*\)".*/\1/p' \
    | head -n 1)"
if [ -z "$exe" ]; then
    echo "privhelper-unit-as-root: cargo reported no test binary for codetags-privhelper"
    exit 1
fi
echo "privhelper-unit-as-root: running $exe as root"
sudo -n "$exe"
