//! The helper binary run unprivileged: what can be tested without root.
//!
//! Unprivileged, fanotify_init with FAN_REPORT_FID succeeds (Linux 5.13+,
//! V86), so the helper starts and serves the protocol, but every
//! filesystem mark fails with EPERM, so every subscription is refused. The
//! checker works for the user it already is. The privileged paths run in
//! features/watch/privhelper.feature, in CI's privileged-linux job.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use codetags_watch::privhelper::proto::{read_frame, write_frame};
use codetags_watch::privhelper::{Client, HelperError, HelperSocket};
use codetags_watch::{WatchConfig, Watcher};

const HELPER: &str = env!("CARGO_BIN_EXE_codetags-privhelper");

fn ids() -> (String, String) {
    let id = |flag: &str| {
        let out = Command::new("id").arg(flag).output().unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    (id("-u"), id("-g"))
}

fn checker(uid: &str, gid: &str) -> Child {
    Command::new(HELPER)
        .args(["--checker", uid, gid, gid])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap()
}

fn ask(child: &mut Child, tag: u8, body: &[u8]) -> (u8, Vec<u8>) {
    let stdin = child.stdin.as_mut().unwrap();
    write_frame(stdin, tag, body).unwrap();
    stdin.flush().unwrap();
    read_frame(child.stdout.as_mut().unwrap()).unwrap().unwrap()
}

/// A `P` request: is `path` under `root` visible, where the event came
/// from the directory `held_by`?
fn path_request(root: &Path, held_by: &Path, path: &Path) -> Vec<u8> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(held_by).unwrap();
    let mut body = (root.as_os_str().len() as u32).to_le_bytes().to_vec();
    body.extend_from_slice(root.as_os_str().as_bytes());
    body.push(b'D');
    body.extend_from_slice(&meta.dev().to_le_bytes());
    body.extend_from_slice(&meta.ino().to_le_bytes());
    body.extend_from_slice(path.as_os_str().as_bytes());
    body
}

#[test]
fn the_checker_answers_as_the_current_user() {
    let (uid, gid) = ids();
    let mut child = checker(&uid, &gid);
    let ready = read_frame(child.stdout.as_mut().unwrap()).unwrap().unwrap();
    assert_eq!(ready.0, b'K', "{}", String::from_utf8_lossy(&ready.1));

    let dir = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    let (tag, body) = ask(&mut child, b'R', dir.path().as_os_str().as_bytes());
    assert_eq!(tag, b'Y');
    assert_eq!(Path::new(std::ffi::OsStr::from_bytes(&body)), canonical);
    let (tag, reason) = ask(&mut child, b'R', b"/definitely/not/here");
    assert_eq!(tag, b'N');
    assert!(!reason.is_empty());

    let held_by_root = path_request(&canonical, &canonical, &canonical.join("a.rs"));
    assert_eq!(ask(&mut child, b'P', &held_by_root).0, b'Y');
    let outside = path_request(&canonical, Path::new("/etc"), Path::new("/etc/passwd"));
    assert_eq!(ask(&mut child, b'P', &outside).0, b'N');
    // Threat model F1: an event recorded as coming from one directory is
    // refused when its path leads to another, though both are listable.
    let other = canonical.join("other");
    std::fs::create_dir(&other).unwrap();
    let elsewhere = path_request(&canonical, &other, &canonical.join("a.rs"));
    assert_eq!(ask(&mut child, b'P', &elsewhere).0, b'N');

    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
}

#[test]
fn an_unprivileged_checker_refuses_to_be_anyone_else() {
    let (uid, _) = ids();
    if uid == "0" {
        return; // Root may become anyone; this checks the unprivileged path.
    }
    let mut child = checker("0", "0");
    let (tag, reason) = read_frame(child.stdout.as_mut().unwrap()).unwrap().unwrap();
    assert_eq!(tag, b'N');
    assert!(String::from_utf8_lossy(&reason).contains("cannot become uid 0"));
    assert!(!child.wait().unwrap().success());
}

/// The helper, started unprivileged on a socket in a scratch directory.
struct Helper {
    child: Child,
    socket: PathBuf,
    _dir: tempfile::TempDir,
}

impl Helper {
    fn start() -> Self {
        Self::start_with(&[])
    }

    /// Starts the helper with `args` after `--socket`. Unprivileged, its
    /// socket's directory may be owned by this user (not group- or
    /// other-writable), since no one else can change it.
    fn start_with(args: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("run/privhelper.sock");
        let child = Command::new(HELPER)
            .arg("--socket")
            .arg(&socket)
            .args(args)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut helper = Self {
            child,
            socket,
            _dir: dir,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !helper.socket.exists() {
            if let Some(status) = helper.child.try_wait().unwrap() {
                panic!("the helper exited with {status}: {}", helper.stderr());
            }
            assert!(Instant::now() < deadline, "no socket after 10 s");
            std::thread::sleep(Duration::from_millis(20));
        }
        helper
    }

    fn stderr(&mut self) -> String {
        let mut text = String::new();
        if let Some(mut err) = self.child.stderr.take() {
            let _ = err.read_to_string(&mut text);
        }
        text
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn unprivileged_the_helper_serves_but_refuses_every_root() {
    let helper = Helper::start();
    let mut client = Client::connect(&helper.socket).unwrap();
    assert_eq!(client.backend(), "fanotify");
    let project = tempfile::tempdir().unwrap();
    let error = client.subscribe(project.path()).unwrap_err();
    let HelperError::Refused { reason, .. } = &error else {
        panic!("expected a refusal, got {error}");
    };
    assert!(reason.contains("fanotify cannot mark"), "{reason}");
    // A root that does not exist is refused by the checker.
    let error = client
        .subscribe(Path::new("/definitely/not/here"))
        .unwrap_err();
    assert!(matches!(error, HelperError::Refused { .. }), "{error}");
}

#[test]
fn a_refusing_helper_leaves_the_watcher_on_notify() {
    let helper = Helper::start();
    let project = tempfile::tempdir().unwrap();
    let config = WatchConfig {
        helper: HelperSocket::At(helper.socket.clone()),
        ..WatchConfig::default()
    };
    let watcher = Watcher::start(project.path(), config).unwrap();
    let mode = watcher.mode();
    assert_eq!(mode.source(), "notify");
    assert!(mode.to_string().contains("refused"), "{mode}");
    std::fs::write(project.path().join("a.rs"), "x").unwrap();
    let batch = watcher
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .expect("notify reports the write");
    assert_eq!(batch.changes[0].path, Path::new("a.rs"));
}

#[test]
fn the_helper_exits_when_its_socket_is_removed() {
    let mut helper = Helper::start();
    std::fs::remove_file(&helper.socket).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = helper.child.try_wait().unwrap() {
            assert!(status.success(), "{}", helper.stderr());
            assert!(helper.stderr().contains("removed or replaced"));
            return;
        }
        assert!(Instant::now() < deadline, "still running 10 s later");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_non_socket_file_is_never_replaced() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    // A directory the helper accepts, whatever this process's umask.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = dir.path().join("privhelper.sock");
    std::fs::write(&path, "precious").unwrap();
    let out = Command::new(HELPER)
        .arg("--socket")
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a socket"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "precious");
}

/// Runs the helper to completion, expecting it to refuse to start.
fn refused_start(socket: &Path, args: &[&str]) -> (Option<i32>, String) {
    let out = Command::new(HELPER)
        .arg("--socket")
        .arg(socket)
        .args(args)
        .output()
        .unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Threat model F2: the socket is created with its mode, by a umask around
/// `bind`, not by a later chmod of its path.
#[test]
fn the_socket_is_created_with_mode_0666() {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    let helper = Helper::start();
    let meta = std::fs::symlink_metadata(&helper.socket).unwrap();
    assert!(meta.file_type().is_socket());
    assert_eq!(meta.permissions().mode() & 0o777, 0o666);
}

#[test]
fn a_stale_socket_is_replaced_but_a_live_one_is_not() {
    let mut first = Helper::start();
    let (code, stderr) = refused_start(&first.socket, &[]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("another helper is listening"), "{stderr}");
    // Killed, the first helper leaves its socket behind, stale.
    first.child.kill().unwrap();
    first.child.wait().unwrap();
    assert!(first.socket.exists());
    let mut second = Command::new(HELPER)
        .arg("--socket")
        .arg(&first.socket)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Client::connect(&first.socket).is_err() {
        assert!(
            second.try_wait().unwrap().is_none(),
            "the second helper exited"
        );
        assert!(
            Instant::now() < deadline,
            "the second helper never answered"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = second.kill();
    let _ = second.wait();
}

/// Threat model F2: a directory anyone else can write is refused, and the
/// error names it.
#[test]
fn a_socket_directory_others_can_write_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    for mode in [0o777, 0o1777, 0o770] {
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(mode)).unwrap();
        let (code, stderr) = refused_start(&dir.path().join("privhelper.sock"), &[]);
        assert_eq!(code, Some(1), "{stderr}");
        assert!(stderr.contains("writable by group or other"), "{stderr}");
        assert!(
            stderr.contains(&dir.path().display().to_string()),
            "{stderr}"
        );
        assert!(!dir.path().join("privhelper.sock").exists());
    }
}

/// Threat model F2: the socket's directory may not be a symlink, even to a
/// directory that would itself be accepted.
#[test]
fn a_symlinked_socket_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let (code, stderr) = refused_start(&link.join("privhelper.sock"), &[]);
    assert_eq!(code, Some(1), "{stderr}");
    assert!(stderr.contains("symbolic link"), "{stderr}");
    assert!(!real.join("privhelper.sock").exists());
}

/// Connects until `ok` holds for the result, for up to 10 s: a connection's
/// place is freed when the helper notices it hung up.
fn connect_until(socket: &Path, ok: impl Fn(&Result<Client, HelperError>) -> bool) -> Client {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let result = Client::connect(socket);
        if ok(&result) {
            return result.unwrap();
        }
        assert!(Instant::now() < deadline, "still {:?}", result.err());
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The reason a connection was turned away.
fn rejected(socket: &Path) -> String {
    match Client::connect(socket) {
        Err(HelperError::Handshake(reason)) => reason,
        Err(other) => panic!("expected a rejection, got {other}"),
        Ok(_) => panic!("expected a rejection, got a connection"),
    }
}

#[test]
fn connections_over_the_total_limit_are_rejected() {
    let helper = Helper::start_with(&["--max-connections", "2"]);
    let first = Client::connect(&helper.socket).unwrap();
    let _second = Client::connect(&helper.socket).unwrap();
    let reason = rejected(&helper.socket);
    assert!(reason.contains("maximum of 2 connections"), "{reason}");
    drop(first);
    connect_until(&helper.socket, Result::is_ok);
}

#[test]
fn connections_over_the_per_uid_limit_are_rejected() {
    let helper = Helper::start_with(&["--max-connections-per-uid", "1"]);
    let first = Client::connect(&helper.socket).unwrap();
    let reason = rejected(&helper.socket);
    let (uid, _) = ids();
    assert!(reason.contains(&format!("uid {uid}")), "{reason}");
    drop(first);
    connect_until(&helper.socket, Result::is_ok);
}

#[test]
fn checkers_over_the_limit_are_rejected() {
    let helper = Helper::start_with(&["--max-checkers", "1"]);
    let first = Client::connect(&helper.socket).unwrap();
    let reason = rejected(&helper.socket);
    assert!(reason.contains("maximum of 1 checkers"), "{reason}");
    drop(first);
    connect_until(&helper.socket, Result::is_ok);
}

#[test]
fn limits_must_be_positive_numbers() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("privhelper.sock");
    for args in [["--queue", "0"], ["--max-checkers", "lots"]] {
        let (code, stderr) = refused_start(&socket, &args);
        assert_eq!(code, Some(2), "{stderr}");
        assert!(stderr.contains("positive number"), "{stderr}");
    }
    let (code, stderr) = refused_start(&socket, &["--max-subscriptions"]);
    assert_eq!(code, Some(2), "{stderr}");
}
