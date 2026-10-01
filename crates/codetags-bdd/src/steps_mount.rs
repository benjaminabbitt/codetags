//! Steps for live mounts. The Linux FUSE spike (P0b S1) is the first backend.

use cucumber::{given, then, when};

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
    use super::*;

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
    let leftovers: Vec<_> = std::fs::read_dir(dir)
        .expect("list the unmounted directory")
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}
