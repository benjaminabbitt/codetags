//! Delay-loads the WinFsp DLL in this package's Windows binaries, so they
//! start without WinFsp installed (D3).
//!
//! The flags mirror `winfsp::build::winfsp_link_delayload`, which
//! codetags-mount-winfsp's build script calls. They are repeated here because
//! `cargo:rustc-link-arg` reaches only the package that emits it, and the
//! GPL-3.0 `winfsp` crate may be a dependency of codetags-mount-winfsp only
//! (D9, deny.toml). Keep crates/codetags-bdd/build.rs the same.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let var = |name| std::env::var(name).unwrap_or_default();
    if var("CARGO_CFG_TARGET_OS") != "windows" || var("CARGO_CFG_TARGET_ENV") != "msvc" {
        return;
    }
    let dll = match var("CARGO_CFG_TARGET_ARCH").as_str() {
        "x86_64" => "winfsp-x64.dll",
        "x86" => "winfsp-x86.dll",
        "aarch64" => "winfsp-a64.dll",
        _ => return,
    };
    println!("cargo:rustc-link-lib=dylib=delayimp");
    println!("cargo:rustc-link-arg=/DELAYLOAD:{dll}");
}
