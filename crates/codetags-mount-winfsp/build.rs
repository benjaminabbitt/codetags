//! Delay-loads the WinFsp DLL, so binaries start without WinFsp installed
//! (D3). `cargo:rustc-link-arg` applies only to this package's own targets
//! (its tests here), so each package that links a binary using this backend
//! (`codetags`, `codetags-bdd`) has a build script that does the same.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // `cfg(windows)` is the host: winfsp is a build dependency only there. A
    // cross-check from another host links nothing, so needs no flags.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winfsp::build::winfsp_link_delayload();
    }
}
