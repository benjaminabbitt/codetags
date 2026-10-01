//! Upstream lspmux's configuration file, read the way lspmux reads it (D14,
//! D17, D20; docs/proxy-zero-change.md §3.3).
//!
//! lspmux has no option for another config file: it reads
//! `ProjectDirs::from("", "", "lspmux")`'s config directory, `config.toml`,
//! with `directories` 4.0.1 at the pinned revision (V116). An unparsable file
//! (an unknown key, a wrong type) is ignored, and lspmux runs on its defaults:
//! loopback TCP port 27631 and the whole environment as the routing key
//! (V36). [`check`] mirrors that parse, so the shim connects where lspmux
//! will listen and `codetags doctor` can say why a file is ignored.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};

/// The lspmux revision codetags is pinned to (D19); `providers.toml`
/// `[lspmux]` holds the same value for `just setup-lspmux`.
pub const PINNED_REV: &str = "18861f9d59e74ece8d867772cf07fa302c2dae98";
/// lspmux's default TCP port.
pub const DEFAULT_PORT: u16 = 27631;
/// The idle timeout `codetags lsp setup` writes, in seconds (lspmux's
/// default, written out so drift shows).
pub const DEFAULT_INSTANCE_TIMEOUT: u32 = 300;
/// The environment allowlist `codetags lsp setup` writes: only the routing-key
/// variables the shim sets (R9, R11), never raw `PATH` (R8).
pub const PASS_ENVIRONMENT: [&str; 1] = ["CODETAGS_KEY_*"];
/// The socket's file name inside its private directory.
pub const SOCKET_NAME: &str = "lspmux.sock";
/// The keys lspmux's `Config` accepts (`deny_unknown_fields`).
const KNOWN_KEYS: [&str; 6] = [
    "instance_timeout",
    "gc_interval",
    "listen",
    "connect",
    "log_filters",
    "pass_environment",
];

/// Where lspmux reads its config: `config.toml` in
/// `ProjectDirs::from("", "", "lspmux")`'s config directory. `None` when
/// there is no home directory.
pub fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "lspmux")
        .map(|dirs| dirs.config_dir().join("config.toml"))
}

/// An lspmux `listen` or `connect` address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    /// TCP: an IP address and a port.
    Tcp(IpAddr, u16),
    /// A Unix socket (Unix only in lspmux).
    Unix(PathBuf),
}

impl Address {
    /// lspmux's default, `127.0.0.1:27631`.
    pub fn lspmux_default() -> Self {
        Self::Tcp(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_PORT)
    }

    /// Parses `IP:PORT` (`[IPv6]:PORT` for IPv6) or an absolute socket path.
    pub fn parse(text: &str) -> Result<Self, String> {
        if let Ok(socket) = text.parse::<std::net::SocketAddr>() {
            return Ok(Self::Tcp(socket.ip(), socket.port()));
        }
        let path = Path::new(text);
        if path.is_absolute() {
            if cfg!(unix) {
                return Ok(Self::Unix(path.to_path_buf()));
            }
            return Err(format!(
                "{text}: lspmux supports Unix sockets only on Linux and macOS; use 127.0.0.1:PORT"
            ));
        }
        Err(format!(
            "{text}: expected IP:PORT (e.g. 127.0.0.1:{DEFAULT_PORT}) or an absolute socket path"
        ))
    }

    /// The address as a TOML value, as lspmux deserializes it: `["IP",
    /// PORT]` or a path string.
    pub fn to_toml(&self) -> String {
        match self {
            Self::Tcp(ip, port) => format!("[{}, {port}]", toml_string(&ip.to_string())),
            Self::Unix(path) => toml_string(&path.to_string_lossy()),
        }
    }

    fn from_toml(value: &toml::Value) -> Result<Self, String> {
        match value {
            toml::Value::Array(items) => match items.as_slice() {
                [toml::Value::String(ip), toml::Value::Integer(port)] => {
                    let ip = ip
                        .parse::<IpAddr>()
                        .map_err(|_| format!("`{ip}` is not an IP address"))?;
                    let port = u16::try_from(*port).map_err(|_| format!("bad port {port}"))?;
                    Ok(Self::Tcp(ip, port))
                }
                _ => Err("expected [\"IP\", PORT] or a socket path".to_string()),
            },
            toml::Value::String(path) if cfg!(unix) => Ok(Self::Unix(PathBuf::from(path))),
            toml::Value::String(_) => Err("Unix sockets are not supported on Windows".to_string()),
            _ => Err("expected [\"IP\", PORT] or a socket path".to_string()),
        }
    }

    /// Why other local users could reach this address, if they could: a TCP
    /// address that is not loopback, or a socket whose directory is not
    /// private to this user. Loopback TCP is reachable by every local user
    /// too; that is accepted on Windows, where lspmux has nothing else
    /// (C-3), and reported as a note by [`Address::exposure_note`].
    pub fn unsafe_reason(&self) -> Option<String> {
        match self {
            Self::Tcp(ip, port) if !ip.is_loopback() => Some(format!(
                "listen {} is not loopback",
                Address::Tcp(*ip, *port)
            )),
            Self::Tcp(..) => None,
            Self::Unix(path) => {
                let dir = path.parent().unwrap_or(Path::new("/"));
                private_dir_problem(dir)
                    .map(|problem| format!("the socket's directory {}: {problem}", dir.display()))
            }
        }
    }

    /// A note for loopback TCP: any local user can connect, and the
    /// handshake names a program for the daemon to run (V1, C-3).
    pub fn exposure_note(&self) -> Option<&'static str> {
        match self {
            Self::Tcp(..) => Some(
                "any local user can connect to loopback TCP and have the daemon run a program as you; \
                 accepted on single-user machines (PLAN.md D17, C-3)",
            ),
            Self::Unix(_) => None,
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp(ip @ IpAddr::V6(_), port) => write!(f, "[{ip}]:{port}"),
            Self::Tcp(ip, port) => write!(f, "{ip}:{port}"),
            Self::Unix(path) => write!(f, "{}", path.display()),
        }
    }
}

/// A TOML basic string holding `text`.
fn toml_string(text: &str) -> String {
    toml::Value::String(text.to_string()).to_string()
}

/// The longest Unix socket path, in bytes: `sun_path` is 108 bytes on Linux
/// and 104 on macOS, including the terminating NUL.
pub const MAX_SOCKET_PATH: usize = if cfg!(target_os = "linux") { 107 } else { 103 };

/// The default listen address `codetags lsp setup` writes. On Linux and
/// macOS a socket in a private directory (D17): `$XDG_RUNTIME_DIR/codetags`
/// when that is set (Linux sessions), otherwise `$XDG_STATE_HOME/codetags`,
/// otherwise `~/.local/state/codetags`. On Windows, lspmux's default
/// loopback TCP address.
pub fn default_listen() -> Result<Address, String> {
    if cfg!(unix) {
        let absolute = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        let dir = if let Some(runtime) = absolute("XDG_RUNTIME_DIR") {
            runtime.join("codetags")
        } else if let Some(state) = absolute("XDG_STATE_HOME") {
            state.join("codetags")
        } else {
            let home = absolute("HOME").ok_or("HOME is not set to an absolute path")?;
            home.join(".local").join("state").join("codetags")
        };
        Ok(Address::Unix(dir.join(SOCKET_NAME)))
    } else {
        Ok(Address::lspmux_default())
    }
}

/// Checks that `address` is one `codetags lsp setup` may write: loopback
/// TCP, or an absolute socket path short enough for `sun_path`. Whether the
/// socket's directory is private is checked when it is created.
pub fn validate_listen(address: &Address) -> Result<(), String> {
    match address {
        Address::Tcp(ip, _) if !ip.is_loopback() => Err(format!(
            "{address} is not a loopback address; other machines could reach it (brief §2)"
        )),
        Address::Tcp(..) => Ok(()),
        Address::Unix(path) => {
            let length = path.as_os_str().len();
            if length > MAX_SOCKET_PATH {
                return Err(format!(
                    "{address} is {length} bytes; a socket path may have at most {MAX_SOCKET_PATH}"
                ));
            }
            Ok(())
        }
    }
}

/// The config text `codetags lsp setup` writes.
pub fn render(listen: &Address, instance_timeout: u32) -> String {
    let pass: Vec<String> = PASS_ENVIRONMENT
        .iter()
        .map(|name| toml_string(name))
        .collect();
    format!(
        "# lspmux configuration, written by `codetags lsp setup` (codetags PLAN.md D20).\n\
         # Change it by running that command again; `codetags doctor` reports drift.\n\
         instance_timeout = {instance_timeout}\n\
         listen = {address}\n\
         connect = {address}\n\
         pass_environment = [{pass}]\n",
        address = listen.to_toml(),
        pass = pass.join(", "),
    )
}

/// What lspmux makes of a config file's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective {
    /// The address lspmux servers listen on.
    pub listen: Address,
    /// The address lspmux clients connect to.
    pub connect: Address,
}

impl Default for Effective {
    fn default() -> Self {
        Self {
            listen: Address::lspmux_default(),
            connect: Address::lspmux_default(),
        }
    }
}

/// Parses `text` as lspmux's `Config` does: every key known, every value of
/// the right type. `Err` says why lspmux would ignore the file and run on
/// its defaults.
pub fn check(text: &str) -> Result<Effective, String> {
    let table: toml::Table = text.parse().map_err(|error| format!("not TOML: {error}"))?;
    let mut effective = Effective::default();
    for (key, value) in &table {
        match key.as_str() {
            "instance_timeout" => match value {
                toml::Value::Integer(seconds) if u32::try_from(*seconds).is_ok() => {}
                toml::Value::Boolean(false) => {}
                _ => {
                    return Err("`instance_timeout` must be a non-negative integer or false".into());
                }
            },
            "gc_interval" => match value {
                toml::Value::Integer(seconds)
                    if *seconds >= 1 && u32::try_from(*seconds).is_ok() => {}
                _ => return Err("`gc_interval` must be an integer 1 or greater".into()),
            },
            "listen" => {
                effective.listen =
                    Address::from_toml(value).map_err(|error| format!("`listen`: {error}"))?;
            }
            "connect" => {
                effective.connect =
                    Address::from_toml(value).map_err(|error| format!("`connect`: {error}"))?;
            }
            "log_filters" => {
                if !value.is_str() {
                    return Err("`log_filters` must be a string".into());
                }
            }
            "pass_environment" => match value {
                toml::Value::Array(items) if items.iter().all(toml::Value::is_str) => {}
                _ => return Err("`pass_environment` must be a list of strings".into()),
            },
            unknown => {
                return Err(format!(
                    "unknown key `{unknown}` (lspmux knows {})",
                    KNOWN_KEYS.join(", ")
                ));
            }
        }
    }
    Ok(effective)
}

/// The address an lspmux client will connect to, given the config file's
/// text (`None`: no file): the file's `connect`, or lspmux's default when the
/// file is missing or would be ignored.
pub fn effective_connect(text: Option<&str>) -> Address {
    text.and_then(|text| check(text).ok())
        .unwrap_or_default()
        .connect
}

/// Reads lspmux's config file, if there is one.
pub fn read_config() -> Result<(PathBuf, Option<String>), String> {
    let path = config_path().ok_or("no home directory, so no lspmux config directory")?;
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok((path, Some(text))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((path, None)),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

/// Why `dir` is not private to this user, if it is not: it must exist, be
/// a directory owned by this user, and grant nothing to group or others.
#[cfg(unix)]
pub fn private_dir_problem(dir: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = match std::fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error) => return Some(error.to_string()),
    };
    if !metadata.is_dir() {
        return Some("not a directory".to_string());
    }
    let uid = nix::unistd::getuid().as_raw();
    if metadata.uid() != uid {
        return Some(format!("owned by uid {}, not {uid}", metadata.uid()));
    }
    let mode = metadata.mode() & 0o777;
    if mode & 0o077 != 0 {
        return Some(format!("mode {mode:04o}; it must be 0700"));
    }
    None
}

/// Windows has no Unix sockets in lspmux; nothing to check.
#[cfg(not(unix))]
pub fn private_dir_problem(_dir: &Path) -> Option<String> {
    None
}

/// Creates `dir` (and missing parents) private to this user, or tightens an
/// existing directory this user owns to 0700. Fails if another user owns it.
#[cfg(unix)]
pub fn ensure_private_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    if !dir.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    }
    let metadata = std::fs::symlink_metadata(dir)
        .map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    let uid = nix::unistd::getuid().as_raw();
    if metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("cannot make {} private: {error}", dir.display()))?;
    }
    match private_dir_problem(dir) {
        None => Ok(()),
        Some(problem) => Err(format!("{}: {problem}", dir.display())),
    }
}

/// Creates `dir` if it is missing (Windows: no permission model to apply).
#[cfg(not(unix))]
pub fn ensure_private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("cannot create {}: {error}", dir.display()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn the_pin_matches_providers_toml() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../providers.toml");
        let providers: toml::Table = std::fs::read_to_string(path).unwrap().parse().unwrap();
        assert_eq!(providers["lspmux"]["rev"].as_str(), Some(PINNED_REV));
    }

    #[test]
    fn rendered_config_parses_as_lspmux_would() {
        let address = Address::Tcp(IpAddr::V4(Ipv4Addr::LOCALHOST), 4000);
        let text = render(&address, 300);
        let effective = check(&text).unwrap();
        assert_eq!(effective.listen, address);
        assert_eq!(effective.connect, address);
        assert!(
            text.contains("pass_environment = [\"CODETAGS_KEY_*\"]"),
            "{text}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn socket_addresses_round_trip() {
        let address = Address::Unix(PathBuf::from("/run/user/1/codetags/lspmux.sock"));
        let text = render(&address, 5);
        assert!(
            text.contains("listen = \"/run/user/1/codetags/lspmux.sock\""),
            "{text}"
        );
        assert_eq!(check(&text).unwrap().connect, address);
    }

    #[test]
    fn unknown_keys_and_bad_types_are_why_lspmux_ignores_a_file() {
        assert!(
            check("pass_enviroment = []")
                .unwrap_err()
                .contains("pass_enviroment")
        );
        assert!(check("instance_timeout = true").is_err());
        assert!(check("gc_interval = 0").is_err());
        assert!(check("listen = [1, 2]").is_err());
        assert_eq!(
            check("instance_timeout = false").unwrap(),
            Effective::default()
        );
    }

    #[test]
    fn effective_connect_falls_back_to_lspmux_defaults() {
        assert_eq!(effective_connect(None), Address::lspmux_default());
        assert_eq!(
            effective_connect(Some("nonsense = 1")),
            Address::lspmux_default()
        );
        assert_eq!(
            effective_connect(Some("connect = [\"127.0.0.1\", 9]")),
            Address::Tcp(IpAddr::V4(Ipv4Addr::LOCALHOST), 9)
        );
    }

    #[test]
    fn addresses_parse_and_display() {
        let tcp = Address::parse("127.0.0.1:27631").unwrap();
        assert_eq!(tcp, Address::lspmux_default());
        assert_eq!(tcp.to_string(), "127.0.0.1:27631");
        assert_eq!(Address::parse("[::1]:5").unwrap().to_string(), "[::1]:5");
        assert!(Address::parse("relative/path").is_err());
    }

    #[test]
    fn only_loopback_tcp_is_valid() {
        assert!(validate_listen(&Address::parse("0.0.0.0:1").unwrap()).is_err());
        assert!(validate_listen(&Address::parse("127.0.0.1:1").unwrap()).is_ok());
        assert!(
            Address::parse("0.0.0.0:1")
                .unwrap()
                .unsafe_reason()
                .is_some()
        );
    }

    #[cfg(unix)]
    #[test]
    fn long_socket_paths_are_refused() {
        let long = Address::Unix(PathBuf::from(format!("/{}", "a".repeat(200))));
        assert!(validate_listen(&long).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn private_dirs_are_created_and_tightened() {
        use std::os::unix::fs::PermissionsExt;

        let scratch = tempfile::tempdir().unwrap();
        let dir = scratch.path().join("a").join("b");
        ensure_private_dir(&dir).unwrap();
        assert!(private_dir_problem(&dir).is_none());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_dir_problem(&dir).unwrap().contains("0755"));
        ensure_private_dir(&dir).unwrap();
        assert!(private_dir_problem(&dir).is_none());
    }
}
