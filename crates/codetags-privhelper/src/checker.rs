//! Access checks made as the client's user, never as root (PLAN.md §2.8).
//!
//! For each client the helper starts a checker: this same executable, run
//! through `/proc/self/exe` with `--checker`, which drops to the client's
//! user, groups and supplementary groups before it reads a single request.
//! The helper then asks it, for each subscription and each event, whether
//! that user may see the path. Because the checker *is* that user, the
//! kernel applies every permission bit, ACL and LSM rule exactly as it
//! would for the client.
//!
//! A checker process is used rather than switching the helper's own
//! credentials: credentials are per thread in the kernel but glibc's
//! `setgroups` changes every thread, and the helper's fanotify thread must
//! keep root's. After dropping, the checker makes itself non-dumpable, so
//! the client's user cannot ptrace it into approving paths.
//!
//! Its protocol, over stdin and stdout, uses the helper's frames
//! ([`codetags_watch::privhelper::proto`]):
//!
//! | Tag | Body | Reply |
//! |---|---|---|
//! | (none) | | `K` once privileges are dropped, or `N` and a reason |
//! | `R` | root path | `Y` and the canonical root, or `N` and a reason |
//! | `P` | `u32` root length, root, path | `Y` or `N` |

use std::ffi::{CString, OsStr};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use codetags_watch::privhelper::proto::{read_frame, write_frame};
use nix::fcntl::AtFlags;
use nix::unistd::{AccessFlags, Gid, Uid, User};

/// The argument that starts a checker.
pub const CHECKER_ARG: &str = "--checker";

/// Who a checker runs as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The user.
    pub uid: u32,
    /// The primary group.
    pub gid: u32,
    /// The supplementary groups, from the group database.
    pub groups: Vec<u32>,
}

impl Identity {
    /// The identity of a client with `uid` and `gid` (from `SO_PEERCRED`),
    /// with the supplementary groups the group database gives that user
    /// (`getgrouplist`). A user missing from the database gets none beyond
    /// `gid`, which can only refuse more, never less.
    pub fn of(uid: u32, gid: u32) -> Self {
        let groups = User::from_uid(Uid::from_raw(uid))
            .ok()
            .flatten()
            .and_then(|user| CString::new(user.name).ok())
            .and_then(|name| nix::unistd::getgrouplist(&name, Gid::from_raw(gid)).ok())
            .map(|groups| groups.into_iter().map(Gid::as_raw).collect())
            .unwrap_or_else(|| vec![gid]);
        Self { uid, gid, groups }
    }

    fn args(&self) -> Vec<String> {
        let groups: Vec<String> = self.groups.iter().map(u32::to_string).collect();
        vec![
            CHECKER_ARG.to_string(),
            self.uid.to_string(),
            self.gid.to_string(),
            groups.join(","),
        ]
    }

    fn parse(args: &[String]) -> Result<Self, String> {
        let [uid, gid, groups] = args else {
            return Err(format!("{CHECKER_ARG} takes UID GID GROUPS"));
        };
        let number = |s: &str| s.parse::<u32>().map_err(|e| format!("{s:?}: {e}"));
        Ok(Self {
            uid: number(uid)?,
            gid: number(gid)?,
            groups: groups
                .split(',')
                .filter(|g| !g.is_empty())
                .map(number)
                .collect::<Result<_, _>>()?,
        })
    }
}

/// A running checker, owned by one client's connection.
#[derive(Debug)]
pub struct Checker {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

fn frame_error(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("checker: {what}"))
}

impl Checker {
    /// Starts a checker for `who` from `exe` (in production
    /// `/proc/self/exe`, so it is this very binary even if the file on disk
    /// has been replaced), and waits until it has dropped privileges.
    pub fn spawn(exe: &Path, who: &Identity) -> io::Result<Self> {
        let mut child = Command::new(exe)
            .args(who.args())
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return Err(frame_error("no pipes"));
        };
        let mut checker = Self {
            child,
            stdin,
            stdout,
        };
        match read_frame(&mut checker.stdout)? {
            Some((b'K', _)) => Ok(checker),
            Some((b'N', reason)) => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "the checker could not become uid {}: {}",
                    who.uid,
                    String::from_utf8_lossy(&reason)
                ),
            )),
            _ => Err(frame_error("no ready signal")),
        }
    }

    fn ask(&mut self, tag: u8, body: &[u8]) -> io::Result<(u8, Vec<u8>)> {
        write_frame(&mut self.stdin, tag, body)?;
        read_frame(&mut self.stdout)?.ok_or_else(|| frame_error("exited"))
    }

    /// Whether the user may watch `root`, which must be a directory they can
    /// list. Returns it canonicalized as that user.
    pub fn check_root(&mut self, root: &Path) -> io::Result<Result<PathBuf, String>> {
        Ok(match self.ask(b'R', root.as_os_str().as_bytes())? {
            (b'Y', canonical) => Ok(PathBuf::from(OsStr::from_bytes(&canonical))),
            (b'N', reason) => Err(String::from_utf8_lossy(&reason).into_owned()),
            _ => return Err(frame_error("bad reply")),
        })
    }

    /// Whether the user may learn of `path`, under the accepted `root`.
    pub fn may_see(&mut self, root: &Path, path: &Path) -> io::Result<bool> {
        let root = root.as_os_str().as_bytes();
        let mut body = u32::try_from(root.len())
            .map_err(|_| frame_error("root too long"))?
            .to_le_bytes()
            .to_vec();
        body.extend_from_slice(root);
        body.extend_from_slice(path.as_os_str().as_bytes());
        match self.ask(b'P', &body)? {
            (b'Y', _) => Ok(true),
            (b'N', _) => Ok(false),
            _ => Err(frame_error("bad reply")),
        }
    }
}

impl Drop for Checker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Drops to `who`: supplementary groups, then group, then user, all real,
/// effective and saved IDs; then checks root cannot be regained, and makes
/// the process non-dumpable.
///
/// Run unprivileged (as the tests do), it can only be the user it already
/// is, and keeps its own groups.
fn drop_privileges(who: &Identity) -> Result<(), String> {
    if !Uid::effective().is_root() {
        let me = Uid::current();
        if Uid::effective() != me || me.as_raw() != who.uid {
            return Err(format!("not root, so it cannot become uid {}", who.uid));
        }
        return nix::sys::prctl::set_dumpable(false).map_err(|e| format!("prctl: {e}"));
    }
    let groups: Vec<Gid> = who.groups.iter().copied().map(Gid::from_raw).collect();
    let (uid, gid) = (Uid::from_raw(who.uid), Gid::from_raw(who.gid));
    nix::unistd::setgroups(&groups).map_err(|e| format!("setgroups: {e}"))?;
    nix::unistd::setresgid(gid, gid, gid).map_err(|e| format!("setresgid: {e}"))?;
    nix::unistd::setresuid(uid, uid, uid).map_err(|e| format!("setresuid: {e}"))?;
    let ids = nix::unistd::getresuid().map_err(|e| format!("getresuid: {e}"))?;
    if (ids.real, ids.effective, ids.saved) != (uid, uid, uid) {
        return Err("the user IDs did not change".into());
    }
    if who.uid != 0
        && nix::unistd::setresuid(Uid::from_raw(0), Uid::from_raw(0), Uid::from_raw(0)).is_ok()
    {
        return Err("root could be regained after dropping it".into());
    }
    nix::sys::prctl::set_dumpable(false).map_err(|e| format!("prctl: {e}"))
}

/// Whether the current user may list the directory `dir`: read and search
/// permission on it, and search permission on every directory above it.
fn can_list(dir: &Path) -> Result<(), String> {
    nix::unistd::faccessat(
        nix::fcntl::AT_FDCWD,
        dir,
        AccessFlags::R_OK | AccessFlags::X_OK,
        AtFlags::AT_EACCESS,
    )
    .map_err(|errno| format!("{}: {}", dir.display(), errno.desc()))
}

/// Answers an `R` request.
fn check_root(root: &Path) -> Result<PathBuf, String> {
    if !root.is_absolute() {
        return Err(format!("{} is not an absolute path", root.display()));
    }
    let canonical =
        std::fs::canonicalize(root).map_err(|error| format!("{}: {error}", root.display()))?;
    if !canonical.is_dir() {
        return Err(format!("{} is not a directory", canonical.display()));
    }
    can_list(&canonical)?;
    Ok(canonical)
}

/// Answers a `P` request: `path` must be under `root`, and the user must be
/// able to list the directory that holds it (or, for the root itself, the
/// root). A name in a directory the user cannot list is never shown.
fn may_see(root: &Path, path: &Path) -> bool {
    if !path.starts_with(root) {
        return false;
    }
    let dir = if path == root {
        root
    } else {
        match path.parent() {
            Some(parent) => parent,
            None => return false,
        }
    };
    can_list(dir).is_ok()
}

fn answer(body: &[u8]) -> (u8, Vec<u8>) {
    let Some((len, rest)) = body.split_first_chunk::<4>() else {
        return (b'N', Vec::new());
    };
    let len = u32::from_le_bytes(*len) as usize;
    let (Some(root), Some(path)) = (rest.get(..len), rest.get(len..)) else {
        return (b'N', Vec::new());
    };
    let (root, path) = (
        Path::new(OsStr::from_bytes(root)),
        Path::new(OsStr::from_bytes(path)),
    );
    if may_see(root, path) {
        (b'Y', Vec::new())
    } else {
        (b'N', Vec::new())
    }
}

/// The checker's `main`: `args` follow [`CHECKER_ARG`]. Returns the exit
/// status.
pub fn run(args: &[String]) -> i32 {
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let ready = Identity::parse(args).and_then(|who| drop_privileges(&who));
    let sent = match &ready {
        Ok(()) => write_frame(&mut stdout, b'K', &[]),
        Err(reason) => write_frame(&mut stdout, b'N', reason.as_bytes()),
    };
    if ready.is_err() || sent.is_err() {
        return 1;
    }
    loop {
        let (tag, body) = match read_frame(&mut stdin) {
            Ok(Some(frame)) => frame,
            Ok(None) => return 0,
            Err(_) => return 1,
        };
        let reply = match tag {
            b'R' => match check_root(Path::new(OsStr::from_bytes(&body))) {
                Ok(canonical) => (b'Y', canonical.as_os_str().as_bytes().to_vec()),
                Err(reason) => (b'N', reason.into_bytes()),
            },
            b'P' => answer(&body),
            _ => (b'N', b"unknown request".to_vec()),
        };
        if write_frame(&mut stdout, reply.0, &reply.1).is_err() || stdout.flush().is_err() {
            return 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_round_trip_through_arguments() {
        let who = Identity {
            uid: 1000,
            gid: 1000,
            groups: vec![1000, 27, 100],
        };
        let args = who.args();
        assert_eq!(args[0], CHECKER_ARG);
        assert_eq!(Identity::parse(&args[1..]).unwrap(), who);
        assert!(Identity::parse(&args[1..3]).is_err());
        assert!(Identity::parse(&["x".into(), "1".into(), "".into()]).is_err());
    }

    #[test]
    fn the_current_user_has_a_group_list() {
        let who = Identity::of(Uid::current().as_raw(), Gid::current().as_raw());
        assert!(who.groups.contains(&Gid::current().as_raw()));
    }

    #[test]
    fn roots_must_be_absolute_listable_directories() {
        let dir = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(check_root(dir.path()).unwrap(), canonical);
        assert!(check_root(Path::new("relative")).is_err());
        std::fs::write(dir.path().join("f"), "").unwrap();
        assert!(
            check_root(&dir.path().join("f"))
                .unwrap_err()
                .contains("not a directory")
        );
        assert!(check_root(&dir.path().join("absent")).is_err());
    }

    #[test]
    fn a_path_is_seen_only_under_its_root_in_a_listable_directory() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let closed = root.join("closed");
        std::fs::create_dir(&closed).unwrap();
        assert!(may_see(&root, &root));
        assert!(may_see(&root, &root.join("a.rs")));
        assert!(may_see(&root, &closed));
        assert!(may_see(&root, &closed.join("x")));
        assert!(!may_see(&root, Path::new("/etc/passwd")));
        std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o300)).unwrap();
        let listable = can_list(&closed).is_ok();
        std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o700)).unwrap();
        // Root ignores permission bits, so this only bites unprivileged.
        if !Uid::effective().is_root() {
            assert!(
                !listable,
                "a directory without read permission is not listable"
            );
        }
    }

    #[test]
    fn path_requests_decode() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let mut body = (root.as_os_str().len() as u32).to_le_bytes().to_vec();
        body.extend_from_slice(root.as_os_str().as_bytes());
        body.extend_from_slice(root.join("a").as_os_str().as_bytes());
        assert_eq!(answer(&body).0, b'Y');
        assert_eq!(answer(&[1, 0]).0, b'N');
        assert_eq!(answer(&[9, 0, 0, 0, b'/']).0, b'N');
    }
}
