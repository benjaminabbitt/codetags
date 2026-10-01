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
    let stderr = log
        .try_clone()
        .map_err(|error| format!("cannot use {}: {error}", log_path.display()))?;
    let mut command = Command::new(lspmux_binary);
    command
        .arg("server")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr);
    let mut child = spawn_detached(&mut command)
        .map_err(|error| format!("cannot start {} server: {error}", lspmux_binary.display()))?;
    let pid = child.id();
    let _ = writeln!(log, "{STARTED_MARK} (pid {pid}) on {address}");
    let pid_path = pid_file(address);
    std::fs::write(&pid_path, format!("{pid}\n"))
        .map_err(|error| format!("cannot write {}: {error}", pid_path.display()))?;
    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        if is_answering(address) {
            return Ok(Daemon::Started(pid));
        }
        if let Ok(Some(status)) = child.try_wait() {
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

/// Spawns `command` so it outlives this process and its session: its own
/// process group on Unix, so the editor's signals to its group miss it; on
/// Windows detached, in its own process group, and out of the editor's job
/// object when the job allows that.
#[cfg(unix)]
fn spawn_detached(command: &mut Command) -> std::io::Result<std::process::Child> {
    use std::os::unix::process::CommandExt;

    command.process_group(0).spawn()
}

/// See the Unix version.
#[cfg(windows)]
fn spawn_detached(command: &mut Command) -> std::io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    let flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    match command
        .creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB)
        .spawn()
    {
        Ok(child) => Ok(child),
        // The job forbids breakaway: stay in it.
        Err(_) => command.creation_flags(flags).spawn(),
    }
}

/// Neither Unix nor Windows: a plain spawn.
#[cfg(not(any(unix, windows)))]
fn spawn_detached(command: &mut Command) -> std::io::Result<std::process::Child> {
    command.spawn()
}
