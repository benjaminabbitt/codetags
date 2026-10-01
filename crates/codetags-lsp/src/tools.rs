//! Finding the programs the shim runs: lspmux and the real language server.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Environment variable naming the lspmux binary to use.
pub const LSPMUX_ENV: &str = "CODETAGS_LSPMUX";

/// `name` with the platform's executable suffix (`.exe` on Windows).
pub fn exe_name(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// Finds `program` the way a shell would: a path with a directory part is
/// made absolute as it is; a bare name is searched for on `PATH` (on
/// Windows also with `.exe`). Never canonicalizes, so the spelling is kept.
pub fn resolve(program: &Path) -> Option<PathBuf> {
    if program.components().count() > 1 || program.is_absolute() {
        return std::path::absolute(program).ok();
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|dir| {
            let plain = dir.join(program);
            let exe = dir.join(exe_name(&program.to_string_lossy()));
            [plain, exe]
        })
        .find(|candidate| candidate.is_file())
        .and_then(|found| std::path::absolute(found).ok())
}

/// The lspmux binary to use: `$CODETAGS_LSPMUX`; otherwise
/// `.codetags/local/bin/lspmux` under the current directory, where `just
/// setup-lspmux` installs it; otherwise `lspmux` on `PATH`.
pub fn find_lspmux() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os(LSPMUX_ENV).filter(|value| !value.is_empty()) {
        return resolve(Path::new(&explicit));
    }
    let local = Path::new(".codetags")
        .join("local")
        .join("bin")
        .join(exe_name("lspmux"));
    if local.is_file() {
        return std::path::absolute(local).ok();
    }
    resolve(Path::new(OsStr::new("lspmux")))
}

/// The git revision `cargo install` recorded for the lspmux at `binary`: the
/// full hash after `#` in the `lspmux` entry of `.crates.toml` in the
/// install root (the binary's directory's parent).
pub fn installed_lspmux_rev(binary: &Path) -> Option<String> {
    let root = binary.parent()?.parent()?;
    let text = std::fs::read_to_string(root.join(".crates.toml")).ok()?;
    text.lines()
        .filter(|line| line.trim_start().starts_with("\"lspmux "))
        .find_map(|line| {
            let after = line.split('#').nth(1)?;
            let rev: String = after.chars().take_while(char::is_ascii_hexdigit).collect();
            (!rev.is_empty()).then_some(rev)
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn reads_the_rev_cargo_install_recorded() {
        let scratch = tempfile::tempdir().unwrap();
        let bin = scratch.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        std::fs::write(
            scratch.path().join(".crates.toml"),
            "[v1]\n\"lspmux 0.3.0 (git+https://codeberg.org/p2502/lspmux?rev=18861f9#18861f9d59e74ece8d867772cf07fa302c2dae98)\" = [\"lspmux\"]\n",
        )
        .unwrap();
        assert_eq!(
            installed_lspmux_rev(&bin.join("lspmux")).as_deref(),
            Some(crate::lspmux::PINNED_REV)
        );
    }

    #[test]
    fn paths_with_a_directory_are_made_absolute() {
        let resolved = resolve(Path::new("some/dir/tool")).unwrap();
        assert!(resolved.is_absolute());
        assert!(resolved.ends_with("some/dir/tool"));
    }
}
