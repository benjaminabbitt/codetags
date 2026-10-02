//! `codetags-privhelper`: the optional privileged helper (PLAN.md P4.4, C2,
//! §2.8; brief §4.3, "Privileged mode: fanotify").
//!
//! codetags works unprivileged everywhere. This helper is only an
//! accelerator an admin may run: never required, and never more than a
//! faster event source for the watcher in `codetags-watch`, which falls back
//! to notify whenever the helper is absent, refuses, or goes away, and logs
//! which mode is active.
//!
//! # Linux: fanotify
//!
//! At startup the helper creates a fanotify group with `FAN_REPORT_FID` and,
//! on Linux 5.9 and later, `FAN_REPORT_DFID_NAME`, so events carry the
//! directory's file handle and the entry's name (V9, V86). When a client
//! subscribes to a root, the helper puts a `FAN_MARK_FILESYSTEM` mark on the
//! filesystem holding it, so it needs no per-directory watches and sees the
//! writer's PID, which unprivileged fanotify hides. Each event's handle is
//! turned back into a path with `open_by_handle_at` and `/proc/self/fd`, and
//! sent to every client whose root holds it, as (path, kind, writer PID).
//!
//! It needs `CAP_SYS_ADMIN` (the filesystem mark), `CAP_DAC_READ_SEARCH`
//! (`open_by_handle_at`), and `CAP_SETUID` and `CAP_SETGID` (the checkers
//! below): run it as root.
//!
//! # Protocol
//!
//! A Unix stream socket, `/run/codetags/privhelper.sock` unless `--socket`
//! or `CODETAGS_PRIVHELPER_SOCKET` names another, mode 0666: anyone may
//! connect, and what each client receives is decided per client. The wire
//! format is versioned and narrow: a handshake, subscriptions, and then
//! events and overflow notices only
//! ([`codetags_watch::privhelper::proto`]).
//!
//! # Security model
//!
//! - **Identity.** Each client is identified by `SO_PEERCRED` (its uid and
//!   gid, from the kernel), and its supplementary groups come from the group
//!   database for that uid.
//! - **Access is checked as the client.** For each client the helper starts a
//!   *checker*: this binary, re-executed through `/proc/self/exe` (so the
//!   very file running, even if the path is replaced), which sets its
//!   groups, gid and uid to the client's (real, effective and saved), checks
//!   that root cannot be regained, and makes itself non-dumpable before it
//!   reads any request. A root is accepted only if the checker can list it;
//!   it is canonicalized there, as the client. Every event is then sent only
//!   if the checker can list the directory holding its path (for the root
//!   itself, the root). So the kernel applies the permission bits, ACLs and
//!   LSM rules exactly as for the client. The aim is that a root helper never
//!   shows one user another user's file names, even inside a root the client
//!   owns; `docs/privhelper-threat-model.md` records where that still falls
//!   short.
//! - **What a client learns.** Paths it could list anyway, the kind of
//!   change, and the writer's PID. PIDs are visible in `/proc` to every user
//!   unless `/proc` is mounted with `hidepid`; on such a system, note that
//!   the helper reveals the PID of a process that wrote into a directory
//!   the client can list. The watcher drops PIDs: its batches carry none in
//!   v1 (PLAN.md D40).
//! - **What it never does.** It never executes project code, and never
//!   writes to a project: it opens roots read-only to mark them and decodes
//!   handles with `O_PATH`. It never follows a client's path as root: the
//!   root it marks is the one the checker canonicalized, opened with
//!   `O_NOFOLLOW`, and every event path is re-checked as the client.
//! - **The socket.** Only an existing *socket* at the path is replaced, and
//!   only if nothing answers on it; any other file is left alone and the
//!   helper refuses to start. The helper exits if its socket is removed or
//!   replaced.
//! - **Mount namespaces.** Paths are as the helper sees them. A client in
//!   another mount namespace (a container) whose paths differ receives
//!   nothing for them, and falls back to notify.
//!
//! # Running it
//!
//! Nobody is told to install this helper until its threat model,
//! `docs/privhelper-threat-model.md`, is signed off (PLAN.md D40). That
//! document lists the open findings and the operational requirements. The
//! `privileged-linux` CI job runs it under `sudo -n` to test it. Stopping it
//! is always safe: watchers switch to notify and rescan.
//!
//! # Windows: the USN change journal (TODO, P4.4 part 2)
//!
//! Not built yet; on Windows this binary only says so. The plan:
//!
//! - A service running as LocalSystem opens each volume holding a
//!   subscribed root (`\\.\C:`) and reads its change journal with
//!   `FSCTL_QUERY_USN_JOURNAL` and `FSCTL_READ_USN_JOURNAL`, which need
//!   Administrators (V10). Records name the file by file reference number
//!   and parent reference number; paths come from `OpenFileById` and
//!   `GetFinalPathNameByHandleW`, with a cache of directory references.
//!   Reason flags map to the same event kinds. The USN record carries no
//!   PID, so Windows events have none.
//! - The channel is a named pipe, `\\.\pipe\codetags-privhelper`, carrying
//!   the same protocol. The client is identified with
//!   `GetNamedPipeClientProcessId` and `ImpersonateNamedPipeClient`, and
//!   access is checked while impersonating it (`AccessCheck` on each event's
//!   parent directory), the counterpart of the Linux checker.
//! - A journal wrap (`ERROR_JOURNAL_ENTRY_DELETED`) or a journal reset is
//!   sent as an overflow, so clients rescan.
//! - On the client side, `codetags-watch`'s `HelperSocket::Default` starts
//!   trying the pipe on Windows once this exists.

#[cfg(target_os = "linux")]
mod checker;
#[cfg(target_os = "linux")]
mod fanotify;
#[cfg(target_os = "linux")]
mod handle;
#[cfg(target_os = "linux")]
mod parse;
#[cfg(target_os = "linux")]
mod server;

const USAGE: &str = "\
usage: codetags-privhelper [--socket PATH]

The optional privileged helper for codetags: run it as root. It serves
fanotify events on a Unix socket (default /run/codetags/privhelper.sock, or
$CODETAGS_PRIVHELPER_SOCKET) to codetags watchers, filtered per client.
codetags works without it.";

fn main() {
    std::process::exit(run());
}

#[cfg(target_os = "linux")]
fn run() -> i32 {
    use std::path::PathBuf;

    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    if args.first().map(String::as_str) == Some(checker::CHECKER_ARG) {
        return checker::run(&args[1..]);
    }
    let mut socket = std::env::var_os(codetags_watch::privhelper::SOCKET_ENV)
        .filter(|value| !value.is_empty())
        .map_or_else(
            || PathBuf::from(codetags_watch::privhelper::DEFAULT_SOCKET),
            PathBuf::from,
        );
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--socket") => match args.next() {
                Some(path) => socket = PathBuf::from(path),
                None => return usage_error("--socket needs a path"),
            },
            Some("--help" | "-h") => {
                println!("{USAGE}");
                return 0;
            }
            Some("--version") => {
                println!("codetags-privhelper {}", env!("CARGO_PKG_VERSION"));
                return 0;
            }
            _ => return usage_error(&format!("unknown argument {arg:?}")),
        }
    }
    match server::serve(&server::Options { socket }) {
        Ok(()) => 0,
        Err(error) => {
            server::log(error);
            1
        }
    }
}

#[cfg(target_os = "linux")]
fn usage_error(message: &str) -> i32 {
    eprintln!("codetags-privhelper: {message}\n\n{USAGE}");
    2
}

#[cfg(not(target_os = "linux"))]
fn run() -> i32 {
    eprintln!(
        "codetags-privhelper: there is no privileged helper for this OS yet (the Windows USN \
         journal helper is planned, P4.4); codetags works without it\n\n{USAGE}"
    );
    2
}
