//! Turns a fanotify file handle back into a path: `open_by_handle_at(2)`,
//! then `/proc/self/fd`.
//!
//! This is the crate's only unsafe code. Neither nix 0.31 nor rustix wraps
//! `open_by_handle_at`, and a `FAN_REPORT_FID` group reports file handles
//! rather than descriptors, so there is no safe way to resolve them. The
//! unsafe part is the one system call, and adopting the descriptor it
//! returns.
#![allow(unsafe_code)]

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use nix::libc;

/// The size of `struct file_handle` before its flexible `f_handle` array.
const FILE_HANDLE_HEADER: usize = 8;

/// Opens the object a file handle names, with `O_PATH`, on the filesystem
/// that `mount` is on.
///
/// `mount` must not be an `O_PATH` descriptor: the kernel looks it up with
/// `fdget`, which ignores those, and fails with `EBADF`. Needs
/// `CAP_DAC_READ_SEARCH`; without it the call fails with `EPERM`.
pub fn open(mount: BorrowedFd<'_>, handle_type: i32, handle: &[u8]) -> io::Result<OwnedFd> {
    let handle_bytes = u32::try_from(handle.len())
        .ok()
        .filter(|&n| n as usize <= libc::MAX_HANDLE_SZ as usize)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "file handle too long"))?;
    // `struct file_handle` is { u32 handle_bytes; i32 handle_type; u8
    // f_handle[]; }, aligned to 4. A Vec<u32> gives that alignment.
    let words = (FILE_HANDLE_HEADER + handle.len()).div_ceil(4);
    let mut buffer = vec![0u32; words];
    buffer[0] = handle_bytes;
    buffer[1] = u32::from_ne_bytes(handle_type.to_ne_bytes());
    for (k, byte) in handle.iter().enumerate() {
        let at = FILE_HANDLE_HEADER + k;
        let word = &mut buffer[at / 4];
        let mut word_bytes = word.to_ne_bytes();
        word_bytes[at % 4] = *byte;
        *word = u32::from_ne_bytes(word_bytes);
    }
    // SAFETY: `buffer` is a live, 4-aligned allocation of at least
    // FILE_HANDLE_HEADER + handle_bytes bytes, laid out as `struct
    // file_handle` with `handle_bytes` equal to the length of the handle that
    // follows, so the kernel reads only within it (it reads the header, then
    // exactly `handle_bytes` bytes). The kernel does not write to it.
    // `mount` is a valid descriptor for the duration of the call.
    let fd = unsafe {
        libc::open_by_handle_at(
            mount.as_raw_fd(),
            buffer.as_mut_ptr().cast::<libc::file_handle>(),
            libc::O_PATH | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is a new descriptor the call just returned, owned by
    // nothing else; OwnedFd closes it exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The path of an open descriptor, from `/proc/self/fd`. `None` if the
/// object has been deleted since, when the kernel appends ` (deleted)`.
pub fn path_of(fd: &OwnedFd) -> Option<PathBuf> {
    let link = std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).ok()?;
    let bytes = link.as_os_str().as_bytes();
    if bytes.ends_with(b" (deleted)") || !bytes.starts_with(b"/") {
        return None;
    }
    Some(link)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::os::fd::AsFd;

    #[test]
    fn an_oversized_handle_is_refused_before_the_call() {
        let dir = tempfile::tempdir().unwrap();
        let mount = File::open(dir.path()).unwrap();
        let error = open(mount.as_fd(), 1, &[0; 200]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_bogus_handle_is_an_os_error_not_a_crash() {
        // Unprivileged this is EPERM; as root, ESTALE or EINVAL. Either way
        // an error, and the buffer handling is exercised.
        let dir = tempfile::tempdir().unwrap();
        let mount = File::open(dir.path()).unwrap();
        assert!(open(mount.as_fd(), 1, &[0xab; 8]).is_err());
    }

    #[test]
    fn path_of_reads_the_proc_link() {
        let dir = tempfile::tempdir().unwrap();
        let file = File::open(dir.path()).unwrap();
        let fd = OwnedFd::from(file);
        assert_eq!(
            path_of(&fd).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );
        let path = dir.path().join("gone");
        std::fs::write(&path, "x").unwrap();
        let fd = OwnedFd::from(File::open(&path).unwrap());
        std::fs::remove_file(&path).unwrap();
        assert_eq!(path_of(&fd), None);
    }
}
