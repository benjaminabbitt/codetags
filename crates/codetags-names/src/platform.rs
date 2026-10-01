//! Target platforms and what each accepts as one file name.
//!
//! The Windows rules follow Microsoft's "Naming Files, Paths, and
//! Namespaces" (V34). Spike S4 (PLAN.md §8) will check what actually
//! reaches a WinFsp file system; until then these are the documented rules.

use std::fmt;
use std::str::FromStr;

/// An operating system a name may land on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    /// Linux.
    Linux,
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
}

impl Platform {
    /// Every supported platform (D1).
    pub const ALL: [Platform; 3] = [Platform::Linux, Platform::MacOs, Platform::Windows];

    /// The platform this binary was built for. Unix-likes other than macOS
    /// count as Linux.
    pub fn host() -> Self {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Linux
        }
    }

    /// The lower-case name used in features and messages.
    pub fn name(self) -> &'static str {
        match self {
            Platform::Linux => "linux",
            Platform::MacOs => "macos",
            Platform::Windows => "windows",
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Platform {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Platform::ALL
            .into_iter()
            .find(|p| p.name() == s)
            .ok_or_else(|| format!("unknown platform {s:?}: use linux, macos or windows"))
    }
}

/// The longest file name, in bytes of UTF-8, accepted on any platform.
///
/// Linux (`NAME_MAX`) and APFS allow 255 bytes; NTFS allows 255 UTF-16
/// code units, which is never fewer characters than 255 UTF-8 bytes. So
/// 255 bytes is the portable limit.
pub const MAX_NAME_BYTES: usize = 255;

/// Windows's reserved device names (V34). A name is reserved when the part
/// before its first `.`, with trailing spaces removed, is one of these,
/// ignoring ASCII case.
pub const WINDOWS_DEVICE_NAMES: [&str; 30] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "COM¹", "COM²", "COM³", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
    "LPT9", "LPT¹", "LPT²", "LPT³", "CONIN$", "CONOUT$",
];

/// Returns `true` for a character Windows forbids anywhere in a file name:
/// `< > : " / \ | ? *` and U+0000 through U+001F.
pub fn is_windows_reserved_char(c: char) -> bool {
    matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c < '\u{20}'
}

/// Returns `true` if `name` is, or begins with, a Windows device name: `CON`,
/// `nul.txt`, `Com1.tar.gz`, `AUX .md`.
pub fn is_windows_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    WINDOWS_DEVICE_NAMES
        .iter()
        .any(|device| device.eq_ignore_ascii_case(stem))
}

/// Returns `true` if `name` can be one file or directory name on
/// `platform`.
///
/// Every platform rejects the empty name, `.`, `..`, a `/`, a NUL, and
/// names over [`MAX_NAME_BYTES`]. Windows also rejects
/// [reserved characters](is_windows_reserved_char),
/// [device names](is_windows_device_name), and a trailing `.` or space.
pub fn is_legal_filename(name: &str, platform: Platform) -> bool {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > MAX_NAME_BYTES
        || name.contains(['/', '\0'])
    {
        return false;
    }
    match platform {
        Platform::Linux | Platform::MacOs => true,
        Platform::Windows => {
            !name.chars().any(is_windows_reserved_char)
                && !name.ends_with(['.', ' '])
                && !is_windows_device_name(name)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legal_on(name: &str) -> Vec<Platform> {
        Platform::ALL
            .into_iter()
            .filter(|p| is_legal_filename(name, *p))
            .collect()
    }

    const ALL: [Platform; 3] = Platform::ALL;
    const UNIX: [Platform; 2] = [Platform::Linux, Platform::MacOs];

    #[test]
    fn ordinary_names_are_legal_everywhere() {
        for name in [
            "a",
            ".hidden",
            "kind=function",
            "name~x",
            "50%25",
            "é",
            "console",
            "com10",
            "lpt",
            "a.con",
        ] {
            assert_eq!(legal_on(name), ALL, "{name:?}");
        }
    }

    #[test]
    fn some_names_are_legal_nowhere() {
        let long = "a".repeat(MAX_NAME_BYTES + 1);
        for name in ["", ".", "..", "a/b", "a\0b", long.as_str()] {
            assert_eq!(legal_on(name), vec![], "{name:?}");
        }
        assert_eq!(legal_on(&"a".repeat(MAX_NAME_BYTES)), ALL);
    }

    #[test]
    fn windows_reserved_characters() {
        for c in ['<', '>', ':', '"', '\\', '|', '?', '*', '\u{1}', '\u{1f}'] {
            assert_eq!(legal_on(&format!("a{c}b")), UNIX, "{c:?}");
        }
    }

    #[test]
    fn windows_trailing_dot_and_space() {
        for name in ["a.", "a ", "a. ", "..."] {
            assert_eq!(legal_on(name), UNIX, "{name:?}");
        }
    }

    #[test]
    fn windows_device_names() {
        for name in [
            "CON",
            "con",
            "Prn",
            "aux",
            "NUL",
            "nul.txt",
            "NUL.tar.gz",
            "com1",
            "COM9",
            "COM¹",
            "lpt3",
            "LPT³",
            "AUX .md",
            "conin$",
            "CONOUT$",
        ] {
            assert!(is_windows_device_name(name), "{name:?}");
            assert_eq!(legal_on(name), UNIX, "{name:?}");
        }
        for name in ["COM0x", "com10", "conx", "xcon", ".con", "a.nul"] {
            assert!(!is_windows_device_name(name), "{name:?}");
        }
    }

    #[test]
    fn platforms_parse_and_print() {
        for p in Platform::ALL {
            assert_eq!(p.to_string().parse::<Platform>(), Ok(p));
        }
        assert!("beos".parse::<Platform>().is_err());
    }
}
