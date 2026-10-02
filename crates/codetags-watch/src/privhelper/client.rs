//! The client side of the helper's socket (Unix only).

use std::collections::VecDeque;
use std::fmt;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::Duration;

use super::proto::{HelperEvent, Reply, Request, VERSION};

/// How long the handshake and each subscription may take before the client
/// gives up on the helper.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Why the helper could not be used. Each is a reason to fall back to
/// notify, never a reason to fail.
#[derive(Debug)]
pub enum HelperError {
    /// Nothing listens at the socket: it does not exist, or refused the
    /// connection, or this user may not connect to it.
    NotRunning {
        /// The socket path.
        socket: PathBuf,
        /// What connecting failed with.
        source: io::Error,
    },
    /// The helper rejected the handshake, or sent something else.
    Handshake(String),
    /// The helper refused to watch a root.
    Refused {
        /// The root asked for.
        root: PathBuf,
        /// The helper's reason.
        reason: String,
    },
    /// The connection failed after the handshake.
    Io(io::Error),
}

impl fmt::Display for HelperError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HelperError::NotRunning { socket, source } => {
                write!(f, "no privileged helper at {} ({source})", socket.display())
            }
            HelperError::Handshake(reason) => {
                write!(f, "the privileged helper's handshake failed: {reason}")
            }
            HelperError::Refused { root, reason } => write!(
                f,
                "the privileged helper refused to watch {}: {reason}",
                root.display()
            ),
            HelperError::Io(error) => {
                write!(f, "the privileged helper's connection failed: {error}")
            }
        }
    }
}

impl std::error::Error for HelperError {}

/// What the helper sends once subscribed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// An event under an accepted root.
    Event(HelperEvent),
    /// The helper lost events: rescan.
    Overflow,
}

/// A connection to the privileged helper.
#[derive(Debug)]
pub struct Client {
    stream: UnixStream,
    socket: PathBuf,
    backend: String,
    /// Events that arrived while waiting for a subscription's reply.
    early: VecDeque<Notice>,
}

impl Client {
    /// Connects to the helper at `socket` and completes the handshake.
    pub fn connect(socket: &Path) -> Result<Self, HelperError> {
        let stream = UnixStream::connect(socket).map_err(|source| HelperError::NotRunning {
            socket: socket.to_path_buf(),
            source,
        })?;
        stream
            .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
            .map_err(HelperError::Io)?;
        let mut client = Self {
            stream,
            socket: socket.to_path_buf(),
            backend: String::new(),
            early: VecDeque::new(),
        };
        // A helper over its connection limit writes its refusal and hangs
        // up without reading this; if it did so first, the write fails but
        // the refusal is still there to read.
        let hello = Request::Hello { version: VERSION }.write_to(&mut client.stream);
        let reply = client.read_reply();
        if let (Err(error), Err(_)) = (&hello, &reply) {
            return Err(HelperError::Handshake(error.to_string()));
        }
        match reply {
            Ok(Reply::Welcome { version, backend }) if version == VERSION => {
                client.backend = backend;
                Ok(client)
            }
            Ok(Reply::Welcome { version, .. }) => Err(HelperError::Handshake(format!(
                "the helper speaks protocol {version}, this client {VERSION}"
            ))),
            Ok(Reply::Rejected { reason }) => Err(HelperError::Handshake(reason)),
            Ok(other) => Err(HelperError::Handshake(format!(
                "unexpected reply {other:?}"
            ))),
            Err(error) => Err(HelperError::Handshake(error.to_string())),
        }
    }

    /// The helper's event source, e.g. `fanotify`.
    pub fn backend(&self) -> &str {
        &self.backend
    }

    /// The socket this client is connected to.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Subscribes to the events under `root`, returning the root as the
    /// helper canonicalized it; event paths start with it. Events for the
    /// root's changes from now on will be delivered.
    pub fn subscribe(&mut self, root: &Path) -> Result<PathBuf, HelperError> {
        Request::Subscribe {
            root: root.to_path_buf(),
        }
        .write_to(&mut self.stream)
        .map_err(HelperError::Io)?;
        loop {
            match self.read_reply().map_err(HelperError::Io)? {
                Reply::Accepted { root } => return Ok(root),
                Reply::Refused { root, reason } => {
                    return Err(HelperError::Refused { root, reason });
                }
                Reply::Event(event) => self.early.push_back(Notice::Event(event)),
                Reply::Overflow => self.early.push_back(Notice::Overflow),
                other => {
                    return Err(HelperError::Io(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unexpected reply {other:?}"),
                    )));
                }
            }
        }
    }

    /// Returns a handle that shuts the connection down, which ends a
    /// [`Client::forward`] thread.
    pub fn shutdown_handle(&self) -> io::Result<Shutdown> {
        Ok(Shutdown(self.stream.try_clone()?))
    }

    /// Reads notices on a thread of its own and hands each to `sink`, until
    /// `sink` returns `false`, or the connection ends; then `sink` gets the
    /// error once (an `UnexpectedEof` if the helper closed the connection).
    pub fn forward<F>(mut self, mut sink: F) -> io::Result<JoinHandle<()>>
    where
        F: FnMut(Result<Notice, io::Error>) -> bool + Send + 'static,
    {
        // macOS fails this with EINVAL once the helper has hung up. A dead
        // connection then shows on the first read, which ends the thread
        // with an error as any lost connection does, so the error is not
        // fatal here.
        let _ = self.stream.set_read_timeout(None);
        std::thread::Builder::new()
            .name("codetags-privhelper-client".into())
            .spawn(move || {
                while let Some(notice) = self.early.pop_front() {
                    if !sink(Ok(notice)) {
                        return;
                    }
                }
                loop {
                    let notice = match self.read_reply() {
                        Ok(Reply::Event(event)) => Ok(Notice::Event(event)),
                        Ok(Reply::Overflow) => Ok(Notice::Overflow),
                        Ok(other) => Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("unexpected reply {other:?}"),
                        )),
                        Err(error) => Err(error),
                    };
                    let failed = notice.is_err();
                    if !sink(notice) || failed {
                        return;
                    }
                }
            })
    }

    fn read_reply(&mut self) -> io::Result<Reply> {
        match Reply::read_from(&mut self.stream)? {
            Some(reply) => Ok(reply),
            None => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the helper closed the connection",
            )),
        }
    }
}

/// Shuts a [`Client`]'s connection down from another thread.
#[derive(Debug)]
pub struct Shutdown(UnixStream);

impl Shutdown {
    /// Shuts the connection down; a blocked read in the forwarding thread
    /// returns at once.
    pub fn shutdown(&self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privhelper::proto::{EventKind, read_frame};
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;

    /// A fake helper that answers one client with `script` after reading its
    /// requests, which it returns.
    fn fake_helper(
        dir: &Path,
        script: Vec<(usize, Vec<Reply>)>,
    ) -> (PathBuf, JoinHandle<Vec<Request>>) {
        let socket = dir.join("helper.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut requests = Vec::new();
            for (read, replies) in script {
                for _ in 0..read {
                    let (tag, body) = read_frame(&mut stream).unwrap().unwrap();
                    requests.push(Request::decode(tag, &body).unwrap());
                }
                for reply in replies {
                    reply.write_to(&mut stream).unwrap();
                }
            }
            requests
        });
        (socket, handle)
    }

    fn welcome() -> Reply {
        Reply::Welcome {
            version: VERSION,
            backend: "fanotify".into(),
        }
    }

    fn event(path: &str) -> Reply {
        Reply::Event(HelperEvent {
            path: path.into(),
            kind: EventKind::Modified,
            pid: Some(7),
        })
    }

    #[test]
    fn no_socket_means_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let error = Client::connect(&dir.path().join("absent.sock")).unwrap_err();
        assert!(matches!(error, HelperError::NotRunning { .. }), "{error}");
        assert!(error.to_string().contains("no privileged helper at"));
    }

    #[test]
    fn handshake_subscribe_and_events() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, helper) = fake_helper(
            dir.path(),
            vec![
                (1, vec![welcome()]),
                (
                    1,
                    vec![Reply::Accepted { root: "/p".into() }, event("/p/a")],
                ),
            ],
        );
        let mut client = Client::connect(&socket).unwrap();
        assert_eq!(client.backend(), "fanotify");
        assert_eq!(client.subscribe(Path::new("/p")).unwrap(), Path::new("/p"));
        let (tx, rx) = mpsc::channel();
        let thread = client
            .forward(move |notice| tx.send(notice.map_err(|e| e.kind())).is_ok())
            .unwrap();
        let Ok(Notice::Event(got)) = rx.recv().unwrap() else {
            panic!("expected an event")
        };
        assert_eq!(got.path, Path::new("/p/a"));
        assert_eq!(got.pid, Some(7));
        assert_eq!(
            helper.join().unwrap(),
            vec![
                Request::Hello { version: VERSION },
                Request::Subscribe { root: "/p".into() }
            ]
        );
        // The fake helper hung up.
        assert_eq!(rx.recv().unwrap(), Err(io::ErrorKind::UnexpectedEof));
        thread.join().unwrap();
    }

    #[test]
    fn events_before_a_later_reply_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, _helper) = fake_helper(
            dir.path(),
            vec![
                (1, vec![welcome()]),
                (1, vec![Reply::Accepted { root: "/p".into() }]),
                (
                    1,
                    vec![event("/p/early"), Reply::Accepted { root: "/q".into() }],
                ),
            ],
        );
        let mut client = Client::connect(&socket).unwrap();
        client.subscribe(Path::new("/p")).unwrap();
        client.subscribe(Path::new("/q")).unwrap();
        let (tx, rx) = mpsc::channel();
        client
            .forward(move |notice| tx.send(notice.ok()).is_ok())
            .unwrap();
        assert!(
            matches!(rx.recv().unwrap(), Some(Notice::Event(e)) if e.path == Path::new("/p/early"))
        );
    }

    #[test]
    fn a_refusal_and_a_rejection_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, _helper) = fake_helper(
            dir.path(),
            vec![
                (1, vec![welcome()]),
                (
                    1,
                    vec![Reply::Refused {
                        root: "/secret".into(),
                        reason: "Permission denied".into(),
                    }],
                ),
            ],
        );
        let mut client = Client::connect(&socket).unwrap();
        let error = client.subscribe(Path::new("/secret")).unwrap_err();
        assert!(matches!(error, HelperError::Refused { .. }), "{error}");

        let other = tempfile::tempdir().unwrap();
        let (socket, _helper) = fake_helper(
            other.path(),
            vec![(
                1,
                vec![Reply::Rejected {
                    reason: "go away".into(),
                }],
            )],
        );
        let error = Client::connect(&socket).unwrap_err();
        assert!(matches!(error, HelperError::Handshake(ref r) if r == "go away"));
    }

    /// A helper over its connection limit writes its refusal at once and
    /// hangs up without reading the handshake. The client still reads the
    /// reason, whichever of the two happened first.
    #[test]
    fn a_refusal_before_the_handshake_is_read_is_still_a_rejection() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("helper.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let helper = std::thread::spawn(move || {
            for _ in 0..20 {
                let (mut stream, _) = listener.accept().unwrap();
                Reply::Rejected {
                    reason: "over the limit".into(),
                }
                .write_to(&mut stream)
                .unwrap();
            }
        });
        for _ in 0..20 {
            let error = Client::connect(&socket).unwrap_err();
            assert!(
                matches!(error, HelperError::Handshake(ref r) if r == "over the limit"),
                "{error}"
            );
        }
        helper.join().unwrap();
    }

    #[test]
    fn a_wrong_version_is_a_handshake_error() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, _helper) = fake_helper(
            dir.path(),
            vec![(
                1,
                vec![Reply::Welcome {
                    version: VERSION + 1,
                    backend: "fanotify".into(),
                }],
            )],
        );
        assert!(matches!(
            Client::connect(&socket).unwrap_err(),
            HelperError::Handshake(_)
        ));
    }
}
