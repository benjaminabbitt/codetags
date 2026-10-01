//! Whether the WinFsp backend can be used, and how to describe it.
//!
//! Platform-neutral, so `codetags doctor` wording is unit-tested everywhere.
//! Loading the DLL itself is Windows-only (`load`).

use std::ffi::OsStr;
use std::fmt;
use std::path::PathBuf;

/// Environment variable that turns the WinFsp backend off when set to `off`.
/// codetags then behaves as if WinFsp were not installed: static views and
/// the CLI only (D3).
pub const DISABLE_ENV: &str = "CODETAGS_WINFSP";

/// The notice WinFsp's FLOSS exception requires in our UI and user-facing
/// docs (V14), shown with [`REPO_URL`].
pub const ATTRIBUTION: &str =
    "WinFsp - Windows File System Proxy, Copyright (C) Bill Zissimopoulos";

/// WinFsp's repository, shown with [`ATTRIBUTION`].
pub const REPO_URL: &str = "https://github.com/winfsp/winfsp";

/// Registry keys under `HKEY_LOCAL_MACHINE` whose `InstallDir` value names
/// the WinFsp install directory, in lookup order. The installer is 32-bit,
/// so x64 installs land in `WOW6432Node`; ARM64 installs use the plain key.
pub const INSTALL_KEYS: [&str; 2] = ["SOFTWARE\\WOW6432Node\\WinFsp", "SOFTWARE\\WinFsp"];

/// Whether `value`, the value of [`DISABLE_ENV`], turns the backend off.
pub fn disabled_by(value: Option<&OsStr>) -> bool {
    value.is_some_and(|value| value.eq_ignore_ascii_case("off"))
}

/// The WinFsp DLL for a target architecture (`std::env::consts::ARCH`), as
/// installed in `<InstallDir>\bin`.
pub fn dll_name(arch: &str) -> Option<&'static str> {
    match arch {
        "x86_64" => Some("winfsp-x64.dll"),
        "x86" => Some("winfsp-x86.dll"),
        "aarch64" => Some("winfsp-a64.dll"),
        _ => None,
    }
}

/// A WinFsp version, as `FspVersion` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    /// Major version.
    pub major: u16,
    /// Minor version.
    pub minor: u16,
}

impl Version {
    /// Unpacks `FspVersion`'s value: major in the high 16 bits, minor in the
    /// low 16.
    pub fn from_packed(packed: u32) -> Self {
        Self {
            major: (packed >> 16) as u16,
            minor: (packed & 0xffff) as u16,
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Why the WinFsp backend is unavailable. Every case falls back to static
/// views and the CLI (D3); none is an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// [`DISABLE_ENV`] is `off`.
    Disabled,
    /// No `InstallDir` under any of [`INSTALL_KEYS`].
    NotInstalled,
    /// WinFsp looks installed, but its DLL did not load or initialise.
    NotLoadable {
        /// The DLL codetags tried.
        dll: PathBuf,
        /// What went wrong.
        reason: String,
    },
    /// No WinFsp build exists for this architecture.
    UnsupportedArch(&'static str),
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => write!(f, "disabled ({DISABLE_ENV}=off)"),
            Self::NotInstalled => write!(
                f,
                "not installed (no InstallDir under HKLM\\{})",
                INSTALL_KEYS[0]
            ),
            Self::NotLoadable { dll, reason } => {
                write!(f, "not loadable ({}: {reason})", dll.display())
            }
            Self::UnsupportedArch(arch) => write!(f, "not available for {arch}"),
        }
    }
}

/// The line `codetags doctor` prints when the backend is unavailable.
pub const FALLBACK: &str = "fallback: static views and the codetags CLI; install WinFsp \
     (https://winfsp.dev) for live mounts";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_off_disables() {
        assert!(disabled_by(Some(OsStr::new("off"))));
        assert!(disabled_by(Some(OsStr::new("OFF"))));
        assert!(!disabled_by(Some(OsStr::new("on"))));
        assert!(!disabled_by(Some(OsStr::new(""))));
        assert!(!disabled_by(None));
    }

    #[test]
    fn dll_names_follow_the_install_layout() {
        assert_eq!(dll_name("x86_64"), Some("winfsp-x64.dll"));
        assert_eq!(dll_name("aarch64"), Some("winfsp-a64.dll"));
        assert_eq!(dll_name("riscv64"), None);
    }

    #[test]
    fn versions_unpack_major_and_minor() {
        let version = Version::from_packed((2 << 16) | 1);
        assert_eq!(version, Version { major: 2, minor: 1 });
        assert_eq!(version.to_string(), "2.1");
    }

    #[test]
    fn reasons_name_what_to_check() {
        assert_eq!(
            Unavailable::Disabled.to_string(),
            "disabled (CODETAGS_WINFSP=off)"
        );
        assert!(
            Unavailable::NotInstalled
                .to_string()
                .starts_with("not installed")
        );
        let unloadable = Unavailable::NotLoadable {
            dll: PathBuf::from("winfsp-x64.dll"),
            reason: "missing".to_string(),
        };
        assert_eq!(
            unloadable.to_string(),
            "not loadable (winfsp-x64.dll: missing)"
        );
    }
}
