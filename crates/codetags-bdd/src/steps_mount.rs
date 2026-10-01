//! Steps for live mounts. The Linux FUSE spike (P0b S1) is the first backend;
//! the macOS NFS loopback spike (P0b S2) the second, and the Windows WinFsp
//! spike (P0b S3) the third.

use cucumber::{given, then};

use crate::CodetagsWorld;

#[given(expr = "a fusermount3 that is not setuid root comes first on PATH")]
fn non_setuid_fusermount3_first_on_path(world: &mut CodetagsWorld) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let bin = world.scratch().join("shadow-bin");
        std::fs::create_dir_all(&bin).expect("create shadow bin dir");
        let helper = bin.join("fusermount3");
        std::fs::write(&helper, "#!/bin/sh\nexit 1\n").expect("write fake helper");
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755))
            .expect("make fake helper executable");
        let path = std::env::var_os("PATH").unwrap_or_default();
        let joined = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&path)))
            .expect("join PATH");
        world.env.push(("PATH".into(), joined));
        world.env_removed.push("FUSERMOUNT_PATH".into());
    }
    #[cfg(not(unix))]
    {
        let _ = world;
        panic!("fusermount3 exists only on Unix; tag this scenario @linux");
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use cucumber::{given, when};

    use crate::CodetagsWorld;

    #[given(expr = "the hello filesystem is mounted on an empty directory")]
    fn hello_is_mounted(world: &mut CodetagsWorld) {
        let dir = world.scratch().join("mnt");
        std::fs::create_dir_all(&dir).expect("create mount dir");
        let mounted = codetags_mount_fuse::spike::mount_hello(&dir)
            .expect("mount through fusermount3 (run `codetags doctor` if this fails with EPERM)");
        world.mount = Some(mounted);
        world.mount_dir = Some(dir);
    }

    #[when(expr = "the mount is unmounted")]
    fn the_mount_is_unmounted(world: &mut CodetagsWorld) {
        let mounted = world.mount.take().expect("a Given step mounted something");
        mounted.unmount().expect("unmount");
    }
}

/// The macOS NFS loopback spike (P0b S2), including its staleness probes.
#[cfg(target_os = "macos")]
mod macos {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use cucumber::{given, then, when};

    use crate::CodetagsWorld;

    fn mount(world: &mut CodetagsWorld, extra: &[&str]) {
        let dir = world.scratch().join("mnt");
        std::fs::create_dir_all(&dir).expect("create mount dir");
        let mounted = codetags_mount_nfs::mount::mount_hello(&dir, extra)
            .expect("mount through mount_nfs (run `codetags doctor` if this fails)");
        world.nfs_mount = Some(mounted);
        world.mount_dir = Some(dir);
    }

    fn mounted_path(world: &CodetagsWorld, name: &str) -> PathBuf {
        world
            .mount_dir
            .as_ref()
            .expect("a Given step mounted something")
            .join(name)
    }

    /// Whether a lookup of `path` finds it. Any error but ENOENT fails.
    fn exists(path: &std::path::Path) -> bool {
        match std::fs::symlink_metadata(path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => panic!("look up {}: {error}", path.display()),
        }
    }

    fn gains(world: &mut CodetagsWorld, name: &str, content: &str, bump: bool) {
        world
            .nfs_mount
            .as_ref()
            .expect("a Given step mounted something")
            .server()
            .add_file(name, content.as_bytes(), bump);
    }

    #[given(expr = "the hello filesystem is mounted on an empty directory")]
    fn hello_is_mounted(world: &mut CodetagsWorld) {
        mount(world, &[]);
    }

    #[given(
        expr = "the hello filesystem is mounted on an empty directory with the extra mount option {string}"
    )]
    fn hello_is_mounted_with(world: &mut CodetagsWorld, option: String) {
        mount(world, &[option.as_str()]);
    }

    #[when(expr = "the mount is unmounted")]
    fn the_mount_is_unmounted(world: &mut CodetagsWorld) {
        let mounted = world
            .nfs_mount
            .take()
            .expect("a Given step mounted something");
        mounted.unmount().expect("unmount");
    }

    #[given(expr = "{string} is missing from the mount")]
    fn is_missing(world: &mut CodetagsWorld, name: String) {
        let path = mounted_path(world, &name);
        assert!(!exists(&path), "{} exists", path.display());
    }

    #[when(expr = "the filesystem gains {string} holding {string}")]
    fn the_filesystem_gains(world: &mut CodetagsWorld, name: String, content: String) {
        gains(world, &name, &content, false);
    }

    #[when(expr = "the filesystem gains {string} holding {string} and bumps the directory's mtime")]
    fn the_filesystem_gains_and_bumps(world: &mut CodetagsWorld, name: String, content: String) {
        gains(world, &name, &content, true);
    }

    #[then(expr = "{string} is still missing from the mount after {int} seconds")]
    fn still_missing_after(world: &mut CodetagsWorld, name: String, seconds: u64) {
        let path = mounted_path(world, &name);
        let start = Instant::now();
        let mut lookups = 0u32;
        while start.elapsed() < Duration::from_secs(seconds) {
            lookups += 1;
            assert!(
                !exists(&path),
                "{name} appeared after {} ms ({lookups} lookups)",
                start.elapsed().as_millis()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!("s2: {name} still missing after {seconds} s ({lookups} lookups)");
    }

    #[then(expr = "{string} appears in the mount within {int} second(s)")]
    fn appears_within(world: &mut CodetagsWorld, name: String, seconds: u64) {
        let path = mounted_path(world, &name);
        let start = Instant::now();
        let mut lookups = 0u32;
        loop {
            lookups += 1;
            if exists(&path) {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(seconds),
                "{name} still missing after {seconds} s ({lookups} lookups)"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        eprintln!(
            "s2: {name} appeared after {} ms ({lookups} lookups)",
            start.elapsed().as_millis()
        );
    }
}

/// The Windows WinFsp spike (P0b S3). The mount point must not exist yet:
/// WinFsp creates the directory, and removes it again on unmount.
#[cfg(windows)]
mod windows {
    use cucumber::{given, when};

    use crate::CodetagsWorld;

    #[given(expr = "the hello filesystem is mounted on an empty directory")]
    fn hello_is_mounted(world: &mut CodetagsWorld) {
        use codetags_mount_winfsp::spike::{Mode, mount_hello};

        let dir = world.scratch().join("mnt");
        let winfsp = codetags_mount_winfsp::load()
            .unwrap_or_else(|why| panic!("WinFsp is needed for this scenario: {why}"));
        let mounted = mount_hello(&winfsp, &dir, Mode::ReadOnly).expect("mount through WinFsp");
        world.mount = Some(mounted);
        world.mount_dir = Some(dir);
    }

    #[when(expr = "the mount is unmounted")]
    fn the_mount_is_unmounted(world: &mut CodetagsWorld) {
        let mounted = world.mount.take().expect("a Given step mounted something");
        mounted.unmount().expect("unmount");
    }
}

#[then(expr = "the mount lists exactly {string}")]
fn the_mount_lists_exactly(world: &mut CodetagsWorld, expected: String) {
    let dir = world
        .mount_dir
        .as_ref()
        .expect("a Given step mounted something");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("list the mount")
        .map(|entry| {
            entry
                .expect("read a directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    let mut expected: Vec<String> = expected.split_whitespace().map(str::to_string).collect();
    expected.sort();
    assert_eq!(names, expected);
}

#[then(expr = "reading {string} through the mount gives {string}")]
fn reading_through_the_mount_gives(world: &mut CodetagsWorld, name: String, expected: String) {
    let dir = world
        .mount_dir
        .as_ref()
        .expect("a Given step mounted something");
    let content = std::fs::read_to_string(dir.join(&name)).expect("read through the mount");
    assert_eq!(content.trim_end(), expected);
}

#[then(expr = "the mount directory is empty again")]
fn the_mount_directory_is_empty_again(world: &mut CodetagsWorld) {
    let dir = world
        .mount_dir
        .as_ref()
        .expect("a Given step mounted something");
    // WinFsp removes the mount point directory it created; nothing is left.
    #[cfg(windows)]
    if !dir.exists() {
        return;
    }
    let leftovers: Vec<_> = std::fs::read_dir(dir)
        .expect("list the unmounted directory")
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}
