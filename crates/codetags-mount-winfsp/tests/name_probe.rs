//! P0b S4: which file names reach a WinFsp filesystem, and how.
//!
//! Mounts the spike filesystem in probe mode, then:
//!
//! 1. **Win32.** For each probe name, looks it up (`std::fs::metadata`, i.e.
//!    `CreateFileW` for attributes) and tries to create it (`CreateFileW` with
//!    `CREATE_NEW`), through a normal path and a `\\?\` verbatim path.
//! 2. **Tools.** Through Git for Windows' bash (MSYS2), runs `touch`, `cat`
//!    and `rg` on names containing each character Win32 forbids, plus a
//!    trailing dot and space; lists the mount with `ls`, `rg --files` and
//!    PowerShell's `Get-ChildItem`; and reports the MSYS2 runtime version.
//!    The filesystem serves each such name in its private-use spelling
//!    (`codetags_mount_winfsp::private_use`), so `cat` succeeds exactly when
//!    the tool sends that spelling.
//!
//! Each row records the result and the exact names the filesystem received,
//! with code points outside printable ASCII written `<U+XXXX>`. The tables
//! feed docs/verification.md and the path profile (P1.2, D15).
//!
//! Needs WinFsp and Git for Windows, so it is ignored by default;
//! `just test-mount` runs it on Windows. `CODETAGS_NAME_PROBE_OUT` names a
//! file to write the tables to as well; `CODETAGS_PROBE_BASH` overrides the
//! bash to use (default `%ProgramFiles%\Git\bin\bash.exe`).
#![cfg(windows)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use codetags_mount_winfsp::private_use::{PROBE_NAMES, code_points, private_use_spelling};
use codetags_mount_winfsp::spike::{Mode, Mounted, mount_hello};

/// Tagma's reserved characters, plus `|` and `?`, which Win32 also reserves.
const CHARACTERS: [char; 14] = [
    ':', '=', '<', '>', '~', '!', '/', '*', '(', ')', '"', '\\', '|', '?',
];

/// Windows device names, bare and with an extension.
const DEVICES: [&str; 10] = [
    "CON", "NUL", "AUX", "COM1", "LPT1", "CON.txt", "NUL.txt", "AUX.txt", "COM1.txt", "LPT1.txt",
];

/// Trailing dots and spaces, and a leading space for contrast.
const TRAILING: [&str; 5] = ["a.", "a..", "a ", "a. ", " a"];

fn win32_names() -> Vec<String> {
    let mut names = vec!["plain".to_string(), "%3A".to_string()];
    names.extend(CHARACTERS.iter().map(|c| format!("a{c}b")));
    names.extend(DEVICES.iter().map(|name| name.to_string()));
    names.extend(TRAILING.iter().map(|name| name.to_string()));
    names
}

/// One call and what came of it.
struct Outcome {
    result: String,
    received: Vec<String>,
}

fn describe(result: io::Result<()>) -> String {
    match result {
        Ok(()) => "ok".to_string(),
        Err(error) => match error.raw_os_error() {
            Some(code) => format!("error {code}"),
            None => format!("error {:?}", error.kind()),
        },
    }
}

/// Runs `call`, and collects the names the filesystem received meanwhile.
/// It leaves out the root, and the lookups that Git Bash's launcher makes for
/// its own `-c` script text (it probes the script as a path, adding `.exe`
/// and `.lnk`), which say nothing about the probed name.
fn observe(mounted: &Mounted, call: impl FnOnce() -> String) -> Outcome {
    mounted.take_names();
    let result = call();
    let mut received: Vec<String> = mounted
        .take_names()
        .into_iter()
        .filter(|name| name != "\\" && !name.contains("PROBE_NAME"))
        .collect();
    received.dedup();
    Outcome { result, received }
}

/// A markdown table cell: code-quoted, code points spelled out, with `|`
/// and backticks escaped.
fn cell(text: &str) -> String {
    format!(
        "`{}`",
        code_points(text).replace('|', "\\|").replace('`', "'")
    )
}

fn received_cell(received: &[String]) -> String {
    if received.is_empty() {
        "none".to_string()
    } else {
        received
            .iter()
            .map(|name| cell(name))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn bash() -> PathBuf {
    std::env::var_os("CODETAGS_PROBE_BASH").map_or_else(
        || {
            let program_files = std::env::var_os("ProgramFiles").expect("ProgramFiles is set");
            Path::new(&program_files).join(r"Git\bin\bash.exe")
        },
        PathBuf::from,
    )
}

/// What a tool printed and how it exited.
struct Run {
    status: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Run {
    /// `exit N`, plus the first line of stdout, or of stderr if it failed.
    fn summary(&self) -> String {
        let status = self
            .status
            .map_or_else(|| "killed".to_string(), |code| format!("exit {code}"));
        let output = if self.status == Some(0) {
            &self.stdout
        } else {
            &self.stderr
        };
        match output.lines().find(|line| !line.trim().is_empty()) {
            Some(line) => format!("{status}: {}", cell(line.trim())),
            None => status,
        }
    }

    fn lines(&self) -> Vec<String> {
        self.stdout
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    }
}

fn run(command: &mut Command) -> Run {
    let output = command.output().expect("start the probe tool");
    Run {
        status: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Runs `script` in Git Bash in `dir`, with `$PROBE_NAME` set to `name`. The
/// name goes through the environment, not the command line, so no quoting or
/// globbing touches it on the way in.
fn in_bash(dir: &Path, script: &str, name: &str) -> Run {
    run(Command::new(bash())
        .arg("-c")
        .arg(script)
        .env("PROBE_NAME", name)
        .current_dir(dir))
}

fn win32_table(mounted: &Mounted, plain_mount: &Path, verbatim_mount: &Path) -> (String, bool) {
    let mut table = String::from(
        "### Win32 calls\n\n| name | path form | lookup | lookup received | create | create received |\n|---|---|---|---|---|---|\n",
    );
    let mut plain_reached = false;
    for name in win32_names() {
        for (form, mount) in [("plain", plain_mount), ("verbatim", verbatim_mount)] {
            // Joined as text, so `/` and `\` inside a name stay as written.
            let path = PathBuf::from(format!("{}\\{name}", mount.display()));
            let lookup = observe(mounted, || describe(std::fs::metadata(&path).map(drop)));
            let create = observe(mounted, || describe(create_new(&path)));
            if name == "plain" && form == "plain" {
                plain_reached = lookup.received == ["\\plain"];
            }
            let _ = writeln!(
                table,
                "| {} | {form} | {} | {} | {} | {} |",
                cell(&name),
                lookup.result,
                received_cell(&lookup.received),
                create.result,
                received_cell(&create.received),
            );
        }
    }
    (table, plain_reached)
}

fn tools_table(mounted: &Mounted, mount: &Path) -> String {
    let mut table = String::from("### Git Bash (MSYS2), rg and PowerShell\n\n");
    let version = in_bash(mount, "uname -sr; rg --version | head -n 1", "");
    let _ = writeln!(table, "Versions: {}\n", cell(&version.lines().join("; ")));
    table.push_str(
        "| name | served as | `touch` | touch received | `cat` | cat received | `rg` from bash | rg received |\n|---|---|---|---|---|---|---|---|\n",
    );
    for name in PROBE_NAMES {
        let touch = observe(mounted, || {
            in_bash(mount, r#"touch -- "$PROBE_NAME""#, name).summary()
        });
        let cat = observe(mounted, || {
            in_bash(mount, r#"cat -- "$PROBE_NAME""#, name).summary()
        });
        let rg = observe(mounted, || {
            in_bash(mount, r#"rg -c private -- "$PROBE_NAME""#, name).summary()
        });
        let _ = writeln!(
            table,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            cell(name),
            cell(&private_use_spelling(name)),
            touch.result,
            received_cell(&touch.received),
            cat.result,
            received_cell(&cat.received),
            rg.result,
            received_cell(&rg.received),
        );
    }
    let listings = [
        ("`ls -1` in Git Bash", in_bash(mount, "ls -1", "")),
        (
            "`rg --files` from Git Bash",
            in_bash(mount, "rg --files", ""),
        ),
        (
            "`Get-ChildItem -Name` in PowerShell (UTF-8 output)",
            run(Command::new("pwsh")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); \
                     Get-ChildItem -Name -LiteralPath $env:PROBE_DIR",
                ])
                .env("PROBE_DIR", mount)),
        ),
    ];
    for (title, listing) in listings {
        let names: Vec<String> = listing.lines().iter().map(|line| cell(line)).collect();
        let _ = writeln!(
            table,
            "\n{title}: {}; {}",
            listing.summary(),
            names.join(", ")
        );
    }
    table
}

#[test]
#[ignore = "needs WinFsp and Git for Windows; `just test-mount` runs it on Windows (S4)"]
fn names_that_reach_the_filesystem() {
    let winfsp = codetags_mount_winfsp::load().expect("WinFsp is installed");
    let scratch = tempfile::tempdir().expect("scratch dir");
    // Verbatim (`\\?\C:\...`) and long: no 8.3 names in the probed paths.
    let base = std::fs::canonicalize(scratch.path()).expect("canonicalize scratch");
    let verbatim_mount = base.join("mnt");
    let plain_mount = PathBuf::from(
        verbatim_mount
            .to_str()
            .expect("scratch path is UTF-8")
            .strip_prefix(r"\\?\")
            .expect("canonicalize gives a verbatim path"),
    );
    let mounted = mount_hello(&winfsp, &plain_mount, Mode::Probe).expect("mount through WinFsp");

    let (win32, plain_reached) = win32_table(&mounted, &plain_mount, &verbatim_mount);
    let tools = tools_table(&mounted, &plain_mount);
    mounted.unmount().expect("unmount");

    let report = format!(
        "WinFsp {} at {}\n\n{win32}\n{tools}",
        winfsp.version(),
        winfsp.dll().display()
    );
    println!("{report}");
    if let Some(out) = std::env::var_os("CODETAGS_NAME_PROBE_OUT") {
        std::fs::write(Path::new(&out), &report).expect("write the probe report");
    }
    assert!(
        plain_reached,
        "the control name `plain` did not reach the filesystem as `\\plain`:\n{report}"
    );
}

fn create_new(path: &Path) -> io::Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(drop)
}
