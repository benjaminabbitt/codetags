//! The "lsp wiring" line of `codetags doctor` (PLAN.md D25): whether this
//! checkout's Claude Code plugin and VS Code would run rust-analyzer
//! through the shim, and if not, why and how to fix it.
//!
//! It works off the facts the wrappers themselves check. On Linux and macOS
//! the sh wrappers (`tools/lsp/rust-analyzer-shim.sh`) need
//! `.codetags/local/bin/codetags-lsp`, executable and able to `serve` (a
//! stage-0 build cannot), a non-empty `.codetags/local/rust-analyzer.path`,
//! and lspmux in `.codetags/local/bin` or on PATH. On Windows the clients
//! start the wrapper executables `just lsp-setup` installs beside the sh
//! wrappers (V119), each with its settings file. The line is information,
//! never a failure: without the shim, the wrappers deliberately fall back to
//! rust-analyzer alone.

use std::path::{Path, PathBuf};

/// The file that marks a checkout with this repo's LSP wiring.
pub const SHIM_SCRIPT: [&str; 3] = ["tools", "lsp", "rust-analyzer-shim.sh"];

/// The wrappers the clients start, relative to the checkout: Claude Code's
/// plugin, then VS Code.
pub const WRAPPERS: [(&str, &[&str]); 2] = [
    (
        "Claude Code",
        &[
            "tools",
            "claude-plugins",
            "codetags-lsp",
            "scripts",
            "rust-analyzer",
        ],
    ),
    ("VS Code", &["tools", "vscode", "rust-analyzer"]),
];

/// The checkout at or above `start` that holds [`SHIM_SCRIPT`].
pub fn find_checkout(start: &Path) -> Option<PathBuf> {
    let script: PathBuf = SHIM_SCRIPT.iter().collect();
    start
        .ancestors()
        .find(|dir| dir.join(&script).is_file())
        .map(Path::to_path_buf)
}

/// The doctor's line for the checkout at or above `cwd`.
pub fn line(cwd: &Path) -> String {
    let Some(checkout) = find_checkout(cwd) else {
        return format!(
            "lsp wiring: no checkout with codetags' LSP wiring ({}) at or above {}",
            SHIM_SCRIPT.join("/"),
            cwd.display()
        );
    };
    let checked = if cfg!(windows) {
        check_windows(&checkout)
    } else {
        check_unix(&checkout, || {
            crate::tools::resolve(Path::new("lspmux")).filter(|found| is_executable(found))
        })
    };
    match checked {
        Ok(through) => format!(
            "lsp wiring: {}: Claude Code and VS Code run rust-analyzer through the shim ({through})",
            checkout.display()
        ),
        Err(why) => format!(
            "lsp wiring: {}: Claude Code and VS Code run rust-analyzer without the shim: {why}",
            checkout.display()
        ),
    }
}

/// The sh wrappers' checks, in their order. `Ok` names lspmux and the
/// server; `Err` says why not, with the fix. `lspmux_on_path` finds lspmux
/// on PATH, as the wrapper's `command -v lspmux` does.
pub fn check_unix(
    checkout: &Path,
    lspmux_on_path: impl Fn() -> Option<PathBuf>,
) -> Result<String, String> {
    let local = checkout.join(".codetags").join("local");
    let shim = local.join("bin").join("codetags-lsp");
    let server = std::fs::read_to_string(local.join("rust-analyzer.path")).unwrap_or_default();
    // The wrapper tests `-s` (non-empty) and reads the file as it is.
    let server = server.trim_end_matches(['\n', '\r']);
    if !is_executable(&shim) || server.is_empty() {
        return Err("codetags-lsp is not set up (run: just lsp-setup)".into());
    }
    let serves = std::process::Command::new(&shim)
        .args(["serve", "--help"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !serves {
        return Err(format!(
            "{} cannot serve (run: just lsp-setup)",
            shim.display()
        ));
    }
    let local_lspmux = local.join("bin").join("lspmux");
    let lspmux = if is_executable(&local_lspmux) {
        local_lspmux
    } else {
        lspmux_on_path().ok_or("lspmux not found (run: just setup-lspmux)")?
    };
    Ok(format!(
        "lspmux {}; rust-analyzer {server}",
        lspmux.display()
    ))
}

/// The Windows clients' wrapper executables and their settings. `Ok` names
/// lspmux and the server; `Err` says why not, with the fix.
pub fn check_windows(checkout: &Path) -> Result<String, String> {
    let mut through = None;
    for (client, parts) in WRAPPERS {
        let exe = checkout
            .join(parts.iter().collect::<PathBuf>())
            .with_extension("exe");
        if !exe.is_file() {
            return Err(format!(
                "{client} has no wrapper executable {} (run: just lsp-setup)",
                exe.display()
            ));
        }
        let path = crate::wrapper::settings_path(&exe);
        let settings = std::fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| crate::wrapper::Settings::parse(&text))
            .map_err(|error| format!("{}: {error} (run: just lsp-setup)", path.display()))?;
        // `just lsp-setup` always names lspmux; without it, serve looks for
        // one as `crate::tools::find_lspmux` does, from the client's
        // directory, which doctor cannot know.
        let lspmux = settings
            .lspmux
            .ok_or_else(|| format!("{}: names no lspmux (run: just lsp-setup)", path.display()))?;
        if !lspmux.is_file() {
            return Err(format!(
                "lspmux {} not found (run: just setup-lspmux)",
                lspmux.display()
            ));
        }
        through.get_or_insert(format!(
            "lspmux {}; rust-analyzer {}",
            lspmux.display(),
            settings.server.display()
        ));
    }
    through.ok_or_else(|| "no wrappers to check".into())
}

/// Whether `path` is a file this user may run (on Windows: a file).
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// A checkout with the shim script, and its `.codetags/local`.
    fn checkout() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let script: PathBuf = SHIM_SCRIPT.iter().collect();
        std::fs::create_dir_all(dir.path().join(&script).parent().unwrap()).unwrap();
        std::fs::write(dir.path().join(&script), "#!/bin/sh\n").unwrap();
        let local = dir.path().join(".codetags").join("local");
        std::fs::create_dir_all(local.join("bin")).unwrap();
        (dir, local)
    }

    /// Writes an executable sh script, and waits until it can be run: a
    /// test thread that forks while the file is open for writing makes
    /// exec fail with ETXTBSY until that child has exec'd.
    #[cfg(unix)]
    fn script(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        for _ in 0..100 {
            match std::process::Command::new(path).arg("--probe").status() {
                Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                _ => return,
            }
        }
    }

    #[test]
    fn finds_the_checkout_above() {
        let (dir, _) = checkout();
        let deep = dir.path().join("src").join("deep");
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(find_checkout(&deep).as_deref(), Some(dir.path()));
        let elsewhere = tempfile::tempdir().unwrap();
        assert_eq!(find_checkout(elsewhere.path()), None);
        assert!(line(elsewhere.path()).starts_with("lsp wiring: no checkout"));
    }

    #[cfg(unix)]
    #[test]
    fn unix_without_setup_names_lsp_setup() {
        let (dir, _) = checkout();
        let why = check_unix(dir.path(), || None).unwrap_err();
        assert_eq!(why, "codetags-lsp is not set up (run: just lsp-setup)");
    }

    #[cfg(unix)]
    #[test]
    fn unix_shim_that_cannot_serve_names_lsp_setup() {
        let (dir, local) = checkout();
        let shim = local.join("bin").join("codetags-lsp");
        script(&shim, "[ \"$1\" = serve ] && exit 2; exit 0");
        std::fs::write(local.join("rust-analyzer.path"), "/ra\n").unwrap();
        let why = check_unix(dir.path(), || None).unwrap_err();
        assert_eq!(
            why,
            format!("{} cannot serve (run: just lsp-setup)", shim.display())
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_without_lspmux_names_setup_lspmux() {
        let (dir, local) = checkout();
        script(&local.join("bin").join("codetags-lsp"), "exit 0");
        std::fs::write(local.join("rust-analyzer.path"), "/ra\n").unwrap();
        let why = check_unix(dir.path(), || None).unwrap_err();
        assert_eq!(why, "lspmux not found (run: just setup-lspmux)");
    }

    #[cfg(unix)]
    #[test]
    fn unix_with_everything_runs_through_the_shim() {
        let (dir, local) = checkout();
        script(&local.join("bin").join("codetags-lsp"), "exit 0");
        std::fs::write(local.join("rust-analyzer.path"), "/ra\n").unwrap();
        let on_path = PathBuf::from("/usr/bin/lspmux");
        let through = check_unix(dir.path(), || Some(on_path.clone())).unwrap();
        assert_eq!(through, "lspmux /usr/bin/lspmux; rust-analyzer /ra");
        let lspmux = local.join("bin").join("lspmux");
        script(&lspmux, "exit 0");
        let through = check_unix(dir.path(), || None).unwrap();
        assert_eq!(
            through,
            format!("lspmux {}; rust-analyzer /ra", lspmux.display())
        );
    }

    #[test]
    fn windows_without_wrappers_names_lsp_setup() {
        let (dir, _) = checkout();
        let why = check_windows(dir.path()).unwrap_err();
        assert!(
            why.starts_with("Claude Code has no wrapper executable "),
            "{why}"
        );
        assert!(why.ends_with(" (run: just lsp-setup)"), "{why}");
    }

    /// Installs a wrapper executable and its settings for `client`.
    fn wrapper(checkout: &Path, client: usize, lspmux: &Path) {
        let parts: PathBuf = WRAPPERS[client].1.iter().collect();
        let exe = checkout.join(parts).with_extension("exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "").unwrap();
        let settings = crate::wrapper::Settings {
            role: crate::policy::Role::Agent,
            server: PathBuf::from("/ra"),
            lspmux: Some(lspmux.to_path_buf()),
            log: None,
        };
        std::fs::write(crate::wrapper::settings_path(&exe), settings.to_toml()).unwrap();
    }

    #[test]
    fn windows_with_both_wrappers_runs_through_the_shim() {
        let (dir, local) = checkout();
        let lspmux = local.join("bin").join("lspmux.exe");
        std::fs::write(&lspmux, "").unwrap();
        wrapper(dir.path(), 0, &lspmux);
        let why = check_windows(dir.path()).unwrap_err();
        assert!(
            why.starts_with("VS Code has no wrapper executable "),
            "{why}"
        );
        wrapper(dir.path(), 1, &lspmux);
        let through = check_windows(dir.path()).unwrap();
        assert_eq!(
            through,
            format!("lspmux {}; rust-analyzer /ra", lspmux.display())
        );
    }

    #[test]
    fn windows_wrapper_whose_lspmux_is_gone_names_setup_lspmux() {
        let (dir, local) = checkout();
        let lspmux = local.join("bin").join("lspmux.exe");
        wrapper(dir.path(), 0, &lspmux);
        wrapper(dir.path(), 1, &lspmux);
        let why = check_windows(dir.path()).unwrap_err();
        assert_eq!(
            why,
            format!(
                "lspmux {} not found (run: just setup-lspmux)",
                lspmux.display()
            )
        );
    }
}
