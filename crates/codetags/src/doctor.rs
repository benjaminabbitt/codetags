//! `codetags doctor`: what this machine can do, and what to fix.
//!
//! Exit status 1 means something is misconfigured that the user can fix.
//! A missing optional capability (e.g. no FUSE) is reported, not an error:
//! static views and the CLI work everywhere (D3).

use std::process::ExitCode;

/// A report: lines to print, and whether anything needs fixing.
struct Report {
    lines: Vec<String>,
    needs_fixing: bool,
}

pub fn run() -> ExitCode {
    let mut report = Report {
        lines: vec![format!("platform: {}", std::env::consts::OS)],
        needs_fixing: false,
    };
    mount_backend(&mut report);
    for line in &report.lines {
        println!("{line}");
    }
    if report.needs_fixing {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(target_os = "linux")]
fn mount_backend(report: &mut Report) {
    use codetags_mount_fuse::helper;

    report.lines.push("mount backend: fuse".to_string());
    match helper::locate_from_env() {
        None => {
            report
                .lines
                .push(format!("{}: not found on PATH", helper::FUSERMOUNT3));
            report.lines.push(
                "hint: install the fuse3 package to enable live mounts; static views work without it"
                    .to_string(),
            );
        }
        Some(found) => {
            let yes_no = if found.setuid_root { "yes" } else { "no" };
            report.lines.push(format!(
                "{}: {} (setuid root: {yes_no})",
                helper::FUSERMOUNT3,
                found.path.display()
            ));
            if !found.setuid_root {
                report.needs_fixing = true;
                report.lines.push(
                    "hint: mounts will fail with EPERM. Put a setuid-root fusermount3 first on \
                     PATH, or set FUSERMOUNT_PATH=/usr/bin/fusermount3"
                        .to_string(),
                );
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn mount_backend(report: &mut Report) {
    report
        .lines
        .push("mount backend: nfs loopback (not built yet: P0b S2)".to_string());
}

#[cfg(windows)]
fn mount_backend(report: &mut Report) {
    report
        .lines
        .push("mount backend: winfsp (not built yet: P0b S3)".to_string());
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn mount_backend(report: &mut Report) {
    report
        .lines
        .push("mount backend: none on this platform".to_string());
}
