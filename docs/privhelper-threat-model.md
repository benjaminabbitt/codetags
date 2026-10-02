# Threat model: `codetags-privhelper`

**Status: draft for the human's sign-off (PLAN.md D40). Not signed off.**
Until this document is signed off, no doc, README or `codetags doctor`
output suggests installing the helper. The two findings that blocked
sign-off, F1 and F2, are fixed, the denial-of-service limits are built, and
F3, a finding made during that work, is fixed too (§6.1, §6.5).

- **Scope:** the Linux fanotify helper (`crates/codetags-privhelper`) and its
  client in the watcher (`crates/codetags-watch/src/privhelper/`), at the
  commit that hardens the helper (H1-H3) and fixes F3. The Windows USN-journal helper is not built
  yet (P4.4 part 2). It needs its own section here before it ships.
- **Reviewed:** the code itself, not only its doc comments. Every mitigation
  below names the function that implements it, and the test that covers it
  when one does.
- **Background:** PLAN.md C2, §2.8 and P4.4; the security model in
  `crates/codetags-privhelper/src/main.rs`; V9, V86 and V90 in
  `docs/verification.md`.

## 1. What the helper is

codetags watches files without privilege everywhere, using notify. The helper is an
optional accelerator. Run as root, it puts one fanotify mark on each whole
filesystem (`FAN_MARK_FILESYSTEM`), in place of thousands of inotify watches. It sends each
connected watcher the changes under that watcher's project root. The watcher
falls back to notify when the helper is absent, refuses a request, or goes away
(`Watcher::start`, `helper_notice` in `crates/codetags-watch/src/watcher.rs`).

```
 client process (uid U)                 helper (root)                       kernel
 ┌──────────────────────┐  0666 Unix    ┌──────────────────────────────┐
 │ codetags-watch       │  socket       │ accept thread                │
 │  privhelper::Client  │◄────────────► │ per client: requests, events │◄── fanotify group
 └──────────────────────┘  frames       │ fanotify thread (dispatch)   │    (FID, DFID_NAME)
                                        │ watchdog thread              │
                                        └───────┬──────────────────────┘
                                                │ stdin/stdout frames
                                        ┌───────▼──────────────────────┐
                                        │ checker (uid U, non-dumpable)│
                                        │ /proc/self/exe --checker     │
                                        └──────────────────────────────┘
```

## 2. Assets

| # | Asset | Why it matters |
|---|---|---|
| A1 | **Root on the host** | The helper runs as root with `CAP_SYS_ADMIN`, `CAP_DAC_READ_SEARCH`, `CAP_SETUID` and `CAP_SETGID`. Anything that steers what root does is a privilege escalation. |
| A2 | **Names of files in directories a user cannot list** | Without read and search permission on a directory, a user cannot learn its entries. The helper sees every name on the filesystems it marks. |
| A3 | **Who wrote what, and when** | Each event carries the writer's PID. Users can usually see PIDs in `/proc`, but not when it is mounted with `hidepid`. The timing of writes is also activity data. |
| A4 | **Integrity of a watcher's view** | A watcher that misses or invents changes keeps an index that is stale or wrong. Every view and proxy answer depends on it. |
| A5 | **Availability of the host** | The helper is a root process that local users can reach. Its threads, processes, descriptors and memory come from the host. |
| A6 | **The files under marked filesystems** | The helper must never write to them or run code from them (PLAN.md §0.10). |

## 3. Actors

| # | Actor | Capabilities assumed |
|---|---|---|
| T1 | **An unprivileged local user** | Can connect to the 0666 socket, open any number of connections, send arbitrary frames, and subscribe to any directory they can list. Can create, rename and remove files wherever they have write permission, including racing those changes against the helper. Can signal their own processes. |
| T2 | **A compromised or malicious client process** | Runs as some user U, and may be codetags itself after a compromise. It has T1's powers as U. It also controls when it reads its socket, so it can make the helper's work for it run late. |
| T3 | **Hostile project contents** | A cloned repository, or a shared directory that other users write to. Names can hold any byte except `/` and NUL, including newlines and text such as ` (deleted)`. Directories can hold symlinks. Mount points inside roots are set up by root (or inside user namespaces the helper does not share). |
| T4 | **A replaced binary on disk** | Someone who can write the helper's installed path, or the directories above it. |
| T5 | **Someone impersonating the helper** | A process listening on the socket path a client will try. |

Out of scope: root and the kernel, who are trusted. Also out of scope is a user
with `CAP_SYS_ADMIN` in the initial user namespace, who already has more
power than the helper has.

## 4. Entry points

| # | Entry point | Code |
|---|---|---|
| E1 | **The Unix socket**, mode 0666, `/run/codetags/privhelper.sock` by default (or `--socket`, or `CODETAGS_PRIVHELPER_SOCKET`). Setting it up: create the parent if missing, check it (root-owned, not a symlink, writable by no one else) and open it, replace a stale socket relative to that descriptor, then bind through it with a umask that gives the mode. | `socket::bind`, `socket::open_dir`, `server::watchdog`, `main::run` |
| E2 | **The handshake**: an `H` frame (version and magic). A frame is at most 64 KiB, framed by a `u32` length. | `proto::read_frame`, `proto::Request::decode`, `server::serve_client` |
| E3 | **Subscriptions**: `S` frames carrying a root path (raw bytes, no NUL). | `server::read_requests`, `checker::Checker::check_root`, `limits::Subscriptions`, `server::Registry::add` |
| E4 | **fanotify events**: raw buffers from the kernel, parsed in safe code. | `server::read_events`, `parse::parse`, `parse::fid`, `server::Registry::dispatch` |
| E5 | **Handle decoding**: `open_by_handle_at` on the root's descriptor, then `readlink /proc/self/fd/N`. | `handle::open` (the crate's only `unsafe`), `handle::path_of`, `server::resolve` |
| E6 | **The checker's re-exec**: `/proc/self/exe --checker UID GID GROUPS` with a cleared environment. It drops to the client's user, and answers `R` and `P` frames on its stdin and stdout. A `P` frame carries the event's path and its binding (§6.1, F1). | `checker::Checker::spawn`, `checker::run`, `checker::drop_privileges`, `checker::answer` |
| E7 | **The client side**: `codetags-watch` trusts the socket it connects to. | `privhelper::HelperSocket::resolve`, `privhelper::Client`, `watcher::subscribe` |

## 5. Trust boundaries

| # | Boundary | What crosses it |
|---|---|---|
| B1 | **Any local user → root**, at the socket (E1–E3). | Frames, connection counts, and the client's credentials as `SO_PEERCRED` reports them. Everything on the client side is untrusted. |
| B2 | **Kernel → helper** (E4, E5). | Event records, file handles, and the paths `/proc/self/fd` gives. The kernel is trusted. But a path is a *name at a moment*: users in T1 and T3 can change the filesystem it names before anyone uses it. |
| B3 | **Helper (root) → checker (uid U)** (E6). | The helper decides what U may see by asking a process that runs as U, so that the kernel applies U's permissions, ACLs and LSM rules. While it runs, the checker is inside U's reach (signals). It is non-dumpable to keep it out of ptrace reach. |
| B4 | **Helper → client** (E7). | Events: path, kind, writer PID. Overflow notices. |
| B5 | **Disk → helper image** (T4). | The bytes `systemd` executes. The checker re-executes *the running image* through `/proc/self/exe`, not the path. |

## 6. Threats and mitigations

Each threat lists its mitigations, the code that implements them, and the
test that covers them. **Status:** *mitigated*, *partial* (some residual
risk, see §7), or *open* (blocks sign-off).

### 6.1 Findings

| # | Class | Status |
|---|---|---|
| F1 | **An access-check race (time-of-check to time-of-use) in per-event filtering.** The helper resolved an event's file handle to a path, and the client's checker later decided by that *path* whether the client could list the directory holding it. A user who can rename entries in a directory holding a directory they cannot list (for example, root's 0700 `secret/` inside the user's own project) could rename it away between the two, and make a directory they can list under the same name. The checker then approved the old path, and the user learned the names of entries in `secret/` (A2). Holding their connection unread made the window as long as they liked. | **fixed** (H3) |
| F2 | **The socket setup trusted the directory it was in.** Root created the parent, removed a stale socket, bound, and then `chmod`ed the socket *by path*. In a directory another user could write, that user could swap the socket for a symlink between `bind` and `chmod` and have root make any file mode 0666 (A1). The default path (`/run/codetags/`, root-owned) and a systemd `RuntimeDirectory=` were not affected, but the helper did not refuse an unsafe location, and the BDD harness used one. | **fixed** (H1) |
| F3 | **A name disclosure through directories the client may search but not list.** Found while fixing F1. An event's path names an entry in *every* directory between the root and the event, but the checker required read permission only on the directory holding the path, and search permission (enough to reach it) on those above. With a traverse-only directory (for example root's 0711 `tunnel/`) inside a root the client can list, and a listable directory below it (`tunnel/open/`), an event in `tunnel/open/` revealed the name `open`, an entry of `tunnel/` that the client cannot list (A2). | **fixed** |

**F1, how it is fixed.** The decision is bound to the object the event came
from, not to its name. When `server::resolve` turns a handle into a path, it
also records a `checker::Binding`, taken from the descriptor the handle
opened, never from the path: `Binding::Dir` with the holding directory's
`(st_dev, st_ino)` when the event names an entry in a directory, and
`Binding::Entry` with the object's own identity when it names the object
itself (a `.` record, or a `FID` record on kernels before 5.9). An event on
the root itself is bound to the root's open descriptor. The binding travels
with the event (`server::Delivery::Event`) and in the `P` frame
(`checker::path_request`). The checker, as the client, reaches the holding
directory with `O_PATH|O_DIRECTORY|O_NOFOLLOW` (since F3's fix, by walking
down from the root, below), checks read and search permission *on that
descriptor* (`faccessat(fd, ".", R_OK|X_OK, AT_EACCESS)`,
`checker::listable`), and answers yes only if its identity is the recorded
one (for `Entry`, if the entry under the path's last name in
it is the recorded object, by `fstatat(..., AT_SYMLINK_NOFOLLOW)`). Anything
else is a no (`checker::may_see`). A rename after the check does not matter:
the name was in a directory the client could list when it was checked.
Tests: `checker::tests::a_path_resolving_to_another_directory_is_refused`
(two directories, one's identity recorded and the other's path checked, and
a real rename swap), `checker::tests::an_object_is_seen_only_where_its_path_finds_it`,
`checker::tests::path_requests_round_trip`, the checker binary in
`tests/unprivileged.rs` (`the_checker_answers_as_the_current_user`), and the
privileged scenario "A path renamed after its event is checked against the
directory the event came from", which stalls the client's connection so the
rename lands between resolution and check (V178). The queue limit (§6.5)
also bounds how long that window can be.

**F2, how it is fixed.** `socket::open_dir` refuses, with an error naming
the directory, unless the socket's directory is a real directory (`lstat`
says it is not a symlink, and it is opened `O_DIRECTORY|O_NOFOLLOW`), owned
by root (or, for a helper that is not root, by its own user), and not
writable by group or other (`fstat` of the opened descriptor). Everything
after that works on the opened directory: the existing entry is examined
with `fstatat` and removed with `unlinkat` on its descriptor, and the socket
is bound through `/proc/self/fd/N/<name>`, so a rename after the check cannot
redirect any step. The socket gets mode 0666 from a umask of 0111 set around
`bind` alone (the process has one thread then), and there is no `chmod`
(`socket::bind`, V175, V176). Only an existing socket is replaced, and only
if nothing answers on it, as before. The BDD harness now puts the socket in a
root-owned 0755 directory made with `sudo -n install -d`
(`steps_privhelper::HelperProcess::start`). Tests: `socket::tests`
(group- or other-writable refused, symlink refused, someone else's directory
refused, root-owned 0755 `/` accepted, a file refused); in
`tests/unprivileged.rs`, `the_socket_is_created_with_mode_0666`,
`a_socket_directory_others_can_write_is_refused`,
`a_symlinked_socket_directory_is_refused`,
`a_stale_socket_is_replaced_but_a_live_one_is_not` and
`a_non_socket_file_is_never_replaced`; and the privileged scenario "The
helper refuses to put its socket in a directory another user can change",
which runs it as root on the runner's scratch directory. The unit tests
also run as root in the `privileged-linux` job (`just test-privileged
unit-as-root`), so the root-only branch of
`socket::tests::a_root_owned_0755_directory_is_accepted` (a root-owned
scratch directory) runs there (V180).

**F3, how it is fixed.** An event may reveal a path's name components only
where the client could list each directory holding them. The checker now
reaches the holding directory by walking from the accepted root, one
component at a time, from open descriptor to open descriptor (`openat(fd,
name, O_PATH|O_DIRECTORY|O_NOFOLLOW)`, never following a symlink), and
requires read and search permission on the root and on every directory it
reaches (`checker::open_listable_from`, `checker::listable`). If any is
missing, or a component is not a plain name, the event is refused. The H3
binding is kept: the directory the walk ends on must have the recorded
identity (`checker::may_see`). Because each step opens a child of the
previous descriptor, all the checks are of one real chain of directories,
whatever the path names by then. The same walk serves events on the root
itself. Tests: `checker::tests::every_directory_from_the_root_down_must_be_listable`
(a traverse-only directory between the root and a listable subdirectory
refuses events in that subdirectory, an event naming the subdirectory, and
an object in it; making the directory 0755 allows all three) and the
privileged scenario "Names below a directory the client may search but not
list are never delivered" (root's 0711 `tunnel/` and 0755 `tunnel/open/` in
the runner's project; root's write in `tunnel/open/` is withheld). **Cost:**
one `openat` and one `faccessat` (and a `close`) per directory below the
root, measured at about 1 µs per level on the dev host, against about
4.5 µs for the pipe round trip to the checker that every event already
makes. An event five levels below the root costs about 5 µs more, so no
cache is built (V181). A safe cache would key each directory's verdict on
its `(st_dev, st_ino)` and its `st_ctime`, which changes on `chmod`,
`chown` and ACL changes. But checking `st_ctime` needs an `fstatat` per
level anyway, and LSM policy changes would not show in it, so it would save
little and add a way to be wrong.

### 6.2 Confidentiality (A2, A3)

| # | Threat | Mitigation | Code | Test | Status |
|---|---|---|---|---|---|
| C1 | A user subscribes to a directory they cannot list, such as another user's home directory, to see its events. | The root is checked **as the client**: the checker canonicalizes it and requires `R_OK\|X_OK` (`faccessat` with `AT_EACCESS`) on the directory. Otherwise the subscription is refused. | `checker::check_root`, `checker::can_list`, `server::read_requests` | `privhelper.feature`: "The helper refuses a root the client cannot read" (V90); `checker::tests::roots_must_be_absolute_listable_directories` | mitigated |
| C2 | Inside a root the client can list, another user writes into a subdirectory the client cannot list. | Every event is checked as the client before it is sent: the path must be under the accepted root, the client must be able to list every directory from the root down to the one that holds it (F3), and that directory must be the one the event came from (F1). | `server::resolve`, `server::send_events` → `checker::Checker::may_see` → `checker::may_see`, `checker::open_listable_from` | `privhelper.feature`: "Paths the client cannot read are never delivered" (V90), "A path renamed after its event is checked against the directory the event came from" and "Names below a directory the client may search but not list are never delivered"; `checker::tests::a_path_is_seen_only_under_its_root_in_a_listable_directory`, `checker::tests::a_path_resolving_to_another_directory_is_refused`, `checker::tests::every_directory_from_the_root_down_must_be_listable` | mitigated |
| C3 | The filesystem mark covers the whole filesystem, so events outside every root reach the helper. | `dispatch` queues an event only for subscribers whose root is a prefix of its resolved path. The checker repeats that test. | `server::Registry::dispatch`, `checker::may_see` | `checker::tests::a_path_is_seen_only_under_its_root_in_a_listable_directory` (the `/etc/passwd` case) | mitigated |
| C4 | Hostile names (T3), for example a name holding `/` or `..` as reported, try to make a resolved path escape its directory. | A `DFID_NAME` name is joined only if it is exactly one normal path component, or `.`. A path from `/proc/self/fd` that ends in ` (deleted)` or is not absolute is dropped. A file *named* `x (deleted)` is therefore never reported, which affects only availability. | `server::resolve`, `handle::path_of` | `handle::tests::path_of_reads_the_proc_link` | mitigated |
| C5 | The checker is tricked into answering for the wrong user: wrong uid, groups kept, or root regained. | The checker gets its identity from `SO_PEERCRED` (set by the kernel at `connect`) and supplementary groups from `getgrouplist`. It calls `setgroups`, then `setresgid`, then `setresuid`, all three ids each. It verifies `getresuid`, checks that `setresuid(0,0,0)` now fails, and then becomes non-dumpable, all before it reads a request. If any step fails, the client is rejected. | `checker::Identity::of`, `checker::drop_privileges`, `checker::run` | `privhelper.feature` privileged scenarios, where the checker drops from root to the runner user (V90); `checker::tests::identities_round_trip_through_arguments`. Not tested directly: the regain-root refusal, and non-dumpability. | mitigated |
| C6 | The client's user attaches to its own checker (ptrace, `/proc/PID/mem`, its pipe descriptors) to make it approve paths. | `PR_SET_DUMPABLE` 0 after the drop. A change of effective uid also resets dumpability in the kernel, so there is no window before the `prctl`. ⚠ That kernel behaviour is reported, not verified in this repository. | `checker::drop_privileges` | none | mitigated (⚠) |
| C7 | A user in a container (another mount namespace) gets paths that mean something else there. | Paths are as the helper's namespace sees them. A client whose paths differ gets no events for them (its root, as canonicalized by the checker, will not match) and falls back to notify. | `server::Registry::dispatch` | none | mitigated |
| C8 | **Writer PIDs.** Under `hidepid`, a user may not see other users' PIDs, but an event in a directory the user can list carries its writer's PID. | Documented. In v1, PIDs stop at the watcher's helper client: `Batch` and `Change` have no PID field, and `watcher::helper_notice` drops the field (D40, V174). | `codetags-watch` `change::Batch`, `watcher::helper_notice` | `privhelper.feature`: "A write by another process is reported with that process's PID" (the client sees it; batches do not) | **residual (§7)** |
| C9 | The timing of other users' writes in directories the client can list. | Not prevented. Unprivileged inotify on a directory the user can read reveals the same. | none | none | accepted |

### 6.3 Privilege escalation (A1, A6)

| # | Threat | Mitigation | Code | Test | Status |
|---|---|---|---|---|---|
| P1 | The helper runs project code, or writes to a project. | It runs no code but its own image. It opens roots `O_RDONLY\|O_DIRECTORY\|O_NOFOLLOW` only to mark them, and decodes handles with `O_PATH`. Nothing in it writes under a root. | `server::Registry::add`, `handle::open` | code review | mitigated |
| P2 | A client steers root into opening a path the client chose. | The root that is opened is the one the checker canonicalized as the client. `O_NOFOLLOW` covers **only the final component**, though. If a client swaps an earlier component for a symlink between the check and the open, root opens a different directory. Effect: a filesystem the client cannot reach may be marked, and the root then receives no events (they are still path-filtered and checked as the client). So this costs work and confuses the client, but discloses nothing. | `server::Registry::add`, `checker::check_root` | none | partial (hardening: open the root as the client, or walk it with `openat2(RESOLVE_NO_SYMLINKS)`) |
| P3 | Memory corruption from hostile frames or kernel buffers. | All parsing is in safe Rust with bounds-checked reads. Frames are capped at 64 KiB, and a zero length is refused. The one `unsafe` call (`open_by_handle_at`) builds its `struct file_handle` in a 4-aligned buffer whose `handle_bytes` matches its length, after capping the handle at `MAX_HANDLE_SZ`. | `proto::read_frame`, `parse::parse`, `parse::fid`, `handle::open` | `proto::tests::oversized_and_empty_frames_are_refused`, `proto::tests::bad_bodies_are_protocol_errors`, `parse::tests::malformed_buffers_are_errors`, `handle::tests::an_oversized_handle_is_refused_before_the_call`, `handle::tests::a_bogus_handle_is_an_os_error_not_a_crash` | mitigated |
| P4 | The checker's re-exec runs a replaced binary (T4), or the environment (`LD_PRELOAD`) steers it. | It re-executes `/proc/self/exe`, the running image even if the path has been replaced, with `env_clear()`. Whoever can replace the installed binary can replace the helper at its next start anyway. So the install location must be root-owned all the way up (main.rs, "Running it"). | `checker::Checker::spawn`, `server::serve` (`exe`) | none | mitigated for the running image; the install path is the admin's responsibility |
| P5 | The socket setup is abused (E1). | The directory must be root's alone, and every step works on it opened; the mode comes from a umask, not a `chmod` (F2). Only an existing *socket* is replaced, and only if nothing answers on it. Any other file makes the helper refuse to start. | `socket::open_dir`, `socket::bind` | `socket::tests`; `tests/unprivileged.rs`: `a_socket_directory_others_can_write_is_refused`, `a_symlinked_socket_directory_is_refused`, `the_socket_is_created_with_mode_0666`, `a_stale_socket_is_replaced_but_a_live_one_is_not`, `a_non_socket_file_is_never_replaced`; `privhelper.feature`: "The helper refuses to put its socket in a directory another user can change" | mitigated |
| P6 | A user replaces the socket so that clients reach the user instead (T5, A4). | The socket's directory is one only root can write (F2), so only root can replace the socket in it. The watchdog exits when the socket's inode changes. | `socket::open_dir`, `server::watchdog` | `the_helper_exits_when_its_socket_is_removed` | mitigated, except as in I1 |

### 6.4 Integrity of the watcher's view (A4)

| # | Threat | Mitigation | Code | Test | Status |
|---|---|---|---|---|---|
| I1 | Someone impersonates the helper (T5) and suppresses or invents events. In helper mode notify does not run, so suppressed events are missed until a rescan. An impersonator could also send an `Accepted` root that does not match, so that later event paths are taken as project paths. | The default socket is in root-owned `/run`, which only root can create. The client does not yet check that the peer is root. | `privhelper::HelperSocket::resolve`, `privhelper::Client::connect` | none | partial (hardening: check the server's `SO_PEERCRED` uid is 0, and that the accepted root equals the canonical root, or is an alias the client already knows) |
| I2 | An event is a lie, or stale. | The watcher treats every helper event as a hint. It reads the path itself and classifies the change from what it finds (`Engine::examine`), so a kind or a path never adds content on the helper's word. | `watcher::Engine::hint`, `watcher::Engine::examine` | `watcher::tests::helper_events_feed_the_coalescer_and_a_lost_helper_falls_back` | mitigated |
| I3 | Events are lost: the fanotify queue overflows, or a client falls behind. | `FAN_Q_OVERFLOW` and a full per-client queue (`--queue`, 8 Ki entries by default) both send `O`, and the watcher rescans against its last batch. | `server::Registry::dispatch`, `server::Subscriber::deliver`, `server::send_events`, `watcher::helper_notice` | `server::tests::a_full_queue_drops_events_and_sends_an_overflow`; `watcher::tests::helper_events_feed_the_coalescer_and_a_lost_helper_falls_back` (the overflow part) | mitigated |
| I4 | A malformed kernel buffer drops a whole read's events, and no overflow is sent. | Logged only. | `server::read_events` | none | partial (hardening: send `O` to every client on a parse error) |
| I5 | The helper dies or hangs up. | The watcher switches to notify and rescans (`RescanCause::HelperLost`). | `watcher::helper_notice` | `watcher::tests::helper_events_feed_the_coalescer_and_a_lost_helper_falls_back`; `privhelper.feature`: "With no helper running, the watcher falls back to notify" | mitigated |

### 6.5 Denial of service (A5)

| # | Threat | Mitigation | Code | Test | Status |
|---|---|---|---|---|---|
| D1 | Many connections: each costs a helper thread (two after the handshake) and a checker process. Root's `fork` is not held to `RLIMIT_NPROC`. | Connections are capped in total (`--max-connections`, 128) and per uid (`--max-connections-per-uid`, 16), counted from `accept` until the connection's thread ends, so a connection that never completes its handshake (5 s timeout) counts too. A connection over a cap is refused on the accept thread itself, with one `Rejected` frame and no thread of its own (the client reads it as its handshake's reply, V177). Checker processes are capped separately (`--max-checkers`, 64); a handshake over that cap is `Rejected`. Each client falls back to notify. | `server::serve`, `server::refuse`, `limits::Admission`, `limits::Gate`, `server::serve_client` | `limits::tests::connections_are_capped_in_total`, `connections_are_capped_per_uid`, `checkers_are_capped`; `tests/unprivileged.rs`: `connections_over_the_total_limit_are_rejected`, `connections_over_the_per_uid_limit_are_rejected`, `checkers_over_the_limit_are_rejected`; `client::tests::a_refusal_before_the_handshake_is_read_is_still_a_rejection` | mitigated |
| D2 | Many subscriptions: each distinct root keeps a descriptor open, and every event on a marked filesystem is resolved once per root (one `open_by_handle_at` each), on the single fanotify thread, under the registry lock. | Roots are shared across clients by path. Each connection may hold at most `--max-subscriptions` (32) distinct roots; one more is `Refused` with the reason. So one user holds at most 16 × 32 roots, and all users together 128 × 32. | `limits::Subscriptions`, `server::read_requests`, `server::Registry::add` | `limits::tests::subscriptions_are_capped_per_connection`; `privhelper.feature`: "The helper refuses a subscription past its per-connection limit" | mitigated |
| D3 | A slow client stalls the helper, or makes it hold unbounded memory. | Delivery to clients never blocks the fanotify thread (`try_send`). Each client's queue holds at most `--queue` (8 Ki) events; a full queue drops the event and marks the client, which is then sent an overflow and rescans. A write that blocks for 30 s drops the client. | `server::Subscriber::deliver`, `server::send_events` (`WRITE_TIMEOUT`), `limits::Limits::queue` | `server::tests::a_full_queue_drops_events_and_sends_an_overflow` | mitigated |
| D4 | A user stops or kills their own checker. | That stalls or drops only that user's connection. | `server::send_events` | none | mitigated |
| D5 | A busy filesystem: a filesystem mark sees every write on it, so the cost scales with all the filesystem's activity, not just the projects'. | By design. The kernel queue overflows (I3) rather than blocking writers (`FAN_CLASS_NOTIF`). | `fanotify::Group::init` | none | accepted |
| D6 | Slow name-service lookups (`getpwuid`, `getgrouplist` through NSS, for example LDAP) on each connection. | None. They run on the client's thread, not the fanotify thread. | `checker::Identity::of` | none | residual |

## 7. Residual risks

Stated plainly, with F1 and F2 fixed and the limits built:

1. **Writer PIDs under `hidepid` (C8).** A user who can list a directory
   learns the PID of any process that writes in it, including other users'
   processes. That defeats `hidepid` for those processes. v1 keeps PIDs out of
   watcher batches (D40). They are still on the wire to any client. Removing
   them from the protocol, or sending them only when the writer's uid is the
   client's, would close this.
2. **Denial of the helper's service (D1, D2, D6).** The limits bound what
   the helper spends: at the defaults, 128 connections, 64 checker processes,
   32 roots per connection and 8 Ki queued events per connection. One user
   can take 16 connections; eight users together can fill the total, after
   which further connections are refused and those watchers use notify.
   That denies the accelerator, not the host, and costs nobody correctness.
   One user's subscriptions (at most 16 × 32 roots) can still slow event
   resolution on the shared fanotify thread, which then overflows and
   everyone rescans. Under systemd, `TasksMax=` and `LimitNOFILE=` remain a
   second bound on the host (§8).
3. **Whole-filesystem cost (D5).** Marking a filesystem makes the kernel
   generate events for all its activity. On a busy shared filesystem that is
   steady overhead, chosen by whoever subscribes first.
4. **Kernel-version assumptions.** These were verified on Linux 6.12 (the dev
   host) and 6.17 (the CI runner) only (V86, V90):
   - `FAN_REPORT_FID` needs 5.1, and `FAN_REPORT_DFID_NAME` 5.9. Before 5.9,
     events name only the object or its directory, so per-event checks run on
     less precise paths.
   - The checker's guarantees assume `setresuid` semantics, that a change of
     effective uid resets dumpability (C6, ⚠), and that `faccessat(AT_EACCESS)`
     applies ACLs and LSMs as for the user.
   - Resolution assumes `open_by_handle_at` and `/proc/self/fd` give the path
     *through the root's mount*. For a root that is a bind mount of a
     subdirectory, how kernels resolve objects outside that subtree has not
     been checked (⚠). The path-based checks still apply to whatever path
     comes back.
   - `same_fs` assumes glibc's 64-bit `f_fsid` packing (or musl's 32-bit one).
5. **Paths are names, not objects.** Since F1's fix, the per-event decision
   is bound to the identity of the directory or object the event came from,
   and since F3's fix every directory from the root down is checked on a
   descriptor reached from its parent. What remains: identity is `(st_dev,
   st_ino)`, so a directory deleted and its inode number reused by a new
   directory between resolution and check would pass (the window is bounded
   by the queue, and the deleter must be able to remove the original).
   Ancestors *above* the root need only search permission, as for the root
   at subscription: the client named the root itself.
6. **No impersonation check on the client (I1).** It is safe at the default
   socket path. The helper refuses to listen in a directory other users can
   write (F2), but a client pointed there by `CODETAGS_PRIVHELPER_SOCKET`
   would still trust whoever listens.
7. **Mount namespaces (C7).** Containers get nothing from the helper. That is
   safe, but it is not a feature.

## 8. Operational requirements, if it is ever installed

These go into the install docs only after sign-off:

- Install the binary root-owned, in a directory that is root-owned all the
  way up.
- Run the helper only with a socket directory that root alone can write.
  The default, `/run/codetags/` via systemd's `RuntimeDirectory=`, meets this.
  The helper now refuses any other (F2).
- Under systemd, set `NoNewPrivileges=yes`, `ProtectSystem=strict`, a
  `TasksMax=` and a `LimitNOFILE=` above what the helper's limits need
  (roughly two threads per connection and one checker process each, and a
  descriptor per root). Do not set `CODETAGS_PRIVHELPER_SOCKET` system-wide
  to a path that users can write.
- Lower the limits (`--max-connections`, `--max-connections-per-uid`,
  `--max-checkers`, `--max-subscriptions`, `--queue`) on hosts where the
  defaults are too generous.
- Stopping the helper is always safe: watchers fall back to notify and rescan.

## 9. Sign-off checklist

The human signs off when every box is checked:

- [x] F1 (the access-check race) is fixed (H3: `server::resolve` records a
      binding, `checker::may_see` enforces it), and the privileged scenario
      "A path renamed after its event is checked against the directory the
      event came from" sets up the race and passes (V178, V179).
- [x] F2 (socket setup in a directory others can write) is fixed (H1): the
      helper refuses such a location (`socket::open_dir`), binds through the
      opened directory with a umask and no `chmod` (`socket::bind`). Unit
      tests, the binary's tests and a privileged scenario cover it, and the
      BDD harness uses a root-owned 0755 directory (V175, V176, V179).
- [x] F3 (names through directories the client may search but not list) is
      fixed: every directory from the root down must be listable, checked
      on a descriptor walk (`checker::open_listable_from`), with a unit test
      and a privileged scenario (V181, V182).
- [x] The helper's unit tests run as root in the `privileged-linux` job, so
      their root-only branches run (V180).
- [ ] §6 re-read against the code at the sign-off commit. Every mitigation
      still names the function that implements it.
- [ ] Residual risk 1 (writer PIDs under `hidepid`) accepted, or the protocol
      changed to drop PIDs or limit them to the client's own uid.
- [ ] Residual risk 2 (denial of the helper's service) accepted: the limits
      are built (H2; D1-D3) with defaults of 128 connections, 16 per uid, 64
      checkers, 32 subscriptions per connection and 8 Ki queued events. The
      defaults, and the systemd limits in §8, are the human's call.
- [ ] Hardening items decided, each either done or accepted: P2 (open the root
      without following symlinks in any component), I1 (client checks that the
      peer is root and validates the accepted root), I4 (overflow on a parse
      error).
- [ ] The ⚠ items in C6 and residual risk 4 verified and recorded in
      `docs/verification.md`, or accepted as unverified.
- [ ] The `privileged (ubuntu-24.04)` CI job is green at the sign-off commit.
- [ ] The Windows helper (P4.4 part 2), when built, gets its own section
      here and its own sign-off. This sign-off covers Linux only.
- [ ] After sign-off: the install guidance in `codetags-privhelper`'s crate
      docs ("Running it") may be restored, and `codetags doctor` may mention
      the helper.

Signed off by: ____________________  Date: __________  Commit: __________
