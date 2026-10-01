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

    let mut path = (canonical.as_os_str().len() as u32).to_le_bytes().to_vec();
    path.extend_from_slice(canonical.as_os_str().as_bytes());
    path.extend_from_slice(canonical.join("a.rs").as_os_str().as_bytes());
    assert_eq!(ask(&mut child, b'P', &path).0, b'Y');
    let mut outside = (canonical.as_os_str().len() as u32).to_le_bytes().to_vec();
    outside.extend_from_slice(canonical.as_os_str().as_bytes());
    outside.extend_from_slice(b"/etc/passwd");
    assert_eq!(ask(&mut child, b'P', &outside).0, b'N');

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
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("run/privhelper.sock");
        let child = Command::new(HELPER)
            .arg("--socket")
            .arg(&socket)
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
    let dir = tempfile::tempdir().unwrap();
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
