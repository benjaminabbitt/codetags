//! Mounting the spike's server with the built-in `mount_nfs`, as the user
//! who owns the mount point: no root, no kext, no install (C3, V43).

use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::options::{DISKUTIL, MOUNT_NFS, UMOUNT, mount_options};
use crate::spike::Server;

/// A mounted spike filesystem and the server behind it. Dropping it
/// unmounts and then stops the server, but [`Mounted::unmount`] reports
/// errors.
#[derive(Debug)]
pub struct Mounted {
    dir: PathBuf,
    mounted: bool,
    server: Server,
}

impl Mounted {
    /// The server, for changing what it serves ([`Server::add_file`]).
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// Unmounts, then stops the server. The order matters: the NFS client
    /// blocks on a server that has stopped answering.
    pub fn unmount(mut self) -> io::Result<()> {
        unmount_dir(&self.dir)?;
        self.mounted = false;
        Ok(())
    }
}

impl Drop for Mounted {
    fn drop(&mut self) {
        if self.mounted {
            let _ = unmount_dir(&self.dir);
        }
        // The server field drops after this, stopping the server.
    }
}

/// Starts the one-file server and mounts it read-only on `mountpoint`, an
/// existing empty directory the caller owns, with `extra` `mount_nfs`
/// options after the defaults ([`mount_options`]).
pub fn mount_hello(mountpoint: &Path, extra: &[&str]) -> io::Result<Mounted> {
    let owner = std::fs::metadata(mountpoint)?;
    let server = Server::start(owner.uid(), owner.gid())?;
    let source = format!("127.0.0.1:{}", server.export_path());
    let output = Command::new(MOUNT_NFS)
        .arg("-o")
        .arg(mount_options(server.port(), extra))
        .arg(&source)
        .arg(mountpoint)
        .output()?;
    check(MOUNT_NFS, &output)?;
    Ok(Mounted {
        dir: mountpoint.to_path_buf(),
        mounted: true,
        server,
    })
}

/// `umount <dir>`; if that fails, `diskutil unmount force <dir>`.
fn unmount_dir(dir: &Path) -> io::Result<()> {
    let output = Command::new(UMOUNT).arg(dir).output()?;
    let Err(umount_error) = check(UMOUNT, &output) else {
        return Ok(());
    };
    let output = Command::new(DISKUTIL)
        .args(["unmount", "force"])
        .arg(dir)
        .output()?;
    check(DISKUTIL, &output)
        .map_err(|error| io::Error::other(format!("{umount_error}; then {error}")))
}

fn check(tool: &str, output: &Output) -> io::Result<()> {
    if output.status.success() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "{tool} failed ({}): {}{}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim(),
        String::from_utf8_lossy(&output.stdout).trim()
    )))
}
