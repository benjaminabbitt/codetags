//! Steps for `features/watch/privhelper.feature`: the optional privileged
//! helper (PLAN.md P4.4).
//!
//! The steps that need root use `sudo -n`, so they run only where sudo
//! needs no password (CI's privileged-linux job, `just test-privileged`).
//! The scenario, its watcher and its helper clients stay unprivileged.

use cucumber::given;

use codetags_watch::HelperSocket;

use crate::CodetagsWorld;

#[given("no privileged helper is listening")]
fn no_helper(world: &mut CodetagsWorld) {
    let socket = world.scratch().join("no-privhelper.sock");
    world.watch.helper = HelperSocket::At(socket);
}

#[cfg(unix)]
pub use unix::HelperState;

#[cfg(unix)]
mod unix {
    use std::fs::File;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
    use std::time::{Duration, Instant};

    use codetags_watch::HelperSocket;
    use codetags_watch::privhelper::proto::{EventKind, HelperEvent};
    use codetags_watch::privhelper::{Client, HelperError, Notice, Shutdown};
    use cucumber::{given, then, when};

    use crate::CodetagsWorld;

    /// How long the helper may take to create its socket, and to exit once
    /// it is removed.
    const STARTUP: Duration = Duration::from_secs(10);

    /// The helper binary, beside the `codetags` binary.
    fn helper_binary() -> PathBuf {
        let binary = crate::CODETAGS_BIN
            .get()
            .expect("run() set the codetags binary")
            .with_file_name("codetags-privhelper");
        assert!(
            binary.exists(),
            "{} is not built: `just test-privileged` builds it with \
             cargo build --workspace --bins",
            binary.display()
        );
        binary
    }

    /// A helper started with `sudo -n`. Dropping it removes its socket, which
    /// makes it exit; the unprivileged test cannot signal a root process.
    ///
    /// Its socket is in a directory root creates, owned by root and mode
    /// 0755, inside a scratch directory: the helper refuses a socket
    /// directory anyone but root can write (threat model F2).
    #[derive(Debug)]
    struct HelperProcess {
        child: Child,
        socket: PathBuf,
        log: PathBuf,
        /// The root-owned directory holding the socket.
        run: PathBuf,
        _dir: tempfile::TempDir,
    }

    impl HelperProcess {
        fn start(args: &[&str]) -> Self {
            let binary = helper_binary();
            let dir = tempfile::tempdir().expect("create the helper's directory");
            let run = dir.path().join("run");
            sudo(&[
                "install".as_ref(),
                "-d".as_ref(),
                "-m".as_ref(),
                "755".as_ref(),
                "-o".as_ref(),
                "0".as_ref(),
                "-g".as_ref(),
                "0".as_ref(),
                run.as_os_str(),
            ]);
            let socket = run.join("privhelper.sock");
            let log = dir.path().join("privhelper.log");
            let out = File::create(&log).expect("create the helper's log");
            let err = out.try_clone().expect("clone the log");
            let child = Command::new("sudo")
                .arg("-n")
                .arg(&binary)
                .arg("--socket")
                .arg(&socket)
                .args(args)
                .stdin(Stdio::null())
                .stdout(out)
                .stderr(err)
                .spawn()
                .expect("run sudo");
            let mut helper = Self {
                child,
                socket,
                log,
                run,
                _dir: dir,
            };
            let deadline = Instant::now() + STARTUP;
            while !helper.socket.exists() {
                if let Ok(Some(status)) = helper.child.try_wait() {
                    panic!(
                        "sudo -n codetags-privhelper exited with {status}: {}",
                        helper.log()
                    );
                }
                assert!(
                    Instant::now() < deadline,
                    "the helper made no socket within {STARTUP:?}: {}",
                    helper.log()
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            helper
        }

        fn log(&self) -> String {
            std::fs::read_to_string(&self.log).unwrap_or_default()
        }
    }

    impl Drop for HelperProcess {
        fn drop(&mut self) {
            // Only root can remove the socket from root's directory.
            let _ = Command::new("sudo")
                .args(["-n", "rm", "-f", "--"])
                .arg(&self.socket)
                .status();
            let deadline = Instant::now() + STARTUP;
            let mut exited = false;
            while Instant::now() < deadline {
                if let Ok(Some(_)) = self.child.try_wait() {
                    exited = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            if !exited {
                eprintln!(
                    "bdd: the helper outlived its socket; its log:\n{}",
                    self.log()
                );
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
            let _ = Command::new("sudo")
                .args(["-n", "rm", "-rf", "--"])
                .arg(&self.run)
                .status();
        }
    }

    /// State the helper steps share within one scenario.
    #[derive(Debug, Default)]
    pub struct HelperState {
        /// The subscribed client's notices, and a handle that ends them.
        /// Declared before `process`, so it hangs up first.
        notices: Option<(Receiver<Result<Notice, String>>, Shutdown)>,
        /// The running helper.
        process: Option<HelperProcess>,
        /// The canonical project root the client subscribed to.
        root: Option<PathBuf>,
        /// Every event the subscribed client received.
        seen: Vec<HelperEvent>,
        /// The PID of the last "another process".
        writer: Option<u32>,
        /// The outcome of the last "subscribes to".
        subscription: Option<Result<PathBuf, String>>,
        /// A subscribed client that is not reading yet.
        idle: Option<Client>,
        /// A helper that was expected to refuse to start: the directory its
        /// socket was in, its exit status and its output.
        refused_start: Option<(PathBuf, Option<i32>, String)>,
    }

    impl HelperState {
        fn socket(&self) -> &Path {
            &self
                .process
                .as_ref()
                .expect("an earlier step started the helper")
                .socket
        }

        fn root(&self) -> &Path {
            self.root
                .as_deref()
                .expect("an earlier step subscribed a client")
        }

        /// Receives notices for up to `timeout`, until `done` holds.
        fn collect(&mut self, timeout: Duration, done: impl Fn(&[HelperEvent]) -> bool) -> bool {
            let deadline = Instant::now() + timeout;
            loop {
                if done(&self.seen) {
                    return true;
                }
                let (notices, _) = self
                    .notices
                    .as_ref()
                    .expect("an earlier step subscribed a client");
                let left = deadline.saturating_duration_since(Instant::now());
                match notices.recv_timeout(left.min(Duration::from_millis(50))) {
                    Ok(Ok(Notice::Event(event))) => self.seen.push(event),
                    Ok(Ok(Notice::Overflow)) => {}
                    Ok(Err(error)) => panic!("the helper connection failed: {error}"),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => panic!("the helper hung up"),
                }
                if left.is_zero() {
                    return done(&self.seen);
                }
            }
        }
    }

    fn canonical(path: &Path) -> PathBuf {
        std::fs::canonicalize(path)
            .unwrap_or_else(|error| panic!("canonicalize {}: {error}", path.display()))
    }

    fn sudo(args: &[&std::ffi::OsStr]) {
        let status = Command::new("sudo")
            .arg("-n")
            .args(args)
            .status()
            .expect("run sudo");
        assert!(status.success(), "sudo -n {args:?} failed with {status}");
    }

    #[given("the privileged helper is running")]
    fn helper_running(world: &mut CodetagsWorld) {
        let process = HelperProcess::start(&[]);
        world.watch.helper = HelperSocket::At(process.socket.clone());
        world.privhelper.process = Some(process);
    }

    #[given(expr = "the privileged helper is running with {string}")]
    fn helper_running_with(world: &mut CodetagsWorld, args: String) {
        let args: Vec<&str> = args.split_whitespace().collect();
        let process = HelperProcess::start(&args);
        world.watch.helper = HelperSocket::At(process.socket.clone());
        world.privhelper.process = Some(process);
    }

    #[when("the privileged helper is started with its socket in a directory the runner owns")]
    fn helper_started_in_runner_dir(world: &mut CodetagsWorld) {
        let dir = world.scratch().to_path_buf();
        let socket = dir.join("privhelper.sock");
        let mut child = Command::new("sudo")
            .arg("-n")
            .arg(helper_binary())
            .arg("--socket")
            .arg(&socket)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run sudo");
        let deadline = Instant::now() + STARTUP;
        while child.try_wait().ok().flatten().is_none() {
            if Instant::now() >= deadline {
                // It started after all. The runner may remove a socket in
                // its own directory, which stops the helper.
                let _ = std::fs::remove_file(&socket);
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let out = child.wait_with_output().expect("wait for the helper");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        world.privhelper.refused_start = Some((dir, out.status.code(), text));
    }

    #[then("the helper refuses to start, naming that directory")]
    fn refuses_to_start(world: &mut CodetagsWorld) {
        let (dir, code, text) = world
            .privhelper
            .refused_start
            .as_ref()
            .expect("an earlier step started the helper");
        assert_eq!(*code, Some(1), "the helper did not refuse: {text}");
        assert!(
            text.contains(&format!("refusing to use {}", dir.display())),
            "the refusal does not name {}: {text}",
            dir.display()
        );
        assert!(!dir.join("privhelper.sock").exists());
    }

    #[given(expr = "the directory {string} in the project is readable only by root")]
    fn root_only_directory(world: &mut CodetagsWorld, dir: String) {
        let path = world.watch.project().join(&dir);
        sudo(&[
            "install".as_ref(),
            "-d".as_ref(),
            "-m".as_ref(),
            "700".as_ref(),
            "-o".as_ref(),
            "0".as_ref(),
            "-g".as_ref(),
            "0".as_ref(),
            path.as_os_str(),
        ]);
        world.watch.root_owned.0.push(path);
    }

    /// Connects a client and subscribes it to the project.
    fn subscribe_to_project(world: &mut CodetagsWorld) -> Client {
        let root = canonical(world.watch.project());
        let mut client = Client::connect(world.privhelper.socket())
            .unwrap_or_else(|error| panic!("connect to the helper: {error}"));
        let accepted = client
            .subscribe(&root)
            .unwrap_or_else(|error| panic!("subscribe to {}: {error}", root.display()));
        assert_eq!(accepted, root);
        world.privhelper.root = Some(root);
        client
    }

    /// Collects `client`'s notices from now on.
    fn start_reading(world: &mut CodetagsWorld, client: Client) {
        let shutdown = client.shutdown_handle().expect("clone the connection");
        let (tx, rx) = mpsc::channel();
        client
            .forward(move |notice| tx.send(notice.map_err(|e| e.to_string())).is_ok())
            .expect("start the client's thread");
        world.privhelper.notices = Some((rx, shutdown));
    }

    #[given(expr = "root creates the directory {string} in the project with mode {string}")]
    fn root_directory_with_mode(world: &mut CodetagsWorld, dir: String, mode: String) {
        let path = world.watch.project().join(&dir);
        sudo(&[
            "install".as_ref(),
            "-d".as_ref(),
            "-m".as_ref(),
            mode.as_ref(),
            "-o".as_ref(),
            "0".as_ref(),
            "-g".as_ref(),
            "0".as_ref(),
            path.as_os_str(),
        ]);
        world.watch.root_owned.0.push(path);
    }

    #[given("a helper client is subscribed to the project")]
    fn client_subscribed(world: &mut CodetagsWorld) {
        let client = subscribe_to_project(world);
        start_reading(world, client);
    }

    #[given("a helper client is subscribed to the project and reads nothing yet")]
    fn client_subscribed_idle(world: &mut CodetagsWorld) {
        let client = subscribe_to_project(world);
        world.privhelper.idle = Some(client);
    }

    #[when("the helper client starts reading")]
    fn client_starts_reading(world: &mut CodetagsWorld) {
        let client = world
            .privhelper
            .idle
            .take()
            .expect("an earlier step subscribed a client that reads nothing yet");
        start_reading(world, client);
    }

    #[when(expr = "a helper client subscribes to {string} and then {string} in the project")]
    fn client_subscribes_twice(world: &mut CodetagsWorld, first: String, second: String) {
        let project = world.watch.project().to_path_buf();
        let mut client = Client::connect(world.privhelper.socket())
            .unwrap_or_else(|error| panic!("connect to the helper: {error}"));
        let first = project.join(first);
        client
            .subscribe(&first)
            .unwrap_or_else(|error| panic!("subscribe to {}: {error}", first.display()));
        let second = project.join(second);
        world.privhelper.subscription = Some(match client.subscribe(&second) {
            Ok(root) => Ok(root),
            Err(HelperError::Refused { reason, .. }) => Err(reason),
            Err(error) => panic!("subscribe to {}: {error}", second.display()),
        });
    }

    #[when(
        expr = "the directory {string} is moved to {string} and replaced by one the runner owns"
    )]
    fn directory_swapped(world: &mut CodetagsWorld, dir: String, to: String) {
        let (from, to) = (
            world.watch.project().join(dir),
            world.watch.project().join(to),
        );
        // The runner owns the project, so it may rename an entry in it,
        // even a directory root owns.
        std::fs::rename(&from, &to)
            .unwrap_or_else(|error| panic!("rename {}: {error}", from.display()));
        std::fs::create_dir(&from)
            .unwrap_or_else(|error| panic!("create {}: {error}", from.display()));
        world.watch.root_owned.0.push(to);
    }

    #[when(expr = "a helper client subscribes to {string} in the project")]
    fn client_subscribes(world: &mut CodetagsWorld, dir: String) {
        let root = world.watch.project().join(dir);
        let mut client = Client::connect(world.privhelper.socket())
            .unwrap_or_else(|error| panic!("connect to the helper: {error}"));
        world.privhelper.subscription = Some(match client.subscribe(&root) {
            Ok(root) => Ok(root),
            Err(HelperError::Refused { reason, .. }) => Err(reason),
            Err(error) => panic!("subscribe to {}: {error}", root.display()),
        });
    }

    #[when(expr = "another process writes the file {string}")]
    fn another_process_writes(world: &mut CodetagsWorld, file: String) {
        let path = world.watch.project().join(&file);
        let mut child = Command::new("sh")
            .args([
                "-c",
                "printf '%s\\n' \"$1\" > \"$2\"",
                "sh",
                "written by sh",
            ])
            .arg(&path)
            .spawn()
            .expect("spawn sh");
        let pid = child.id();
        let status = child.wait().expect("wait for sh");
        assert!(status.success(), "sh failed to write {}", path.display());
        world.privhelper.writer = Some(pid);
    }

    #[when(expr = "root writes the file {string}")]
    fn root_writes(world: &mut CodetagsWorld, file: String) {
        let path = world.watch.project().join(&file);
        sudo(&[
            "sh".as_ref(),
            "-c".as_ref(),
            "printf 'written by root\\n' > \"$1\"".as_ref(),
            "sh".as_ref(),
            path.as_os_str(),
        ]);
    }

    #[then(expr = "within {int} seconds the helper reports a write to {string} by that process")]
    fn reports_write(world: &mut CodetagsWorld, seconds: u64, file: String) {
        let state = &mut world.privhelper;
        let path = state.root().join(&file);
        let writer = state
            .writer
            .expect("an earlier step had another process write");
        let found = state.collect(Duration::from_secs(seconds), |seen| {
            seen.iter().any(|event| {
                event.path == path
                    && event.pid == Some(writer)
                    && matches!(event.kind, EventKind::Modified | EventKind::Created)
            })
        });
        assert!(
            found,
            "no write to {} by pid {writer} within {seconds} s; the helper sent {:#?}",
            path.display(),
            state.seen
        );
    }

    #[then(expr = "the helper reported nothing under {string}")]
    fn reported_nothing_under(world: &mut CodetagsWorld, dir: String) {
        let state = &mut world.privhelper;
        let dir = state.root().join(dir);
        state.collect(Duration::from_millis(500), |_| false);
        let leaked: Vec<&HelperEvent> = state
            .seen
            .iter()
            .filter(|event| event.path.starts_with(&dir) && event.path != dir)
            .collect();
        assert!(leaked.is_empty(), "the helper leaked {leaked:#?}");
    }

    #[then(expr = "the helper refuses the subscription, saying {string}")]
    fn refuses_saying(world: &mut CodetagsWorld, text: String) {
        match world
            .privhelper
            .subscription
            .as_ref()
            .expect("an earlier step subscribed")
        {
            Err(reason) => assert!(reason.contains(&text), "the reason was {reason:?}"),
            Ok(root) => panic!("the helper accepted {}", root.display()),
        }
    }

    #[then("the helper refuses the subscription")]
    fn refuses(world: &mut CodetagsWorld) {
        match world
            .privhelper
            .subscription
            .as_ref()
            .expect("an earlier step subscribed")
        {
            Err(_) => {}
            Ok(root) => panic!("the helper accepted {}", root.display()),
        }
    }

    impl Drop for HelperState {
        fn drop(&mut self) {
            if let Some((_, shutdown)) = &self.notices {
                shutdown.shutdown();
            }
        }
    }
}
