//! The session's toolchain, passed to the server explicitly (PLAN.md D27).
//!
//! The lspmux daemon starts with a fixed, minimal environment
//! ([`crate::daemon::daemon_env`]), and a server it spawns gets that
//! environment plus the variables its client passed through
//! `pass_environment`, which lspmux also keys instances on (V2, R9). So each
//! shim resolves its own session's toolchain, in the project directory so
//! that `rust-toolchain.toml` applies, and passes it as variables
//! normalized so that one toolchain is spelled one way whichever session
//! resolved it: sessions on the same toolchain share a server, and sessions
//! on different ones get their own.
//!
//! For rust-analyzer (V153): `RUSTUP_TOOLCHAIN` is the toolchain's sysroot
//! (rustup takes a toolchain directory there, as rust-analyzer itself sets
//! it, V154), `RUSTC` and `CARGO` its own binaries, `CARGO_HOME` and
//! `RUSTUP_HOME` their effective directories, and
//! [`KEY_TOOLCHAIN_ENV`] names the toolchain.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::tools;

/// The routing-key variable naming the session's toolchain (R9, D27).
pub const KEY_TOOLCHAIN_ENV: &str = "CODETAGS_KEY_TOOLCHAIN";

/// The toolchain variables the shim controls: what `codetags lsp setup`
/// adds to `pass_environment` beside `CODETAGS_KEY_*`. A session's own
/// values of these never reach lspmux; only the resolved ones do.
pub const PASSED: [&str; 5] = [
    "RUSTUP_TOOLCHAIN",
    "RUSTC",
    "CARGO",
    "CARGO_HOME",
    "RUSTUP_HOME",
];

/// How to find a session's toolchain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolchainRule {
    /// Rust: the sysroot `rustc --print sysroot` names in the project.
    Rust,
    /// None: nothing is passed. TODO(P3.4): `GOROOT` for gopls, the
    /// interpreter for pyright, Node for the TypeScript server.
    None,
}

impl ToolchainRule {
    /// The rule for a server binary: [`ToolchainRule::Rust`] for
    /// rust-analyzer, otherwise [`ToolchainRule::None`].
    pub fn for_server(server: &Path) -> Self {
        let stem = server
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if stem.starts_with("rust-analyzer") {
            Self::Rust
        } else {
            Self::None
        }
    }
}

/// The variables to pass for the session whose project is `project`,
/// under `rule`, with [`KEY_TOOLCHAIN_ENV`] among them. Empty when there is
/// nothing to pass or the toolchain cannot be found, which the shim reports.
pub fn resolve(rule: ToolchainRule, project: &Path) -> Result<Vec<(String, OsString)>, String> {
    match rule {
        ToolchainRule::None => Ok(Vec::new()),
        ToolchainRule::Rust => rust(project, &Env::process()),
    }
}

/// Where the session's variables come from (a seam for tests).
struct Env {
    rustc: Option<OsString>,
    cargo_home: Option<OsString>,
    rustup_home: Option<OsString>,
    home: Option<PathBuf>,
}

impl Env {
    fn process() -> Self {
        Self {
            rustc: std::env::var_os("RUSTC").filter(|value| !value.is_empty()),
            cargo_home: std::env::var_os("CARGO_HOME").filter(|value| !value.is_empty()),
            rustup_home: std::env::var_os("RUSTUP_HOME").filter(|value| !value.is_empty()),
            home: std::env::home_dir(),
        }
    }
}

fn rust(project: &Path, env: &Env) -> Result<Vec<(String, OsString)>, String> {
    let rustc_name = env.rustc.clone().unwrap_or_else(|| "rustc".into());
    let rustc = tools::resolve(Path::new(&rustc_name))
        .ok_or_else(|| format!("{} not found", rustc_name.to_string_lossy()))?;
    let output = Command::new(&rustc)
        .args(["--print", "sysroot"])
        .current_dir(project)
        // As rust-analyzer does: never let rustup install a toolchain here.
        .env("RUSTUP_AUTO_INSTALL", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("cannot run {}: {error}", rustc.display()))?;
    if !output.status.success() {
        return Err(format!(
            "`{} --print sysroot` failed in {}: {}",
            rustc.display(),
            project.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let sysroot = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if sysroot.as_os_str().is_empty() {
        return Err(format!(
            "`{} --print sysroot` printed nothing",
            rustc.display()
        ));
    }
    Ok(rust_variables(&sysroot, &rustc, env))
}

/// The variables for the toolchain at `sysroot`, found by running `rustc`.
fn rust_variables(sysroot: &Path, rustc: &Path, env: &Env) -> Vec<(String, OsString)> {
    let bin = sysroot.join("bin");
    let own = |name: &str| {
        let binary = bin.join(tools::exe_name(name));
        binary.is_file().then_some(binary)
    };
    let mut variables = vec![
        (
            KEY_TOOLCHAIN_ENV.to_string(),
            format!("rust:{}", sysroot.display()).into(),
        ),
        ("RUSTUP_TOOLCHAIN".to_string(), sysroot.as_os_str().into()),
        (
            "RUSTC".to_string(),
            own("rustc").unwrap_or_else(|| rustc.to_path_buf()).into(),
        ),
    ];
    if let Some(cargo) = own("cargo") {
        variables.push(("CARGO".to_string(), cargo.into()));
    }
    let home_default = |dir: &str| env.home.as_ref().map(|home| home.join(dir).into());
    if let Some(cargo_home) = env.cargo_home.clone().or_else(|| home_default(".cargo")) {
        variables.push(("CARGO_HOME".to_string(), cargo_home));
    }
    if let Some(rustup_home) = env.rustup_home.clone().or_else(|| home_default(".rustup")) {
        variables.push(("RUSTUP_HOME".to_string(), rustup_home));
    }
    variables
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn env(home: &Path) -> Env {
        Env {
            rustc: None,
            cargo_home: None,
            rustup_home: Some("/opt/rustup".into()),
            home: Some(home.to_path_buf()),
        }
    }

    #[test]
    fn rust_analyzer_gets_the_rust_rule() {
        assert_eq!(
            ToolchainRule::for_server(Path::new("/x/rust-analyzer")),
            ToolchainRule::Rust
        );
        assert_eq!(
            ToolchainRule::for_server(Path::new("/x/gopls")),
            ToolchainRule::None
        );
    }

    #[test]
    fn a_sysroot_with_its_own_binaries_is_named_by_them() {
        let scratch = tempfile::tempdir().unwrap();
        let sysroot = scratch.path().join("toolchains").join("1.96");
        std::fs::create_dir_all(sysroot.join("bin")).unwrap();
        for name in ["rustc", "cargo"] {
            std::fs::write(sysroot.join("bin").join(tools::exe_name(name)), "").unwrap();
        }
        let home = scratch.path().join("home");
        let variables = rust_variables(&sysroot, Path::new("/proxy/rustc"), &env(&home));
        let get = |name: &str| {
            variables
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| PathBuf::from(value))
        };
        assert_eq!(get("RUSTUP_TOOLCHAIN"), Some(sysroot.clone()));
        assert_eq!(
            get("RUSTC"),
            Some(sysroot.join("bin").join(tools::exe_name("rustc")))
        );
        assert_eq!(
            get("CARGO"),
            Some(sysroot.join("bin").join(tools::exe_name("cargo")))
        );
        assert_eq!(get("CARGO_HOME"), Some(home.join(".cargo")));
        assert_eq!(get("RUSTUP_HOME"), Some(PathBuf::from("/opt/rustup")));
        assert_eq!(
            get(KEY_TOOLCHAIN_ENV),
            Some(PathBuf::from(format!("rust:{}", sysroot.display())))
        );
        for (name, _) in &variables {
            assert!(
                name == KEY_TOOLCHAIN_ENV || PASSED.contains(&name.as_str()),
                "{name} is not passed by setup's pass_environment"
            );
        }
    }

    #[test]
    fn a_bare_sysroot_keeps_the_rustc_that_was_run() {
        let scratch = tempfile::tempdir().unwrap();
        let variables = rust_variables(
            scratch.path(),
            Path::new("/proxy/rustc"),
            &env(scratch.path()),
        );
        assert!(
            variables
                .iter()
                .any(|(name, value)| name == "RUSTC" && value == "/proxy/rustc")
        );
        assert!(!variables.iter().any(|(name, _)| name == "CARGO"));
    }
}
