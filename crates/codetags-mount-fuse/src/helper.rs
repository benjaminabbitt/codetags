//! Locating the `fusermount3` helper that mounts will use.
//!
//! `fuser` runs `$FUSERMOUNT_PATH` if set, otherwise the first `fusermount3`
//! on `PATH`. The helper must be setuid root for an unprivileged mount to
//! succeed; some environments shadow the real one with a copy that is not
//! (V12), and mounts then fail with EPERM.

use std::ffi::OsStr;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Name of the helper binary.
pub const FUSERMOUNT3: &str = "fusermount3";

/// The helper a mount would use, and whether it can mount unprivileged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Helper {
    /// Where the helper is.
    pub path: PathBuf,
    /// Whether it is owned by root with the setuid bit set.
    pub setuid_root: bool,
}

/// Finds the helper `fuser` would run, given the values of `FUSERMOUNT_PATH`
/// and `PATH`. Returns `None` if there is none.
pub fn locate(fusermount_path: Option<&OsStr>, path: Option<&OsStr>) -> Option<Helper> {
    let candidate = match fusermount_path {
        Some(explicit) => Some(PathBuf::from(explicit)),
        None => path.and_then(|path| {
            std::env::split_paths(path)
                .map(|dir| dir.join(FUSERMOUNT3))
                .find(|file| file.is_file())
        }),
    }?;
    let metadata = std::fs::metadata(&candidate).ok()?;
    Some(Helper {
        setuid_root: is_setuid_root(metadata.uid(), metadata.mode()),
        path: candidate,
    })
}

/// Finds the helper using this process's environment.
pub fn locate_from_env() -> Option<Helper> {
    locate(
        std::env::var_os("FUSERMOUNT_PATH").as_deref(),
        std::env::var_os("PATH").as_deref(),
    )
}

fn is_setuid_root(uid: u32, mode: u32) -> bool {
    const SETUID: u32 = 0o4000;
    uid == 0 && mode & SETUID != 0
}

/// Whether `path` is the helper's canonical install location on Debian/Ubuntu.
pub fn is_system_location(path: &Path) -> bool {
    path == Path::new("/usr/bin/fusermount3") || path == Path::new("/bin/fusermount3")
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;

    use super::{FUSERMOUNT3, is_setuid_root, locate};

    fn fake_helper(dir: &std::path::Path) -> std::path::PathBuf {
        let file = dir.join(FUSERMOUNT3);
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        file
    }

    #[test]
    fn setuid_root_needs_both_owner_and_bit() {
        assert!(is_setuid_root(0, 0o104755));
        assert!(!is_setuid_root(0, 0o100755));
        assert!(!is_setuid_root(1000, 0o104755));
    }

    #[test]
    fn the_first_match_on_path_wins() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let expected = fake_helper(first.path());
        fake_helper(second.path());
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();
        let helper = locate(None, Some(&path)).unwrap();
        assert_eq!(helper.path, expected);
        assert!(!helper.setuid_root);
    }

    #[test]
    fn fusermount_path_overrides_path() {
        let dir = tempfile::tempdir().unwrap();
        let explicit = fake_helper(dir.path());
        let helper = locate(Some(explicit.as_os_str()), Some(&OsString::new())).unwrap();
        assert_eq!(helper.path, explicit);
    }

    #[test]
    fn absent_helper_is_none() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(locate(None, Some(empty.path().as_os_str())), None);
    }
}
