//! Runs every feature under `features/` against the built `codetags` binary.
//! Entry point: `just bdd`.

use std::path::PathBuf;

/// Stack for the thread that runs cucumber. The runner's future and the
/// scenario world live on it, and Windows gives the main thread only 1 MB
/// (Linux 8 MB), which they outgrew (STATUS_STACK_OVERFLOW in CI).
const RUNNER_STACK: usize = 64 * 1024 * 1024;

fn main() {
    // Steps that need "another process" re-run this executable as a child.
    // Child modes are small synchronous jobs, so they stay on this thread.
    codetags_bdd::maybe_run_child();
    let features = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../features");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_codetags"));
    // `run` exits the process with cucumber's verdict from this thread; a
    // panic in it is re-raised here, so it still fails the run.
    let runner = std::thread::Builder::new()
        .name("bdd".into())
        .stack_size(RUNNER_STACK)
        .spawn(move || futures::executor::block_on(codetags_bdd::run(features, binary)))
        .expect("spawn the BDD runner thread");
    if let Err(panic) = runner.join() {
        std::panic::resume_unwind(panic);
    }
}
