//! The helper's socket, set up safely by construction (threat model F2).
//!
//! Root creates, replaces and binds the socket, so the directory it does
//! that in must be one nobody else can change: a real directory (not a
//! symlink), owned by root, and writable by nobody but its owner. The helper
//! refuses any other. `/run/codetags`, as the helper creates it or as
//! systemd's `RuntimeDirectory=` does, qualifies.
//!
//! Every step after the check works on the directory *opened*, not on its
//! path: the existing entry is examined and removed relative to the open
//! descriptor, and the socket is bound through `/proc/self/fd`, so a path
//! swapped after the check cannot redirect any of it. The socket gets its
//! mode as it is created, from a umask set around `bind`, so there is no
//! later `chmod` of a path. Startup is single-threaded, which makes the
//! umask safe to change.
//!
//! A helper that is not root (only tests run it so; it can mark nothing)
//! also accepts a directory owned by its own user, which no one else can
//! change either.

use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::ErrorKind;
use std::os::fd::AsRawFd;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Component, Path, PathBuf};

use nix::errno::Errno;
use nix::fcntl::{AtFlags, OFlag};
use nix::sys::stat::{Mode, SFlag};

/// The socket's mode: anyone may connect, and what each client receives is
/// decided per client.
const SOCKET_MODE: u32 = 0o666;

/// Opens `dir` if it may hold the helper's socket: a directory, not a
/// symlink, owned by root (or by `euid`, the helper's own user), and not
/// writable by group or other. The error names the directory and says why.
pub fn open_dir(dir: &Path, euid: u32) -> Result<File, String> {
    let shown = dir.display();
    let refuse = |why: String| {
        format!(
            "refusing to use {shown} for the socket: {why}. The socket's directory must be a \
             real directory owned by root and writable only by root, such as /run/codetags"
        )
    };
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(refuse("it is a symbolic link".into()));
        }
        Ok(_) => {}
        Err(error) => return Err(format!("{shown}: {error}")),
    }
    let flags = OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let fd = nix::fcntl::open(dir, flags, Mode::empty()).map_err(|errno| match errno {
        Errno::ELOOP => refuse("it is a symbolic link".into()),
        Errno::ENOTDIR => refuse("it is not a directory".into()),
        errno => format!("open {shown}: {}", errno.desc()),
    })?;
    let stat = nix::sys::stat::fstat(&fd).map_err(|errno| format!("stat {shown}: {errno}"))?;
    if SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT != SFlag::S_IFDIR {
        return Err(refuse("it is not a directory".into()));
    }
    if stat.st_uid != 0 && stat.st_uid != euid {
        return Err(refuse(format!("it is owned by uid {}", stat.st_uid)));
    }
    if stat.st_mode & 0o022 != 0 {
        return Err(refuse(format!(
            "it is writable by group or other (mode {:04o})",
            stat.st_mode & 0o7777
        )));
    }
    Ok(File::from(fd))
}

/// The socket's directory and its name in it. A path without a directory
/// is in the current one.
fn split(path: &Path) -> Result<(PathBuf, &OsStr), String> {
    let name = match path.components().next_back() {
        Some(Component::Normal(name)) => name,
        _ => return Err(format!("{} does not name a socket file", path.display())),
    };
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    Ok((dir, name))
}

/// Binds the socket at `path`, in a directory [`open_dir`] accepts, which
/// is created (mode 0755) if missing. An existing socket is replaced only
/// if nothing answers on it; any other file makes this fail. Returns the
/// listener and the socket's inode number, for the watchdog.
///
/// Changes the process's umask for the duration of `bind`, so call it only
/// while the process has one thread.
pub fn bind(path: &Path) -> Result<(UnixListener, u64), String> {
    let shown = path.display();
    let (dir_path, name) = split(path)?;
    if let Err(error) = fs::symlink_metadata(&dir_path)
        && error.kind() == ErrorKind::NotFound
    {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(&dir_path)
            .map_err(|error| format!("create {}: {error}", dir_path.display()))?;
    }
    let dir = open_dir(&dir_path, nix::unistd::Uid::effective().as_raw())?;
    // The socket's path through the open directory: nothing renamed after
    // the check above can change which directory this is.
    let through = PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd())).join(name);

    match nix::sys::stat::fstatat(&dir, name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(stat) if SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT == SFlag::S_IFSOCK => {
            if UnixStream::connect(&through).is_ok() {
                return Err(format!("another helper is listening on {shown}"));
            }
            nix::unistd::unlinkat(&dir, name, nix::unistd::UnlinkatFlags::NoRemoveDir)
                .map_err(|errno| format!("remove stale {shown}: {}", errno.desc()))?;
        }
        Ok(_) => {
            return Err(format!(
                "{shown} exists and is not a socket; not replacing it"
            ));
        }
        Err(Errno::ENOENT) => {}
        Err(errno) => return Err(format!("{shown}: {}", errno.desc())),
    }

    let umask = Mode::from_bits_truncate(0o777 & !SOCKET_MODE);
    let previous = nix::sys::stat::umask(umask);
    let bound = UnixListener::bind(&through);
    nix::sys::stat::umask(previous);
    let listener = bound.map_err(|error| format!("bind {shown}: {error}"))?;
    let inode = nix::sys::stat::fstatat(&dir, name, AtFlags::AT_SYMLINK_NOFOLLOW)
        .map_err(|errno| format!("{shown}: {}", errno.desc()))?
        .st_ino;
    Ok((listener, inode))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn me() -> u32 {
        nix::unistd::Uid::effective().as_raw()
    }

    fn chmod(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn a_directory_others_can_write_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for mode in [0o775, 0o757, 0o777, 0o1777] {
            chmod(dir.path(), mode);
            let error = open_dir(dir.path(), me()).unwrap_err();
            assert!(error.contains("writable by group or other"), "{error}");
            assert!(error.contains(&dir.path().display().to_string()), "{error}");
        }
        chmod(dir.path(), 0o755);
        open_dir(dir.path(), me()).unwrap();
    }

    #[test]
    fn a_symlinked_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir(&real).unwrap();
        chmod(&real, 0o755);
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let error = open_dir(&link, me()).unwrap_err();
        assert!(error.contains("symbolic link"), "{error}");
        assert!(error.contains(&link.display().to_string()), "{error}");
        open_dir(&real, me()).unwrap();
    }

    #[test]
    fn a_directory_someone_else_owns_is_refused() {
        if me() == 0 {
            return; // Root owns its scratch directories.
        }
        let dir = tempfile::tempdir().unwrap();
        chmod(dir.path(), 0o755);
        // As if the helper were root: the runner's directory is not root's.
        let error = open_dir(dir.path(), 0).unwrap_err();
        assert!(error.contains(&format!("owned by uid {}", me())), "{error}");
    }

    #[test]
    fn a_root_owned_0755_directory_is_accepted() {
        // `/` is owned by root, mode 0755, on every Linux host this runs
        // on, so this needs no privilege.
        let meta = fs::metadata("/").unwrap();
        assert_eq!(std::os::unix::fs::MetadataExt::uid(&meta), 0);
        open_dir(Path::new("/"), 0).unwrap();
        open_dir(Path::new("/"), 1000).unwrap();
        if me() == 0 {
            let dir = tempfile::tempdir().unwrap();
            chmod(dir.path(), 0o755);
            open_dir(dir.path(), 0).unwrap();
        }
    }

    #[test]
    fn a_file_is_not_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        fs::write(&file, "").unwrap();
        let error = open_dir(&file, me()).unwrap_err();
        assert!(error.contains("not a directory"), "{error}");
    }

    #[test]
    fn socket_paths_split_into_directory_and_name() {
        let (dir, name) = split(Path::new("/run/codetags/privhelper.sock")).unwrap();
        assert_eq!(
            (dir.as_path(), name),
            (Path::new("/run/codetags"), OsStr::new("privhelper.sock"))
        );
        let (dir, name) = split(Path::new("helper.sock")).unwrap();
        assert_eq!(
            (dir.as_path(), name),
            (Path::new("."), OsStr::new("helper.sock"))
        );
        assert!(split(Path::new("/")).is_err());
        assert!(split(Path::new("/run/..")).is_err());
    }
}
