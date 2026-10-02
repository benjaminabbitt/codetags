//! The helper's wire protocol, version [`VERSION`]: length-prefixed frames
//! over a Unix stream socket.
//!
//! Every frame is a little-endian `u32` length, then that many bytes: a tag
//! byte and the body. A frame is at most [`MAX_FRAME`] bytes. Paths are raw
//! bytes, so a path need not be UTF-8.
//!
//! | Direction | Tag | Body | Meaning |
//! |---|---|---|---|
//! | client → helper | `H` | `u16` version, then [`MAGIC`] | handshake |
//! | client → helper | `S` | root path | subscribe to a root |
//! | helper → client | `W` | `u16` version, then the backend's name (UTF-8) | handshake accepted |
//! | helper → client | `X` | reason (UTF-8) | handshake rejected; the helper closes the socket |
//! | helper → client | `A` | canonical root path | subscription accepted; events for it follow |
//! | helper → client | `R` | `u32` root length, root path, reason (UTF-8) | subscription refused |
//! | helper → client | `E` | kind byte, `u32` writer PID (0 if unknown), path | an event |
//! | helper → client | `O` | empty | events were lost: rescan |
//!
//! Replies to `S` come in the order the subscriptions were sent. Events for a
//! root come only after its `A`.

use std::fmt;
use std::io::{self, Read, Write};
use std::path::PathBuf;

/// The protocol version this crate speaks.
pub const VERSION: u16 = 1;

/// Follows the version in the handshake, so that a stray connection from
/// something else is rejected.
pub const MAGIC: &[u8] = b"codetags-privhelper";

/// The largest frame, tag included. Paths are at most `PATH_MAX` (4096) bytes
/// on Linux, so this leaves ample room.
pub const MAX_FRAME: usize = 64 * 1024;

/// What happened to an event's path, as fanotify reported it.
///
/// The watcher treats every kind as a hint to look at the path again, so a
/// kind is never more than advice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// Created (`FAN_CREATE`).
    Created,
    /// Deleted (`FAN_DELETE`, `FAN_DELETE_SELF`).
    Deleted,
    /// Written (`FAN_MODIFY`, `FAN_CLOSE_WRITE`).
    Modified,
    /// Renamed away (`FAN_MOVED_FROM`, `FAN_MOVE_SELF`).
    MovedFrom,
    /// Renamed into place (`FAN_MOVED_TO`).
    MovedTo,
    /// Metadata changed (`FAN_ATTRIB`).
    Attrib,
}

impl EventKind {
    fn to_byte(self) -> u8 {
        match self {
            EventKind::Created => 1,
            EventKind::Deleted => 2,
            EventKind::Modified => 3,
            EventKind::MovedFrom => 4,
            EventKind::MovedTo => 5,
            EventKind::Attrib => 6,
        }
    }

    fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            1 => EventKind::Created,
            2 => EventKind::Deleted,
            3 => EventKind::Modified,
            4 => EventKind::MovedFrom,
            5 => EventKind::MovedTo,
            6 => EventKind::Attrib,
            _ => return None,
        })
    }
}

/// One event the helper delivers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperEvent {
    /// The absolute path, under one of the client's accepted roots.
    pub path: PathBuf,
    /// What happened.
    pub kind: EventKind,
    /// The process that caused it, when the kernel reported one.
    ///
    /// It stops at the helper's client: the watcher drops it, and a
    /// [`crate::Batch`] carries no PIDs in v1 (PLAN.md D40).
    pub pid: Option<u32>,
}

/// A message from a client to the helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// The handshake, sent first.
    Hello {
        /// The client's protocol version.
        version: u16,
    },
    /// Subscribe to the events under `root`.
    Subscribe {
        /// An absolute path to a directory.
        root: PathBuf,
    },
}

/// A message from the helper to a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// The handshake succeeded.
    Welcome {
        /// The helper's protocol version.
        version: u16,
        /// The event source, e.g. `fanotify`.
        backend: String,
    },
    /// The handshake failed; the helper closes the connection.
    Rejected {
        /// Why.
        reason: String,
    },
    /// A subscription was accepted; events under `root` follow.
    Accepted {
        /// The root, canonicalized as the client's user.
        root: PathBuf,
    },
    /// A subscription was refused.
    Refused {
        /// The root as the client sent it.
        root: PathBuf,
        /// Why.
        reason: String,
    },
    /// An event under an accepted root.
    Event(HelperEvent),
    /// Events were lost: the client should rescan its roots.
    Overflow,
}

/// A frame that does not decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtoError(pub String);

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "privileged-helper protocol error: {}", self.0)
    }
}

impl std::error::Error for ProtoError {}

impl From<ProtoError> for io::Error {
    fn from(error: ProtoError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, error)
    }
}

/// Writes one frame: `tag`, then `body`.
pub fn write_frame(writer: &mut impl Write, tag: u8, body: &[u8]) -> io::Result<()> {
    let len = body.len() + 1;
    if len > MAX_FRAME {
        return Err(ProtoError(format!("a {len}-byte frame exceeds {MAX_FRAME} bytes")).into());
    }
    let mut frame = Vec::with_capacity(4 + len);
    frame.extend_from_slice(&u32::try_from(len).unwrap_or(u32::MAX).to_le_bytes());
    frame.push(tag);
    frame.extend_from_slice(body);
    writer.write_all(&frame)?;
    writer.flush()
}

/// Reads one frame as `(tag, body)`. `Ok(None)` means the stream ended
/// cleanly between frames.
pub fn read_frame(reader: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < len.len() {
        match reader.read(&mut len[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => got += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(ProtoError(format!("bad frame length {len}")).into());
    }
    let mut frame = vec![0u8; len];
    reader.read_exact(&mut frame)?;
    let tag = frame.remove(0);
    Ok(Some((tag, frame)))
}

fn path_bytes(path: &std::path::Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes()
}

fn bytes_path(bytes: &[u8]) -> Result<PathBuf, ProtoError> {
    use std::os::unix::ffi::OsStrExt;
    if bytes.is_empty() || bytes.contains(&0) {
        return Err(ProtoError("a path is empty or holds a NUL byte".into()));
    }
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn u16_at(body: &[u8]) -> Result<u16, ProtoError> {
    body.get(..2)
        .and_then(|b| b.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| ProtoError("a frame is too short for its version".into()))
}

fn u32_at(body: &[u8], at: usize) -> Result<u32, ProtoError> {
    body.get(at..at + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| ProtoError("a frame is too short".into()))
}

impl Request {
    /// Encodes the request as `(tag, body)`.
    pub fn encode(&self) -> (u8, Vec<u8>) {
        match self {
            Request::Hello { version } => {
                let mut body = version.to_le_bytes().to_vec();
                body.extend_from_slice(MAGIC);
                (b'H', body)
            }
            Request::Subscribe { root } => (b'S', path_bytes(root).to_vec()),
        }
    }

    /// Decodes a frame read with [`read_frame`].
    pub fn decode(tag: u8, body: &[u8]) -> Result<Self, ProtoError> {
        match tag {
            b'H' => {
                let version = u16_at(body)?;
                if &body[2..] != MAGIC {
                    return Err(ProtoError("the handshake lacks the magic string".into()));
                }
                Ok(Request::Hello { version })
            }
            b'S' => Ok(Request::Subscribe {
                root: bytes_path(body)?,
            }),
            other => Err(ProtoError(format!("unknown request tag {other:#04x}"))),
        }
    }

    /// Writes the request as one frame.
    pub fn write_to(&self, writer: &mut impl Write) -> io::Result<()> {
        let (tag, body) = self.encode();
        write_frame(writer, tag, &body)
    }
}

impl Reply {
    /// Encodes the reply as `(tag, body)`.
    pub fn encode(&self) -> (u8, Vec<u8>) {
        match self {
            Reply::Welcome { version, backend } => {
                let mut body = version.to_le_bytes().to_vec();
                body.extend_from_slice(backend.as_bytes());
                (b'W', body)
            }
            Reply::Rejected { reason } => (b'X', reason.as_bytes().to_vec()),
            Reply::Accepted { root } => (b'A', path_bytes(root).to_vec()),
            Reply::Refused { root, reason } => {
                let root = path_bytes(root);
                let mut body = u32::try_from(root.len())
                    .unwrap_or(u32::MAX)
                    .to_le_bytes()
                    .to_vec();
                body.extend_from_slice(root);
                body.extend_from_slice(reason.as_bytes());
                (b'R', body)
            }
            Reply::Event(event) => {
                let mut body = vec![event.kind.to_byte()];
                body.extend_from_slice(&event.pid.unwrap_or(0).to_le_bytes());
                body.extend_from_slice(path_bytes(&event.path));
                (b'E', body)
            }
            Reply::Overflow => (b'O', Vec::new()),
        }
    }

    /// Decodes a frame read with [`read_frame`].
    pub fn decode(tag: u8, body: &[u8]) -> Result<Self, ProtoError> {
        match tag {
            b'W' => Ok(Reply::Welcome {
                version: u16_at(body)?,
                backend: text(&body[2..]),
            }),
            b'X' => Ok(Reply::Rejected { reason: text(body) }),
            b'A' => Ok(Reply::Accepted {
                root: bytes_path(body)?,
            }),
            b'R' => {
                let len = u32_at(body, 0)? as usize;
                let root = body
                    .get(4..4 + len)
                    .ok_or_else(|| ProtoError("a refusal's root is truncated".into()))?;
                Ok(Reply::Refused {
                    root: bytes_path(root)?,
                    reason: text(&body[4 + len..]),
                })
            }
            b'E' => {
                let kind = body
                    .first()
                    .and_then(|&byte| EventKind::from_byte(byte))
                    .ok_or_else(|| ProtoError("an event has no valid kind".into()))?;
                let pid = u32_at(body, 1)?;
                Ok(Reply::Event(HelperEvent {
                    path: bytes_path(&body[5..])?,
                    kind,
                    pid: (pid != 0).then_some(pid),
                }))
            }
            b'O' => Ok(Reply::Overflow),
            other => Err(ProtoError(format!("unknown reply tag {other:#04x}"))),
        }
    }

    /// Writes the reply as one frame.
    pub fn write_to(&self, writer: &mut impl Write) -> io::Result<()> {
        let (tag, body) = self.encode();
        write_frame(writer, tag, &body)
    }

    /// Reads one reply; `Ok(None)` at a clean end of stream.
    pub fn read_from(reader: &mut impl Read) -> io::Result<Option<Self>> {
        match read_frame(reader)? {
            None => Ok(None),
            Some((tag, body)) => Ok(Some(Self::decode(tag, &body)?)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_request(request: Request) {
        let mut wire = Vec::new();
        request.write_to(&mut wire).unwrap();
        let (tag, body) = read_frame(&mut wire.as_slice()).unwrap().unwrap();
        assert_eq!(Request::decode(tag, &body).unwrap(), request);
    }

    fn round_trip_reply(reply: Reply) {
        let mut wire = Vec::new();
        reply.write_to(&mut wire).unwrap();
        assert_eq!(Reply::read_from(&mut wire.as_slice()).unwrap(), Some(reply));
    }

    #[test]
    fn requests_round_trip() {
        round_trip_request(Request::Hello { version: VERSION });
        round_trip_request(Request::Subscribe {
            root: "/home/u/project".into(),
        });
    }

    #[test]
    fn replies_round_trip() {
        round_trip_reply(Reply::Welcome {
            version: VERSION,
            backend: "fanotify".into(),
        });
        round_trip_reply(Reply::Rejected {
            reason: "version 2 is not supported".into(),
        });
        round_trip_reply(Reply::Accepted { root: "/p".into() });
        round_trip_reply(Reply::Refused {
            root: "/root/secret".into(),
            reason: "permission denied".into(),
        });
        round_trip_reply(Reply::Event(HelperEvent {
            path: "/p/src/lib.rs".into(),
            kind: EventKind::Modified,
            pid: Some(4242),
        }));
        round_trip_reply(Reply::Event(HelperEvent {
            path: "/p/a b".into(),
            kind: EventKind::Attrib,
            pid: None,
        }));
        round_trip_reply(Reply::Overflow);
    }

    #[test]
    fn non_utf8_paths_survive() {
        use std::os::unix::ffi::OsStrExt;
        let path = PathBuf::from(std::ffi::OsStr::from_bytes(b"/p/\xff\xfe.rs"));
        round_trip_reply(Reply::Event(HelperEvent {
            path,
            kind: EventKind::Created,
            pid: Some(1),
        }));
    }

    #[test]
    fn a_clean_end_of_stream_is_none_and_a_torn_frame_an_error() {
        assert!(read_frame(&mut [].as_slice()).unwrap().is_none());
        assert!(read_frame(&mut [5u8, 0].as_slice()).is_err());
        assert!(read_frame(&mut [5u8, 0, 0, 0, b'E'].as_slice()).is_err());
    }

    #[test]
    fn oversized_and_empty_frames_are_refused() {
        let huge = (MAX_FRAME as u32 + 1).to_le_bytes();
        assert!(read_frame(&mut huge.as_slice()).is_err());
        assert!(read_frame(&mut [0u8, 0, 0, 0].as_slice()).is_err());
        assert!(write_frame(&mut Vec::new(), b'S', &vec![b'a'; MAX_FRAME]).is_err());
    }

    #[test]
    fn bad_bodies_are_protocol_errors() {
        assert!(Request::decode(b'H', b"\x01\x00not-magic").is_err());
        assert!(Request::decode(b'S', b"").is_err());
        assert!(Request::decode(b'S', b"/a\0b").is_err());
        assert!(Request::decode(b'?', b"").is_err());
        assert!(Reply::decode(b'E', &[9, 0, 0, 0, 0, b'/']).is_err());
        assert!(Reply::decode(b'E', &[1, 0]).is_err());
        assert!(Reply::decode(b'R', &[50, 0, 0, 0, b'/']).is_err());
    }
}
