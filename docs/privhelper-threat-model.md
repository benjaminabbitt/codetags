# Threat model: `codetags-privhelper`

**Status: draft for the human's sign-off (PLAN.md D40). Not signed off.**
Until this document is signed off, no doc, README or `codetags doctor`
output suggests installing the helper. **Two findings below block sign-off**
(§6.1). This document names them only by class; the details went to the
coordinator, and are not in this repository.

- **Scope:** the Linux fanotify helper (`crates/codetags-privhelper`) and its
  client in the watcher (`crates/codetags-watch/src/privhelper/`), at the
  commit that adds this file. The Windows USN-journal helper is not built
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
| E1 | **The Unix socket**, mode 0666, `/run/codetags/privhelper.sock` by default (or `--socket`, or `CODETAGS_PRIVHELPER_SOCKET`). Setting it up: create the parent if missing, replace a stale socket, bind, then chmod. | `server::bind`, `server::watchdog`, `main::run` |
| E2 | **The handshake**: an `H` frame (version and magic). A frame is at most 64 KiB, framed by a `u32` length. | `proto::read_frame`, `proto::Request::decode`, `server::serve_client` |
| E3 | **Subscriptions**: `S` frames carrying a root path (raw bytes, no NUL). | `server::read_requests`, `checker::Checker::check_root`, `server::Registry::add` |
| E4 | **fanotify events**: raw buffers from the kernel, parsed in safe code. | `server::read_events`, `parse::parse`, `parse::fid`, `server::Registry::dispatch` |
| E5 | **Handle decoding**: `open_by_handle_at` on the root's descriptor, then `readlink /proc/self/fd/N`. | `handle::open` (the crate's only `unsafe`), `handle::path_of`, `server::resolve` |
| E6 | **The checker's re-exec**: `/proc/self/exe --checker UID GID GROUPS` with a cleared environment. It drops to the client's user, and answers `R` and `P` frames on its stdin and stdout. | `checker::Checker::spawn`, `checker::run`, `checker::drop_privileges`, `checker::answer` |
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

### 6.1 Findings that block sign-off

| # | Class | Status |
|---|---|---|
| F1 | **An access-check race (time-of-check to time-of-use) in per-event filtering.** Under conditions a local user can arrange, the user can learn the names of entries in a directory they cannot list (A2). This breaks the documented guarantee that the helper "never shows one user another user's file names". Reported to the coordinator with a reproduction. | **open** |
| F2 | **The socket setup trusts the directory it is in.** If the socket path is in a directory another user can write, that user can influence what root does while the helper starts (A1). The default path (`/run/codetags/`, root-owned) is not affected, and neither is a systemd `RuntimeDirectory=`. But the helper does not refuse an unsafe location, and the BDD harness uses one (a scratch directory owned by the runner user, which is harmless in CI). Reported to the coordinator. | **open** |

Sign-off needs both fixed, each with a regression test in the
`privileged-linux` job. After that, they can be described here in full.

### 6.2 Confidentiality (A2, A3)

| # | Threat | Mitigation | Code | Test | Status |
|---|---|---|---|---|---|
| C1 | A user subscribes to a directory they cannot list, such as another user's home directory, to see its events. | The root is checked **as the client**: the checker canonicalizes it and requires `R_OK\|X_OK` (`faccessat` with `AT_EACCESS`) on the directory. Otherwise the subscription is refused. | `checker::check_root`, `checker::can_list`, `server::read_requests` | `privhelper.feature`: "The helper refuses a root the client cannot read" (V90); `checker::tests::roots_must_be_absolute_listable_directories` | mitigated |
| C2 | Inside a root the client can list, another user writes into a subdirectory the client cannot list. | Every event is checked as the client before it is sent: the path must be under the accepted root, and the client must be able to list the directory that holds it. | `server::send_events` → `checker::Checker::may_see` → `checker::may_see` | `privhelper.feature`: "Paths the client cannot read are never delivered" (V90); `checker::tests::a_path_is_seen_only_under_its_root_in_a_listable_directory` | **partial: see F1** |
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
| P5 | The socket setup is abused (E1). | Only an existing *socket* is replaced, and only if nothing answers on it. Any other file makes the helper refuse to start. **But see F2.** | `server::bind` | `server::tests::bind_replaces_a_stale_socket_but_never_a_file` | **open: F2** |
| P6 | A user replaces the socket so that clients reach the user instead (T5, A4). | Under `/run/codetags/` only root can do this. The watchdog exits when the socket's inode changes. | `server::watchdog` | none | mitigated at the default path |

### 6.4 Integrity of the watcher's view (A4)

| # | Threat | Mitigation | Code | Test | Status |
|---|---|---|---|---|---|
| I1 | Someone impersonates the helper (T5) and suppresses or invents events. In helper mode notify does not run, so suppressed events are missed until a rescan. An impersonator could also send an `Accepted` root that does not match, so that later event paths are taken as project paths. | The default socket is in root-owned `/run`, which only root can create. The client does not yet check that the peer is root. | `privhelper::HelperSocket::resolve`, `privhelper::Client::connect` | none | partial (hardening: check the server's `SO_PEERCRED` uid is 0, and that the accepted root equals the canonical root, or is an alias the client already knows) |
| I2 | An event is a lie, or stale. | The watcher treats every helper event as a hint. It reads the path itself and classifies the change from what it finds (`Engine::examine`), so a kind or a path never adds content on the helper's word. | `watcher::Engine::hint`, `watcher::Engine::examine` | `watcher::tests::helper_events_feed_the_coalescer_and_a_lost_helper_falls_back` | mitigated |
| I3 | Events are lost: the fanotify queue overflows, or a client falls behind. | `FAN_Q_OVERFLOW` and a full per-client queue (16 Ki entries) both send `O`, and the watcher rescans against its last batch. | `server::Registry::dispatch`, `server::Subscriber::deliver`, `server::send_events`, `watcher::helper_notice` | `watcher::tests::helper_events_feed_the_coalescer_and_a_lost_helper_falls_back` (the overflow part) | mitigated |
| I4 | A malformed kernel buffer drops a whole read's events, and no overflow is sent. | Logged only. | `server::read_events` | none | partial (hardening: send `O` to every client on a parse error) |
| I5 | The helper dies or hangs up. | The watcher switches to notify and rescans (`RescanCause::HelperLost`). | `watcher::helper_notice` | `watcher::tests::helper_events_feed_the_coalescer_and_a_lost_helper_falls_back`; `privhelper.feature`: "With no helper running, the watcher falls back to notify" | mitigated |

### 6.5 Denial of service (A5)

| # | Threat | Mitigation | Code | Test | Status |
|---|---|---|---|---|---|
| D1 | Many connections: each completed handshake costs a helper thread pair and a checker process. Connections that never complete the handshake time out after 5 s. | A handshake timeout only. There is no limit per uid or in total. Root's `fork` is not held to `RLIMIT_NPROC`. | `server::serve`, `server::serve_client` | none | **residual (§7)** |
| D2 | Many subscriptions: each distinct root keeps a descriptor open, and every event on a marked filesystem is resolved once per root (one `open_by_handle_at` each), on the single fanotify thread, under the registry lock. | Roots are shared across clients by path. There is no limit on subscriptions per client. | `server::Registry::add`, `server::Registry::dispatch` | none | **residual (§7)** |
| D3 | A slow client stalls the helper. | Delivery to clients never blocks the fanotify thread (`try_send`). A full queue sends overflow. A write that blocks for 30 s drops the client. | `server::Subscriber::deliver`, `server::send_events` (`WRITE_TIMEOUT`) | none | mitigated |
| D4 | A user stops or kills their own checker. | That stalls or drops only that user's connection. | `server::send_events` | none | mitigated |
| D5 | A busy filesystem: a filesystem mark sees every write on it, so the cost scales with all the filesystem's activity, not just the projects'. | By design. The kernel queue overflows (I3) rather than blocking writers (`FAN_CLASS_NOTIF`). | `fanotify::Group::init` | none | accepted |
| D6 | Slow name-service lookups (`getpwuid`, `getgrouplist` through NSS, for example LDAP) on each connection. | None. They run on the client's thread, not the fanotify thread. | `checker::Identity::of` | none | residual |

## 7. Residual risks

Stated plainly, after F1 and F2 are fixed:

1. **Writer PIDs under `hidepid` (C8).** A user who can list a directory
   learns the PID of any process that writes in it, including other users'
   processes. That defeats `hidepid` for those processes. v1 keeps PIDs out of
   watcher batches (D40). They are still on the wire to any client. Removing
   them from the protocol, or sending them only when the writer's uid is the
   client's, would close this.
2. **Denial of service by any local user (D1, D2, D6).** Unbounded
   connections, checker processes, subscriptions and open descriptors. One
   user can exhaust the helper's descriptors (then it stops accepting, and
   watchers fall back to notify) or slow event resolution for everyone (then
   overflows and rescans follow). Under systemd, `TasksMax=` and
   `LimitNOFILE=` bound the damage to the host. In-process limits per uid
   are not built.
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
5. **Paths are names, not objects.** Every check after resolution works on a
   path string. F1 is the instance found so far. Any fix should move checks
   to descriptors where it can.
6. **No impersonation check on the client (I1).** It is safe at the default
   socket path, but not if an admin points `CODETAGS_PRIVHELPER_SOCKET` at a
   directory other users can write.
7. **Mount namespaces (C7).** Containers get nothing from the helper. That is
   safe, but it is not a feature.

## 8. Operational requirements, if it is ever installed

These go into the install docs only after sign-off:

- Install the binary root-owned, in a directory that is root-owned all the
  way up.
- Run the helper only with a socket directory that root alone can write.
  The default, `/run/codetags/` via systemd's `RuntimeDirectory=`, meets this.
  No other location is supported until F2 is fixed.
- Under systemd, set `NoNewPrivileges=yes`, `ProtectSystem=strict`, a
  `TasksMax=` and a `LimitNOFILE=`. Do not set `CODETAGS_PRIVHELPER_SOCKET`
  system-wide to a path that users can write.
- Stopping the helper is always safe: watchers fall back to notify and rescan.

## 9. Sign-off checklist

The human signs off when every box is checked:

- [ ] F1 (the access-check race) is fixed, and a privileged regression
      scenario reproduces the race and passes.
- [ ] F2 (socket setup in a directory others can write) is fixed: the helper
      refuses such a location, or is immune to it. A unit or privileged test
      covers it, and the BDD harness uses a safe location.
- [ ] §6 re-read against the code at the sign-off commit. Every mitigation
      still names the function that implements it.
- [ ] Residual risk 1 (writer PIDs under `hidepid`) accepted, or the protocol
      changed to drop PIDs or limit them to the client's own uid.
- [ ] Residual risk 2 (local denial of service) accepted with the systemd
      limits in §8, or per-uid limits built.
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
