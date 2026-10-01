//! Wrappers: copies of `codetags-lsp` named after a language server, for
//! clients that can only start an executable (V118, V119).
//!
//! VS Code and Claude Code start a language server without a shell, so on
//! Windows a `.cmd` or `.bat` wrapper is refused, and a command with no
//! extension resolves to `<name>.exe`. A wrapper is therefore a copy of this
//! binary, `<dir>/<name>[.exe]`, with its settings beside it in
//! `<dir>/<name>.codetags-lsp.toml`; started under any name but
//! `codetags-lsp`, the binary reads them and runs [`crate::serve`]. Linux and
//! macOS can use the same wrappers, or sh scripts.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::policy::Role;
use crate::serve;

/// The suffix of a wrapper's settings file.
pub const SETTINGS_SUFFIX: &str = ".codetags-lsp.toml";

/// A wrapper's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The session role.
    pub role: Role,
    /// The real language server.
    pub server: PathBuf,
    /// The lspmux binary, if not the default.
    pub lspmux: Option<PathBuf>,
    /// A session log template, if any.
    pub log: Option<PathBuf>,
}

impl Settings {
    /// The settings as TOML.
    pub fn to_toml(&self) -> String {
        let mut table = toml::Table::new();
        table.insert("role".into(), self.role.as_str().into());
        table.insert(
            "server".into(),
            self.server.to_string_lossy().as_ref().into(),
        );
        if let Some(lspmux) = &self.lspmux {
            table.insert("lspmux".into(), lspmux.to_string_lossy().as_ref().into());
        }
        if let Some(log) = &self.log {
            table.insert("log".into(), log.to_string_lossy().as_ref().into());
        }
        format!(
            "# Settings for the codetags-lsp wrapper beside this file (codetags-lsp install-wrapper).\n{}",
            table
        )
    }

    /// Parses settings written by [`Settings::to_toml`].
    pub fn parse(text: &str) -> Result<Self, String> {
        let table: toml::Table = text.parse().map_err(|error| format!("not TOML: {error}"))?;
        let text_of = |key: &str| table.get(key).and_then(toml::Value::as_str);
        let role = text_of("role").ok_or("no `role`")?.parse()?;
        let server = PathBuf::from(text_of("server").ok_or("no `server`")?);
        Ok(Self {
            role,
            server,
            lspmux: text_of("lspmux").map(PathBuf::from),
            log: text_of("log").map(PathBuf::from),
        })
    }
}

/// The settings file for the wrapper executable at `exe`.
pub fn settings_path(exe: &Path) -> PathBuf {
    let stem = exe
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    exe.with_file_name(format!("{stem}{SETTINGS_SUFFIX}"))
}

/// If this process was started as a wrapper (its file name is not
/// `codetags-lsp`), the wrapper's executable path.
pub fn invoked_as_wrapper() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = std::env::args_os()
        .next()
        .map(PathBuf::from)
        .and_then(|argv0| argv0.file_stem().map(|stem| stem.to_os_string()))
        .or_else(|| exe.file_stem().map(|stem| stem.to_os_string()))?;
    if name == "codetags-lsp" {
        return None;
    }
    // On Linux, current_exe resolves symlinks; the settings sit beside the
    // name the client started.
    let dir = std::env::args_os()
        .next()
        .map(PathBuf::from)
        .filter(|argv0| argv0.components().count() > 1)
        .and_then(|argv0| argv0.parent().map(Path::to_path_buf))
        .or_else(|| exe.parent().map(Path::to_path_buf))?;
    Some(dir.join(format!(
        "{}{}",
        name.to_string_lossy(),
        std::env::consts::EXE_SUFFIX
    )))
}

/// Runs the wrapper at `exe` with the client's arguments `args`.
pub fn run(exe: &Path, args: Vec<OsString>) -> Result<serve::Outcome, String> {
    let path = settings_path(exe);
    let text = std::fs::read_to_string(&path).map_err(|error| {
        format!(
            "{} is a codetags-lsp wrapper, but its settings {} cannot be read: {error}",
            exe.display(),
            path.display()
        )
    })?;
    let settings =
        Settings::parse(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    serve::run(&serve::Options {
        role: settings.role,
        server: settings.server,
        args,
        lspmux: settings.lspmux,
        log: settings.log,
        lsp_subcommands: Vec::new(),
        root_rule: None,
    })
}

/// Installs a wrapper at `dest` (`.exe` added on Windows): a copy of this
/// executable, and its settings beside it. Returns the wrapper's path.
pub fn install(dest: &Path, settings: &Settings) -> Result<PathBuf, String> {
    let mut exe = dest.to_path_buf();
    if cfg!(windows) && exe.extension().is_none() {
        exe.set_extension("exe");
    }
    let me = std::env::current_exe().map_err(|error| format!("cannot find myself: {error}"))?;
    if let Some(dir) = exe.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    }
    // Through a temporary name and a rename, so a running wrapper keeps its
    // binary (on Windows, a running one cannot be replaced at all).
    let temporary = exe.with_file_name(format!(
        "{}.new",
        exe.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    std::fs::copy(&me, &temporary).map_err(|error| {
        format!(
            "cannot copy {} to {}: {error}",
            me.display(),
            temporary.display()
        )
    })?;
    std::fs::rename(&temporary, &exe)
        .map_err(|error| format!("cannot replace {}: {error}", exe.display()))?;
    let path = settings_path(&exe);
    std::fs::write(&path, settings.to_toml())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(exe)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn settings_round_trip() {
        let settings = Settings {
            role: Role::Agent,
            server: PathBuf::from("C:\\tools\\rust-analyzer.exe"),
            lspmux: Some(PathBuf::from("/x/lspmux")),
            log: None,
        };
        assert_eq!(Settings::parse(&settings.to_toml()).unwrap(), settings);
    }

    #[test]
    fn settings_sit_beside_the_wrapper() {
        assert_eq!(
            settings_path(Path::new("/a/rust-analyzer.exe")),
            Path::new("/a/rust-analyzer.codetags-lsp.toml")
        );
        assert_eq!(
            settings_path(Path::new("/a/rust-analyzer")),
            Path::new("/a/rust-analyzer.codetags-lsp.toml")
        );
    }
}
