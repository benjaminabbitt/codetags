//! Steps for `features/lsp/setup.feature` (M3 stage 1, PLAN.md D19, D20):
//! `codetags lsp setup` and the proxy checks of `codetags doctor`, plus the
//! isolated home that `features/lsp/shim.feature` builds on.
//!
//! Every scenario gets its own home: `HOME`, the XDG directories,
//! `USERPROFILE`, `APPDATA` and `LOCALAPPDATA` point into its scratch
//! directory for every process it starts, so lspmux's config file, socket
//! and daemon are its own. lspmux has no option for another config file
//! (V36); the "isolated home" step fails if `codetags lsp setup` would still
//! name a config file outside it (V116). Daemons a scenario's shims started
//! are killed when it ends.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use cucumber::{given, then, when};

use crate::{CODETAGS_BIN, CodetagsWorld};

/// The mark `codetags-lsp` writes in the daemon's log for each start
/// (`codetags_lsp::daemon::STARTED_MARK`).
const STARTED_MARK: &str = "codetags-lsp: started lspmux server";

/// State for the setup and doctor scenarios, and the shim's home.
#[derive(Debug, Default)]
pub(crate) struct LspmuxState {
    /// The scenario's home and daemon.
    pub(crate) home: Option<Home>,
}

/// A scenario's own home directory and daemon.
#[derive(Debug)]
pub(crate) struct Home {
    /// The home directory.
    root: PathBuf,
    /// Linux and macOS: the runtime directory holding the socket's private
    /// directory. Kept short (a socket path is at most 103 bytes on macOS),
    /// so it is a separate temporary directory.
    #[cfg(unix)]
    runtime: tempfile::TempDir,
    /// Windows: the loopback port the daemon listens on.
    pub(crate) port: Option<u16>,
}

impl Home {
    /// The directory holding the daemon's lock, pid and log files
    /// (`codetags_lsp::daemon::state_dir`).
    fn state_dir(&self) -> PathBuf {
        #[cfg(unix)]
        {
            self.runtime.path().join("codetags")
        }
        #[cfg(not(unix))]
        {
            self.root
                .join("AppData")
                .join("Local")
                .join("codetags")
                .join("lspmux")
        }
    }

    /// The daemon's log (`codetags_lsp::daemon::log_file`).
    fn daemon_log(&self) -> PathBuf {
        match self.port {
            Some(port) => self.state_dir().join(format!("lspmux-{port}.log")),
            None => self.state_dir().join("lspmux.log"),
        }
    }

    /// Whether the scenario's daemon accepts connections.
    pub(crate) fn daemon_answers(&self) -> bool {
        if let Some(port) = self.port {
            return std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
                Duration::from_secs(1),
            )
            .is_ok();
        }
        #[cfg(unix)]
        {
            std::os::unix::net::UnixStream::connect(self.state_dir().join("lspmux.sock")).is_ok()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    /// The process ids of the daemons `codetags-lsp` started, from its log.
    pub(crate) fn daemon_pids(&self) -> Vec<u32> {
        started_daemons(&self.daemon_log())
    }
}

/// The process ids of the daemons `codetags-lsp` started, from the daemon
/// log at `log` (its [`STARTED_MARK`] lines).
pub(crate) fn started_daemons(log: &Path) -> Vec<u32> {
    let log = std::fs::read_to_string(log).unwrap_or_default();
    log.lines()
        .filter(|line| line.starts_with(STARTED_MARK))
        .filter_map(|line| {
            let after = line.split("(pid ").nth(1)?;
            after.split(')').next()?.parse().ok()
        })
        .collect()
}

impl Drop for Home {
    fn drop(&mut self) {
        for pid in self.daemon_pids() {
            kill_tree(pid);
        }
    }
}

/// Kills a process, and on Windows its descendants (the fake servers);
/// on Unix those end when the daemon's pipes close.
pub(crate) fn kill_tree(pid: u32) {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("taskkill");
        command.args(["/F", "/T", "/PID", &pid.to_string()]);
        command
    } else {
        let mut command = Command::new("kill");
        command.args(["-KILL", &pid.to_string()]);
        command
    };
    let _ = command.stdout(Stdio::null()).stderr(Stdio::null()).status();
}

pub(crate) fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/codetags-bdd sits two levels below the repo root")
}

/// The lspmux binary: `$CODETAGS_LSPMUX`, `.codetags/local/bin/lspmux` in
/// this checkout (`just setup-lspmux`), or `lspmux` on PATH.
pub(crate) fn lspmux_binary() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("CODETAGS_LSPMUX").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(explicit));
    }
    let name = format!("lspmux{}", std::env::consts::EXE_SUFFIX);
    let local = repo_root()
        .join(".codetags")
        .join("local")
        .join("bin")
        .join(&name);
    if local.is_file() {
        return Some(local);
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join(&name))
        .find(|candidate| candidate.is_file())
}

pub(crate) fn required_lspmux() -> PathBuf {
    lspmux_binary().expect(
        "lspmux not found: run `just setup-lspmux`, or set CODETAGS_LSPMUX (the @lspmux scenarios need it)",
    )
}

pub(crate) fn home(world: &CodetagsWorld) -> &Home {
    world
        .lspmux
        .home
        .as_ref()
        .expect("a Given step set up an isolated home")
}

/// The environment pointing every per-user directory into `root`.
fn home_env(root: &Path, #[allow(unused_variables)] runtime: &Path) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = vec![
        ("HOME".into(), root.into()),
        ("XDG_CONFIG_HOME".into(), root.join(".config").into()),
        (
            "XDG_STATE_HOME".into(),
            root.join(".local").join("state").into(),
        ),
        (
            "XDG_DATA_HOME".into(),
            root.join(".local").join("share").into(),
        ),
        ("USERPROFILE".into(), root.into()),
        (
            "APPDATA".into(),
            root.join("AppData").join("Roaming").into(),
        ),
        (
            "LOCALAPPDATA".into(),
            root.join("AppData").join("Local").into(),
        ),
    ];
    #[cfg(unix)]
    env.push(("XDG_RUNTIME_DIR".into(), runtime.into()));
    if let Some(lspmux) = lspmux_binary() {
        env.push(("CODETAGS_LSPMUX".into(), lspmux.into()));
    }
    env
}

#[given("an isolated home for lspmux")]
pub(crate) fn an_isolated_home(world: &mut CodetagsWorld) {
    let root = world.scratch().join("home");
    for dir in [
        root.join(".config"),
        root.join("AppData").join("Roaming"),
        root.join("AppData").join("Local"),
    ] {
        std::fs::create_dir_all(&dir).expect("create the isolated home");
    }
    #[cfg(unix)]
    let runtime = tempfile::Builder::new()
        .prefix("ct")
        .tempdir()
        .expect("create a runtime directory");
    #[cfg(unix)]
    let runtime_path = runtime.path().to_path_buf();
    #[cfg(not(unix))]
    let runtime_path = root.join("run");
    world.env.extend(home_env(&root, &runtime_path));
    world.lspmux.home = Some(Home {
        root: root.clone(),
        #[cfg(unix)]
        runtime,
        port: None,
    });
    // `codetags lsp setup`, unconfirmed, names the file it would write:
    // it must be inside this home, or lspmux would read a real config.
    let output = codetags(world, &["lsp", "setup"], Some(""));
    let path = output
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("lspmux config: "))
        .unwrap_or_else(|| {
            panic!(
                "codetags lsp setup named no config file:\n{}{}",
                output.stdout, output.stderr
            )
        })
        .to_string();
    assert!(
        Path::new(&path).starts_with(&root),
        "lspmux's config file {path} is outside the isolated home {}: on this OS the \
         environment does not redirect it (V116), so these scenarios would touch the real one",
        root.display()
    );
}

/// What a `codetags` run produced.
struct Ran {
    status: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Runs `codetags args` with the scenario's environment and `input` on
/// stdin (`None`: no stdin).
fn codetags(world: &CodetagsWorld, args: &[&str], input: Option<&str>) -> Ran {
    let binary = CODETAGS_BIN.get().expect("run() sets the binary path");
    let mut command = Command::new(binary);
    command.args(args);
    for name in &world.env_removed {
        command.env_remove(name);
    }
    command.envs(world.env.iter().map(|(name, value)| (name, value)));
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn codetags");
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        let _ = stdin.write_all(input.as_bytes());
    }
    let output = child.wait_with_output().expect("wait for codetags");
    Ran {
        status: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

#[given("codetags lsp setup has run")]
fn setup_has_run(world: &mut CodetagsWorld) {
    let ran = codetags(world, &["lsp", "setup", "--yes"], None);
    assert_eq!(
        ran.status,
        Some(0),
        "codetags lsp setup --yes:\n{}{}",
        ran.stdout,
        ran.stderr
    );
}

#[given("lspmux is set up in an isolated home")]
fn lspmux_is_set_up(world: &mut CodetagsWorld) {
    an_isolated_home(world);
    let mut args = vec!["lsp".to_string(), "setup".to_string(), "--yes".to_string()];
    if cfg!(windows) {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .expect("find a free loopback port")
            .port();
        world.lspmux.home.as_mut().expect("the home exists").port = Some(port);
        args.push("--listen".into());
        args.push(format!("127.0.0.1:{port}"));
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let ran = codetags(world, &args, None);
    assert_eq!(
        ran.status,
        Some(0),
        "codetags lsp setup:\n{}{}",
        ran.stdout,
        ran.stderr
    );
}

#[when(expr = "codetags is run with {string} and the answer {string}")]
fn codetags_with_answer(world: &mut CodetagsWorld, args: String, answer: String) {
    let args: Vec<&str> = args.split_whitespace().collect();
    let ran = codetags(world, &args, Some(&format!("{answer}\n")));
    world.last = Some(crate::CommandOutcome {
        status: ran.status,
        stdout: ran.stdout,
        stderr: ran.stderr,
    });
}

/// lspmux's config file in the isolated home (`ProjectDirs` per OS, V116).
fn config_file(world: &CodetagsWorld) -> PathBuf {
    let root = &home(world).root;
    if cfg!(windows) {
        root.join("AppData")
            .join("Roaming")
            .join("lspmux")
            .join("config")
            .join("config.toml")
    } else if cfg!(target_os = "macos") {
        root.join("Library")
            .join("Application Support")
            .join("lspmux")
            .join("config.toml")
    } else {
        root.join(".config").join("lspmux").join("config.toml")
    }
}

#[given(expr = "lspmux's config file holds {string}")]
fn config_holds(world: &mut CodetagsWorld, text: String) {
    let path = config_file(world);
    std::fs::create_dir_all(path.parent().expect("a directory")).expect("create the config dir");
    std::fs::write(&path, format!("{text}\n")).expect("write lspmux's config");
}

#[given(expr = "lspmux's config file has {string} replaced by {string}")]
fn config_replaced(world: &mut CodetagsWorld, from: String, to: String) {
    let path = config_file(world);
    let text = std::fs::read_to_string(&path).expect("read lspmux's config");
    assert!(
        text.contains(&from),
        "{} does not hold {from:?}:\n{text}",
        path.display()
    );
    std::fs::write(&path, text.replace(&from, &to)).expect("write lspmux's config");
}

fn config_table(world: &CodetagsWorld) -> toml::Table {
    let path = config_file(world);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    text.parse()
        .unwrap_or_else(|error| panic!("{} is not TOML: {error}\n{text}", path.display()))
}

#[then(expr = "lspmux's config sets {string} to {string}")]
fn config_sets(world: &mut CodetagsWorld, key: String, value: String) {
    let expected: toml::Table = format!("value = {value}")
        .parse()
        .expect("the expected value is TOML");
    let table = config_table(world);
    assert_eq!(
        table.get(&key),
        expected.get("value"),
        "{key} in {table:#?}"
    );
}

#[then("lspmux's config listens on a socket in a private directory")]
fn config_listens_on_socket(world: &mut CodetagsWorld) {
    let table = config_table(world);
    let listen = table
        .get("listen")
        .and_then(toml::Value::as_str)
        .expect("listen is a socket path");
    assert_eq!(
        table.get("connect").and_then(toml::Value::as_str),
        Some(listen)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let expected = home(world).state_dir().join("lspmux.sock");
        assert_eq!(Path::new(listen), expected);
        let mode = std::fs::metadata(home(world).state_dir())
            .expect("the socket's directory exists")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o700,
            "the socket's directory has mode {:o}",
            mode & 0o777
        );
    }
}

fn backups(world: &CodetagsWorld) -> Vec<PathBuf> {
    let path = config_file(world);
    let dir = path.parent().expect("a directory");
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("config.toml.bak-")
        })
        .map(|entry| entry.path())
        .collect();
    found.sort();
    found
}

#[then(expr = "a backup of lspmux's config holds {string}")]
fn backup_holds(world: &mut CodetagsWorld, text: String) {
    let found = backups(world);
    assert_eq!(found.len(), 1, "backups: {found:?}");
    let held = std::fs::read_to_string(&found[0]).expect("read the backup");
    assert_eq!(held.trim_end(), text);
}

#[then("no backup of lspmux's config exists")]
fn no_backup(world: &mut CodetagsWorld) {
    let found = backups(world);
    assert!(found.is_empty(), "backups: {found:?}");
}

#[then(expr = "lspmux's config file still holds {string}")]
fn config_still_holds(world: &mut CodetagsWorld, text: String) {
    let held = std::fs::read_to_string(config_file(world)).expect("read lspmux's config");
    assert_eq!(held.trim_end(), text);
}

#[then("lspmux's config file does not exist")]
fn config_does_not_exist(world: &mut CodetagsWorld) {
    let path = config_file(world);
    assert!(!path.exists(), "{} exists", path.display());
}

#[when("lspmux prints its effective config")]
fn lspmux_prints_config(world: &mut CodetagsWorld) {
    let mut command = Command::new(required_lspmux());
    command.arg("config");
    command.envs(world.env.iter().map(|(name, value)| (name, value)));
    let output = command.output().expect("run lspmux config");
    world.last = Some(output.into());
}

#[then("lspmux's effective config listens where codetags lsp setup wrote")]
fn effective_config_matches(world: &mut CodetagsWorld) {
    let table = config_table(world);
    let printed = &world.last().stdout;
    let expected = match table.get("listen") {
        Some(toml::Value::String(path)) => {
            format!("listen: Unix(\n        {:?},\n    )", Path::new(path))
        }
        Some(toml::Value::Array(items)) => format!(
            "listen: Tcp(\n        {},\n        {},\n    )",
            items[0].as_str().expect("an IP"),
            items[1].as_integer().expect("a port")
        ),
        other => panic!("unexpected listen {other:?}"),
    };
    assert!(
        printed.contains(&expected),
        "lspmux config printed:\n{printed}\nexpected it to contain:\n{expected}\nstderr: {}",
        world.last().stderr
    );
}

#[then("an lspmux daemon is answering")]
fn daemon_is_answering(world: &mut CodetagsWorld) {
    assert!(home(world).daemon_answers(), "no lspmux daemon answers");
}

#[given("no lspmux daemon is answering")]
#[then("no lspmux daemon is answering")]
fn no_daemon_is_answering(world: &mut CodetagsWorld) {
    assert!(!home(world).daemon_answers(), "an lspmux daemon answers");
}

#[then(expr = "the lspmux daemon was started {int} time(s)")]
fn daemon_started(world: &mut CodetagsWorld, count: usize) {
    let pids = home(world).daemon_pids();
    assert_eq!(pids.len(), count, "daemons started: {pids:?}");
}
