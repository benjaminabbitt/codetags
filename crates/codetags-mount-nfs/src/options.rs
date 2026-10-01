//! The macOS tools and `mount_nfs` options the NFS backend uses. Plain data,
//! so it is tested on every OS; [`crate::mount`] runs the tools on macOS.
//!
//! Each option is checked against Apple's `mount_nfs(8)` and `mount_nfs.c`
//! (V43).

/// The built-in NFS mount helper. `mount -t nfs` runs the same binary.
pub const MOUNT_NFS: &str = "/sbin/mount_nfs";
/// Unmounts; the user who mounted may unmount.
pub const UMOUNT: &str = "/sbin/umount";
/// The fallback when `umount` fails: `diskutil unmount force <dir>`.
pub const DISKUTIL: &str = "/usr/sbin/diskutil";

/// The `-o` argument for mounting a server listening on `port`, plus
/// `extra` options (e.g. `nonegnamecache`).
///
/// - `vers=3,tcp`: NFSv3 over TCP only, so there is no fallback to v2 or UDP.
/// - `port`, `mountport`: the server answers both protocols on one port, so
///   no portmapper is needed.
/// - `nolocks`: no NLM locking, so no `rpc.statd` is needed on the server.
/// - `actimeo=0`: no attribute caching. The server cannot push invalidations.
/// - `rdonly`: the spike's filesystem is read-only.
/// - `nobrowse`: keep the mount out of the Finder sidebar.
///
/// No `resvport`: that needs root.
pub fn mount_options(port: u16, extra: &[&str]) -> String {
    let mut options = vec![
        "vers=3".to_string(),
        "tcp".to_string(),
        format!("port={port}"),
        format!("mountport={port}"),
        "nolocks".to_string(),
        "actimeo=0".to_string(),
        "rdonly".to_string(),
        "nobrowse".to_string(),
    ];
    options.extend(extra.iter().map(|option| (*option).to_string()));
    options.join(",")
}

#[cfg(test)]
mod tests {
    use super::mount_options;

    #[test]
    fn options_pin_nfsv3_over_tcp_on_the_servers_port_without_caching() {
        assert_eq!(
            mount_options(40123, &[]),
            "vers=3,tcp,port=40123,mountport=40123,nolocks,actimeo=0,rdonly,nobrowse"
        );
    }

    #[test]
    fn extra_options_come_last() {
        assert!(mount_options(1, &["nonegnamecache"]).ends_with(",nobrowse,nonegnamecache"));
    }
}
