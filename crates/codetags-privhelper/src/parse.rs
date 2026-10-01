//! Parses the events a `FAN_REPORT_FID` fanotify group reads (fanotify(7)).
//!
//! nix's `Fanotify::read_events` returns only each event's metadata and
//! drops the information records that carry the file handles and names, so
//! the helper reads the raw buffer and parses it here, in safe code.
//!
//! Layout, all in native byte order:
//!
//! - `fanotify_event_metadata` (24 bytes): `event_len: u32`, `vers: u8`,
//!   `reserved: u8`, `metadata_len: u16`, `mask: u64`, `fd: i32`, `pid: i32`;
//! - then, up to `event_len`, information records, each starting with
//!   `info_type: u8`, `pad: u8`, `len: u16`. The FID types follow that with
//!   `fsid: [i32; 2]` and a `file_handle` (`handle_bytes: u32`,
//!   `handle_type: i32`, then the handle); `DFID_NAME` then holds a
//!   NUL-terminated name, padded.

use std::ffi::OsString;
use std::fmt;
use std::os::unix::ffi::OsStringExt;

/// `FANOTIFY_METADATA_VERSION`.
pub const METADATA_VERSION: u8 = 3;
/// The size of `fanotify_event_metadata`.
const METADATA_LEN: usize = 24;
/// `FAN_EVENT_INFO_TYPE_FID`: the object's own handle.
pub const INFO_FID: u8 = 1;
/// `FAN_EVENT_INFO_TYPE_DFID_NAME`: the parent directory's handle and the
/// entry's name.
pub const INFO_DFID_NAME: u8 = 2;
/// `FAN_EVENT_INFO_TYPE_DFID`: the parent directory's handle.
pub const INFO_DFID: u8 = 3;
/// The record header, `fsid` and the `file_handle` header.
const FID_HEADER_LEN: usize = 4 + 8 + 8;

/// One file-identifier record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fid {
    /// [`INFO_FID`], [`INFO_DFID_NAME`] or [`INFO_DFID`].
    pub info_type: u8,
    /// The filesystem, as `statfs` reports `f_fsid` (`val[0]` in the low
    /// half), which is how `statvfs`'s `f_fsid` packs it on 64-bit glibc.
    pub fsid: u64,
    /// `file_handle.handle_type`.
    pub handle_type: i32,
    /// `file_handle.f_handle`.
    pub handle: Vec<u8>,
    /// The entry's name, for [`INFO_DFID_NAME`]; `"."` means the directory
    /// itself.
    pub name: Option<OsString>,
}

/// One event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEvent {
    /// The `FAN_*` bits that happened.
    pub mask: u64,
    /// The process that caused it; 0 if hidden or unknown.
    pub pid: i32,
    /// Its file-identifier records; other record types are skipped.
    pub fids: Vec<Fid>,
}

/// A buffer the kernel would not have written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "malformed fanotify event: {}", self.0)
    }
}

fn bytes<const N: usize>(buf: &[u8], at: usize) -> Result<[u8; N], ParseError> {
    buf.get(at..at + N)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| ParseError(format!("{N} bytes at offset {at} run past the end")))
}

fn u16_at(buf: &[u8], at: usize) -> Result<u16, ParseError> {
    bytes(buf, at).map(u16::from_ne_bytes)
}

fn u32_at(buf: &[u8], at: usize) -> Result<u32, ParseError> {
    bytes(buf, at).map(u32::from_ne_bytes)
}

fn i32_at(buf: &[u8], at: usize) -> Result<i32, ParseError> {
    bytes(buf, at).map(i32::from_ne_bytes)
}

fn u64_at(buf: &[u8], at: usize) -> Result<u64, ParseError> {
    bytes(buf, at).map(u64::from_ne_bytes)
}

/// Parses every event in `buf`, the bytes one `read` returned.
pub fn parse(buf: &[u8]) -> Result<Vec<RawEvent>, ParseError> {
    let mut events = Vec::new();
    let mut at = 0;
    while at < buf.len() {
        let event_len = u32_at(buf, at)? as usize;
        let version = bytes::<1>(buf, at + 4)?[0];
        let metadata_len = u16_at(buf, at + 6)? as usize;
        if version != METADATA_VERSION {
            return Err(ParseError(format!(
                "metadata version {version}, not {METADATA_VERSION}"
            )));
        }
        if metadata_len < METADATA_LEN || event_len < metadata_len || at + event_len > buf.len() {
            return Err(ParseError(format!(
                "event length {event_len}, metadata length {metadata_len}, {} bytes left",
                buf.len() - at
            )));
        }
        let event = &buf[at..at + event_len];
        let mut fids = Vec::new();
        let mut record = metadata_len;
        while record < event_len {
            let info_type = event[record];
            let len = u16_at(event, record + 2)? as usize;
            if len < 4 || record + len > event_len {
                return Err(ParseError(format!(
                    "record length {len} at offset {record} of a {event_len}-byte event"
                )));
            }
            if matches!(info_type, INFO_FID | INFO_DFID_NAME | INFO_DFID) {
                fids.push(fid(info_type, &event[record..record + len])?);
            }
            record += len;
        }
        events.push(RawEvent {
            mask: u64_at(event, 8)?,
            pid: i32_at(event, 20)?,
            fids,
        });
        at += event_len;
    }
    Ok(events)
}

/// Parses one FID record, header included.
fn fid(info_type: u8, record: &[u8]) -> Result<Fid, ParseError> {
    if record.len() < FID_HEADER_LEN {
        return Err(ParseError(format!(
            "a {}-byte FID record is shorter than its header",
            record.len()
        )));
    }
    let fsid0 = u64::from(u32_at(record, 4)?);
    let fsid1 = u64::from(u32_at(record, 8)?);
    let handle_bytes = u32_at(record, 12)? as usize;
    let handle_type = i32_at(record, 16)?;
    let handle = record
        .get(FID_HEADER_LEN..FID_HEADER_LEN + handle_bytes)
        .ok_or_else(|| ParseError(format!("a {handle_bytes}-byte handle overruns its record")))?
        .to_vec();
    let name = (info_type == INFO_DFID_NAME).then(|| {
        let rest = &record[FID_HEADER_LEN + handle_bytes..];
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        OsString::from_vec(rest[..end].to_vec())
    });
    Ok(Fid {
        info_type,
        fsid: fsid0 | (fsid1 << 32),
        handle_type,
        handle,
        name,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds one event the way the kernel lays it out.
    pub(crate) fn event(mask: u64, pid: i32, records: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = records.concat();
        let len = (METADATA_LEN + body.len()) as u32;
        let mut out = Vec::new();
        out.extend_from_slice(&len.to_ne_bytes());
        out.push(METADATA_VERSION);
        out.push(0);
        out.extend_from_slice(&(METADATA_LEN as u16).to_ne_bytes());
        out.extend_from_slice(&mask.to_ne_bytes());
        out.extend_from_slice(&(-1i32).to_ne_bytes());
        out.extend_from_slice(&pid.to_ne_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// Builds one FID record, padded to 4 bytes as the kernel pads it.
    pub(crate) fn record(
        info_type: u8,
        fsid: [u32; 2],
        handle: &[u8],
        name: Option<&str>,
    ) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&fsid[0].to_ne_bytes());
        body.extend_from_slice(&fsid[1].to_ne_bytes());
        body.extend_from_slice(&(handle.len() as u32).to_ne_bytes());
        body.extend_from_slice(&1i32.to_ne_bytes());
        body.extend_from_slice(handle);
        if let Some(name) = name {
            body.extend_from_slice(name.as_bytes());
            body.push(0);
        }
        while (body.len() + 4) % 4 != 0 {
            body.push(0);
        }
        let mut out = vec![info_type, 0];
        out.extend_from_slice(&((body.len() + 4) as u16).to_ne_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn layout_constants_match_libc() {
        use nix::libc;
        assert_eq!(METADATA_LEN, size_of::<libc::fanotify_event_metadata>());
        assert_eq!(METADATA_VERSION, libc::FANOTIFY_METADATA_VERSION);
        assert_eq!(INFO_FID, libc::FAN_EVENT_INFO_TYPE_FID);
        assert_eq!(INFO_DFID_NAME, libc::FAN_EVENT_INFO_TYPE_DFID_NAME);
        assert_eq!(INFO_DFID, libc::FAN_EVENT_INFO_TYPE_DFID);
        assert_eq!(
            FID_HEADER_LEN,
            size_of::<libc::fanotify_event_info_fid>() + size_of::<libc::file_handle>()
        );
    }

    #[test]
    fn parses_events_with_names_and_object_fids() {
        let mut buf = event(
            0x100,
            42,
            &[
                record(
                    INFO_DFID_NAME,
                    [0xdead, 0xbeef],
                    &[1, 2, 3, 4, 5, 6, 7, 8],
                    Some("x.rs"),
                ),
                record(INFO_FID, [0xdead, 0xbeef], &[9; 12], None),
            ],
        );
        buf.extend(event(0x4000, 0, &[]));
        let events = parse(&buf).unwrap();
        assert_eq!(events.len(), 2);
        let first = &events[0];
        assert_eq!((first.mask, first.pid), (0x100, 42));
        assert_eq!(first.fids.len(), 2);
        assert_eq!(first.fids[0].fsid, 0xdead | (0xbeef << 32));
        assert_eq!(first.fids[0].handle, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(first.fids[0].name.as_deref(), Some("x.rs".as_ref()));
        assert_eq!(first.fids[1].info_type, INFO_FID);
        assert_eq!(first.fids[1].name, None);
        assert_eq!(events[1].mask, 0x4000);
        assert!(events[1].fids.is_empty());
    }

    #[test]
    fn other_record_types_are_skipped() {
        let mut pidfd = vec![4u8, 0];
        pidfd.extend_from_slice(&8u16.to_ne_bytes());
        pidfd.extend_from_slice(&3i32.to_ne_bytes());
        let buf = event(0x2, 7, &[pidfd, record(INFO_DFID, [1, 0], &[5; 8], None)]);
        let events = parse(&buf).unwrap();
        assert_eq!(events[0].fids.len(), 1);
        assert_eq!(events[0].fids[0].info_type, INFO_DFID);
    }

    #[test]
    fn malformed_buffers_are_errors() {
        let good = event(
            0x2,
            7,
            &[record(INFO_DFID_NAME, [1, 0], &[5; 8], Some("a"))],
        );
        // Truncated anywhere.
        for cut in 1..good.len() {
            assert!(parse(&good[..cut]).is_err(), "cut at {cut}");
        }
        // Wrong version.
        let mut bad = good.clone();
        bad[4] = 2;
        assert!(parse(&bad).is_err());
        // A record claiming more than the event holds.
        let mut bad = good.clone();
        bad[METADATA_LEN + 2] = 0xff;
        assert!(parse(&bad).is_err());
        // A handle claiming more than its record holds.
        let mut bad = good;
        bad[METADATA_LEN + 12] = 0xff;
        assert!(parse(&bad).is_err());
    }
}
