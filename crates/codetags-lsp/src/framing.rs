//! LSP base-protocol framing: a `Content-Length` header block, a blank line,
//! then the body.
//!
//! The reader keeps every byte it consumed ([`Frame::raw`]), so a proxy can
//! forward a message exactly as it arrived and never re-serializes a body.
//! It is tolerant in what it accepts: header names in any case, any other
//! header (such as `Content-Type`) kept but not interpreted, lines ending in
//! `\r\n` or a bare `\n`, and stray blank lines before a header block. What
//! it cannot frame it reports as [`FrameError::Malformed`] together with the
//! bytes it consumed, so the caller can still pass them on.

use std::io::{self, BufRead, Read, Write};

/// Longest header line accepted, in bytes, including its line ending.
const MAX_HEADER_LINE: u64 = 8 * 1024;
/// Largest body accepted, in bytes.
const MAX_BODY: usize = 1 << 30;

/// One framed message, as it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    raw: Vec<u8>,
    body_start: usize,
    headers: Vec<(String, String)>,
}

impl Frame {
    /// Every byte of the message: header block, blank line and body.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The message body, normally a JSON-RPC message in UTF-8.
    pub fn body(&self) -> &[u8] {
        &self.raw[self.body_start..]
    }

    /// The headers in order, names as written, values trimmed.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// The value of the header `name`, compared case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Why a message could not be read.
#[derive(Debug)]
pub enum FrameError {
    /// Reading the underlying stream failed.
    Io(io::Error),
    /// The input is not a well-formed message. `raw` holds every byte
    /// consumed while trying, which the caller should pass on unchanged.
    Malformed {
        /// The bytes consumed.
        raw: Vec<u8>,
        /// What was wrong.
        reason: String,
    },
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "read failed: {error}"),
            Self::Malformed { reason, .. } => write!(f, "malformed message: {reason}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Reads [`Frame`]s from a buffered stream.
#[derive(Debug)]
pub struct FrameReader<R> {
    inner: R,
}

impl<R: BufRead> FrameReader<R> {
    /// Wraps `inner`.
    pub fn new(inner: R) -> Self {
        Self { inner }
    }

    /// Returns the stream, with any bytes it buffered but this reader did not
    /// consume.
    pub fn into_inner(self) -> R {
        self.inner
    }

    /// Reads the next message. `Ok(None)` means the stream ended cleanly,
    /// between messages. Stray blank lines before a header block belong to
    /// the next frame's [`Frame::raw`]; at the end of input they are
    /// returned as [`FrameError::Malformed`], so no byte is lost.
    pub fn read_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        let mut raw = Vec::new();
        let mut headers = Vec::new();
        let mut length: Option<usize> = None;
        loop {
            let start = raw.len();
            let read = (&mut self.inner)
                .take(MAX_HEADER_LINE)
                .read_until(b'\n', &mut raw)
                .map_err(FrameError::Io)?;
            if read == 0 {
                if raw.is_empty() {
                    return Ok(None);
                }
                return Err(malformed(raw, "input ended inside a header block"));
            }
            let line = &raw[start..];
            if !line.ends_with(b"\n") {
                let reason = if read as u64 == MAX_HEADER_LINE {
                    "header line too long"
                } else {
                    "input ended inside a header line"
                };
                return Err(malformed(raw, reason));
            }
            let line = trim_line_end(line);
            if line.is_empty() {
                if headers.is_empty() {
                    continue; // a stray blank line between messages
                }
                break;
            }
            let Some((name, value)) = std::str::from_utf8(line)
                .ok()
                .and_then(|line| line.split_once(':'))
            else {
                return Err(malformed(raw, "header line is not `name: value`"));
            };
            let (name, value) = (name.trim().to_string(), value.trim().to_string());
            if name.eq_ignore_ascii_case("content-length") {
                let Ok(parsed) = value.parse::<usize>() else {
                    return Err(malformed(raw, "Content-Length is not a number"));
                };
                if length.is_some_and(|earlier| earlier != parsed) {
                    return Err(malformed(raw, "conflicting Content-Length headers"));
                }
                if parsed > MAX_BODY {
                    return Err(malformed(raw, "Content-Length too large"));
                }
                length = Some(parsed);
            }
            headers.push((name, value));
        }
        let Some(length) = length else {
            return Err(malformed(raw, "no Content-Length header"));
        };
        let body_start = raw.len();
        let read = (&mut self.inner)
            .take(length as u64)
            .read_to_end(&mut raw)
            .map_err(FrameError::Io)?;
        if read < length {
            return Err(malformed(raw, "input ended inside a body"));
        }
        Ok(Some(Frame {
            raw,
            body_start,
            headers,
        }))
    }
}

fn malformed(raw: Vec<u8>, reason: &str) -> FrameError {
    FrameError::Malformed {
        raw,
        reason: reason.to_string(),
    }
}

fn trim_line_end(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// Writes `body` as one message with only a `Content-Length` header.
pub fn write_frame(out: &mut impl Write, body: &[u8]) -> io::Result<()> {
    write!(out, "Content-Length: {}\r\n\r\n", body.len())?;
    out.write_all(body)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn reader(input: &[u8]) -> FrameReader<&[u8]> {
        FrameReader::new(input)
    }

    fn malformed_raw(result: Result<Option<Frame>, FrameError>) -> (Vec<u8>, String) {
        match result {
            Err(FrameError::Malformed { raw, reason }) => (raw, reason),
            other => panic!("expected a malformed message, got {other:?}"),
        }
    }

    #[test]
    fn reads_a_message_and_keeps_its_bytes() {
        let input = b"Content-Length: 2\r\n\r\n{}";
        let frame = reader(input).read_frame().unwrap().unwrap();
        assert_eq!(frame.raw(), input);
        assert_eq!(frame.body(), b"{}");
        assert_eq!(frame.header("content-length"), Some("2"));
    }

    #[test]
    fn length_counts_bytes_not_characters() {
        let body = "{\"a\":\"é\"}";
        let input = format!("Content-Length: {}\r\n\r\n{body}", body.len());
        let frame = reader(input.as_bytes()).read_frame().unwrap().unwrap();
        assert_eq!(frame.body(), body.as_bytes());
    }

    #[test]
    fn content_type_is_kept_in_either_order() {
        for input in [
            &b"Content-Length: 2\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{}"[..],
            &b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\nContent-Length: 2\r\n\r\n{}"[..],
        ] {
            let frame = reader(input).read_frame().unwrap().unwrap();
            assert_eq!(frame.body(), b"{}");
            assert_eq!(
                frame.header("Content-Type"),
                Some("application/vscode-jsonrpc; charset=utf-8")
            );
            assert_eq!(frame.raw(), input);
        }
    }

    #[test]
    fn bare_newlines_and_any_header_case_are_accepted() {
        let input = b"content-length:2\n\n{}";
        let frame = reader(input).read_frame().unwrap().unwrap();
        assert_eq!(frame.body(), b"{}");
        assert_eq!(frame.raw(), input);
    }

    #[test]
    fn consecutive_messages_are_read_in_turn() {
        let input = b"Content-Length: 1\r\n\r\naContent-Length: 1\r\n\r\nb";
        let mut reader = reader(input);
        assert_eq!(reader.read_frame().unwrap().unwrap().body(), b"a");
        assert_eq!(reader.read_frame().unwrap().unwrap().body(), b"b");
        assert!(reader.read_frame().unwrap().is_none());
    }

    #[test]
    fn stray_blank_lines_between_messages_are_kept_in_the_next_frame() {
        let input = b"\r\nContent-Length: 1\r\n\r\na";
        let frame = reader(input).read_frame().unwrap().unwrap();
        assert_eq!(frame.raw(), input);
        assert_eq!(frame.body(), b"a");
    }

    #[test]
    fn end_of_input_between_messages_is_clean() {
        assert!(reader(b"").read_frame().unwrap().is_none());
    }

    #[test]
    fn trailing_blank_lines_are_returned_not_dropped() {
        let (raw, _) = malformed_raw(reader(b"\r\n").read_frame());
        assert_eq!(raw, b"\r\n");
    }

    #[test]
    fn a_truncated_body_returns_the_bytes_read() {
        let input = b"Content-Length: 5\r\n\r\nab";
        let (raw, reason) = malformed_raw(reader(input).read_frame());
        assert_eq!(raw, input);
        assert!(reason.contains("body"), "{reason}");
    }

    #[test]
    fn a_missing_length_is_malformed() {
        let input = b"Content-Type: x\r\n\r\n{}";
        let (raw, reason) = malformed_raw(reader(input).read_frame());
        assert_eq!(raw, b"Content-Type: x\r\n\r\n");
        assert!(reason.contains("Content-Length"), "{reason}");
    }

    #[test]
    fn garbage_is_malformed_and_returned() {
        let (raw, _) = malformed_raw(reader(b"hello world\n").read_frame());
        assert_eq!(raw, b"hello world\n");
    }

    #[test]
    fn conflicting_lengths_are_malformed() {
        let input = b"Content-Length: 1\r\nContent-Length: 2\r\n\r\nab";
        malformed_raw(reader(input).read_frame());
    }

    #[test]
    fn written_frames_read_back() {
        let mut out = Vec::new();
        write_frame(&mut out, "{\"x\":\"é\"}".as_bytes()).unwrap();
        assert!(out.starts_with(b"Content-Length: 10\r\n\r\n"));
        let frame = reader(&out).read_frame().unwrap().unwrap();
        assert_eq!(frame.body(), "{\"x\":\"é\"}".as_bytes());
    }
}
