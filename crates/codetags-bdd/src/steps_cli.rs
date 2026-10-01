//! Steps that run the `codetags` binary and inspect what it did.

use std::process::Command;

use cucumber::{then, when};

use crate::{CODETAGS_BIN, CodetagsWorld};

#[when(expr = "codetags is run with {string}")]
fn codetags_is_run_with(world: &mut CodetagsWorld, args: String) {
    let binary = CODETAGS_BIN.get().expect("run() sets the binary path");
    let output = Command::new(binary)
        .args(args.split_whitespace())
        .output()
        .expect("spawn codetags");
    world.last = Some(output.into());
}

#[then(expr = "it exits with status {int}")]
fn it_exits_with_status(world: &mut CodetagsWorld, code: i32) {
    let last = world.last();
    assert_eq!(last.status, Some(code), "stderr was: {}", last.stderr);
}

#[then(expr = "stdout matches {string}")]
fn stdout_matches(world: &mut CodetagsWorld, pattern: String) {
    let regex = regex::Regex::new(&pattern).expect("the step's pattern is a valid regex");
    let stdout = &world.last().stdout;
    assert!(
        regex.is_match(stdout),
        "stdout {stdout:?} does not match {pattern:?}"
    );
}
