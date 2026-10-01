//! Starting `lspmux server` on demand (PLAN.md D21): the same way on every
//! OS, detached from the session that started it, and once only.
//!
//! lspmux's `Listener::bind` deletes whatever is at a Unix socket path
//! before binding (V36), so two daemons started at once would both run, and
//! the first would be orphaned. The start therefore holds an exclusive lock
//! file, checks again that nothing answers, starts the daemon, and waits for
//! it to answer before releasing the lock (V117).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(not(windows))]
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::lspmux::{self, Address};

/// How long a started daemon may take to answer.
const START_TIMEOUT: Duration = Duration::from_secs(20);
/// The line the shim appends to the daemon's log each time it starts one.
pub const STARTED_MARK: &str = "codetags-lsp: started lspmux server";

/// Whether something accepts connections at `address`.
pub fn is_answering(address: &Address) -> bool {
    match address {
        Address::Tcp(ip, port) => std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::new(*ip, *port),
            Duration::from_secs(1),
        )
        .is_ok(),
        #[cfg(unix)]
        Address::Unix(path) => std::os::unix::net::UnixStream::connect(path).is_ok(),
        #[cfg(not(unix))]
        Address::Unix(_) => false,
    }
}

/// The directory for the daemon's lock, pid and log files: the socket's own
/// private directory, or for TCP a per-user state directory
/// (`%LOCALAPPDATA%\codetags\lspmux` on Windows, `~/.local/state/codetags`
/// elsewhere), falling back to the temporary directory.
pub fn state_dir(address: &Address) -> PathBuf {
    if let Address::Unix(path) = address
        && let Some(dir) = path.parent()
    {
        return dir.to_path_buf();
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(|dir| PathBuf::from(dir).join("codetags").join("lspmux"))
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| PathBuf::from(home).join(".local").join("state"))
            })
            .map(|dir| dir.join("codetags"))
    };
    base.unwrap_or_else(|| std::env::temp_dir().join("codetags-lspmux"))
}

/// The base name of the state files for `address`: `lspmux` for a socket
/// (its directory is the address), `lspmux-<port>` for TCP.
fn state_name(address: &Address) -> String {
    match address {
        Address::Tcp(_, port) => format!("lspmux-{port}"),
        Address::Unix(_) => "lspmux".to_string(),
    }
}

/// The daemon's pid file for `address` (the last daemon a shim started).
pub fn pid_file(address: &Address) -> PathBuf {
    state_dir(address).join(format!("{}.pid", state_name(address)))
}

/// The daemon's log for `address`: its stderr, and a [`STARTED_MARK`] line
/// for each start.
pub fn log_file(address: &Address) -> PathBuf {
    state_dir(address).join(format!("{}.log", state_name(address)))
}

/// How [`ensure_running`] found the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Daemon {
    /// It was already answering.
    Running,
    /// This call started it, with this process id.
    Started(u32),
}

/// Makes sure an lspmux daemon answers at `address`, starting `lspmux
/// server` if nothing does.
pub fn ensure_running(lspmux_binary: &Path, address: &Address) -> Result<Daemon, String> {
    if is_answering(address) {
        return Ok(Daemon::Running);
    }
    let dir = state_dir(address);
    lspmux::ensure_private_dir(&dir)?;
    let lock_path = dir.join(format!("{}.lock", state_name(address)));
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("cannot open {}: {error}", lock_path.display()))?;
    lock.lock()
        .map_err(|error| format!("cannot lock {}: {error}", lock_path.display()))?;
    // Another shim may have started it while this one waited for the lock.
    let result = if is_answering(address) {
        Ok(Daemon::Running)
    } else {
        start(lspmux_binary, address)
    };
    drop(lock);
    result
}

fn start(lspmux_binary: &Path, address: &Address) -> Result<Daemon, String> {
    let log_path = log_file(address);
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|error| format!("cannot open {}: {error}", log_path.display()))?;
    let mut daemon = spawn_detached(lspmux_binary, &log)
        .map_err(|error| format!("cannot start {} server: {error}", lspmux_binary.display()))?;
    let pid = daemon.pid;
    let _ = writeln!(log, "{STARTED_MARK} (pid {pid}) on {address}");
    let pid_path = pid_file(address);
    std::fs::write(&pid_path, format!("{pid}\n"))
        .map_err(|error| format!("cannot write {}: {error}", pid_path.display()))?;
    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        if is_answering(address) {
            return Ok(Daemon::Started(pid));
        }
        if let Some(Ok(Some(status))) = daemon.child.as_mut().map(std::process::Child::try_wait) {
            return Err(format!(
                "lspmux server exited ({status}) before answering on {address}; see {}",
                log_path.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err(format!(
        "lspmux server did not answer on {address} within {START_TIMEOUT:?}; see {}",
        log_path.display()
    ))
}

/// A started daemon: its process id, and on Unix its handle.
struct Detached {
    pid: u32,
    child: Option<std::process::Child>,
}

/// Starts `lspmux server` so it outlives this process and its session, with
/// its stderr appended to `log`. On Unix it gets its own process group, so
/// the editor's signals to its group miss it; Rust closes every other
/// descriptor in the child.
#[cfg(unix)]
fn spawn_detached(lspmux_binary: &Path, log: &std::fs::File) -> std::io::Result<Detached> {
    use std::os::unix::process::CommandExt;

    let child = Command::new(lspmux_binary)
        .arg("server")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log.try_clone()?)
        .process_group(0)
        .spawn()?;
    Ok(Detached {
        pid: child.id(),
        child: Some(child),
    })
}

/// On Windows, `std::process::Command` always lets the child inherit every
/// inheritable handle of this process (its `inherit_handles(false)` is
/// unstable), including the pipes the editor gave this shim. A daemon that
/// outlives the shim would hold them open, so the editor would never see the
/// server's output end. So the daemon is created with `CreateProcessW` and no
/// inherited handles at all, detached and in its own process group, and out
/// of the editor's job object when the job allows it. It has no standard
/// handles, so its own log output is lost; the start line still goes to the
/// log. Its environment is this process's.
#[cfg(windows)]
fn spawn_detached(lspmux_binary: &Path, _log: &std::fs::File) -> std::io::Result<Detached> {
    windows_spawn::detached(lspmux_binary, "server").map(|pid| Detached { pid, child: None })
}

/// Neither Unix nor Windows: a plain spawn.
#[cfg(not(any(unix, windows)))]
fn spawn_detached(lspmux_binary: &Path, log: &std::fs::File) -> std::io::Result<Detached> {
    let child = Command::new(lspmux_binary)
        .arg("server")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log.try_clone()?)
        .spawn()?;
    Ok(Detached {
        pid: child.id(),
        child: Some(child),
    })
}

/// `CreateProcessW` without handle inheritance: this crate's only unsafe
/// code. The unsafe part is the one call and closing the two handles it
/// returns.
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_spawn {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CreateProcessW, DETACHED_PROCESS,
        PROCESS_INFORMATION, STARTUPINFOW,
    };
    use windows::core::{PCWSTR, PWSTR};

    /// Starts `program argument` detached, inheriting no handles; its pid.
    pub(super) fn detached(program: &Path, argument: &str) -> std::io::Result<u32> {
        let wide = |text: &std::ffi::OsStr| -> Vec<u16> { text.encode_wide().chain([0]).collect() };
        let application = wide(program.as_os_str());
        // The program's path quoted (no quotes can occur in a Windows path),
        // then the argument, which needs none.
        let mut command_line: Vec<u16> =
            wide(format!("\"{}\" {argument}", program.display()).as_ref());
        let startup = STARTUPINFOW {
            cb: u32::try_from(std::mem::size_of::<STARTUPINFOW>()).unwrap_or(u32::MAX),
            ..Default::default()
        };
        let flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
        let mut last = None;
        for flags in [flags | CREATE_BREAKAWAY_FROM_JOB, flags] {
            let mut info = PROCESS_INFORMATION::default();
            // SAFETY: every pointer is to a live, NUL-terminated buffer or
            // struct owned by this frame; `command_line` is mutable, as the
            // call requires; no handles are inherited.
            let created = unsafe {
                CreateProcessW(
                    PCWSTR(application.as_ptr()),
                    Some(PWSTR(command_line.as_mut_ptr())),
                    None,
                    None,
                    false,
                    flags,
                    None,
                    PCWSTR::null(),
                    &startup,
                    &mut info,
                )
            };
            match created {
                Ok(()) => {
                    // SAFETY: both handles were just returned by
                    // CreateProcessW and are closed once.
                    unsafe {
                        let _ = CloseHandle(info.hThread);
                        let _ = CloseHandle(info.hProcess);
                    }
                    return Ok(info.dwProcessId);
                }
                // A job that forbids breakaway refuses the first attempt.
                Err(error) => last = Some(error),
            }
        }
        Err(std::io::Error::other(last.map_or_else(
            || "CreateProcessW failed".to_string(),
            |error| error.to_string(),
        )))
    }
}
