//! Link settings for this package's binaries.
//!
//! **WinFsp delay-loading (Windows).** Delay-loads the WinFsp DLL in this
//! package's Windows binaries, so they start without WinFsp installed (D3).
//! The flags mirror `winfsp::build::winfsp_link_delayload`, which
//! codetags-mount-winfsp's build script calls. They are repeated here because
//! `cargo:rustc-link-arg` reaches only the package that emits it, and the
//! GPL-3.0 `winfsp` crate may be a dependency of codetags-mount-winfsp only
//! (D9, deny.toml). Keep crates/codetags-bdd/build.rs the same.
//!
//! **libduckdb run path (Linux, macOS).** Lets `target/<profile>/codetags`
//! find the prebuilt libduckdb when it runs outside cargo (V29, V72).
//! Development builds link libduckdb dynamically, and libduckdb-sys copies
//! the library into `target/<profile>/deps`. Cargo puts that directory on the
//! library path only for processes it starts (tests, `cargo run`), so the
//! binary gets a run path to `deps` beside itself: `$ORIGIN/deps` on Linux,
//! `@executable_path/deps` on macOS. Release builds link DuckDB statically
//! (`bundled-duckdb`), where the entry is unused. Windows has no run path:
//! `just` recipes put `deps` on `PATH` instead.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    duckdb_rpath();
    winfsp_delayload();
}

fn duckdb_rpath() {
    let rpath = match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("linux") => Some("$ORIGIN/deps"),
        Ok("macos") => Some("@executable_path/deps"),
        _ => None,
    };
    if let Some(rpath) = rpath {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,{rpath}");
    }
}

fn winfsp_delayload() {
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
