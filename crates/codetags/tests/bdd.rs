//! Runs every feature under `features/` against the built `codetags` binary.
//! Entry point: `just bdd`.

use std::path::PathBuf;

fn main() {
    let features = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../features");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_codetags"));
    futures::executor::block_on(codetags_bdd::run(features, binary));
}
