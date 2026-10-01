//! Steps that run the `codetags` binary and inspect what it did.

use std::process::Command;

use cucumber::{given, then, when};

use crate::{CODETAGS_BIN, CodetagsWorld};

#[given(expr = "the environment variable {string} is {string}")]
fn the_environment_variable_is(world: &mut CodetagsWorld, name: String, value: String) {
    world.env.push((name.into(), value.into()));
}

#[when(expr = "codetags is run with {string}")]
fn codetags_is_run_with(world: &mut CodetagsWorld, args: String) {
    let binary = CODETAGS_BIN.get().expect("run() sets the binary path");
    let mut command = Command::new(binary);
    command.args(args.split_whitespace());
    for name in &world.env_removed {
        command.env_remove(name);
    }
    command.envs(world.env.iter().map(|(name, value)| (name, value)));
    let output = command.output().expect("spawn codetags");
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

#[then(expr = "stderr matches {string}")]
fn stderr_matches(world: &mut CodetagsWorld, pattern: String) {
    let regex = regex::Regex::new(&pattern).expect("the step's pattern is a valid regex");
    let stderr = &world.last().stderr;
    assert!(
        regex.is_match(stderr),
        "stderr {stderr:?} does not match {pattern:?}"
    );
}
