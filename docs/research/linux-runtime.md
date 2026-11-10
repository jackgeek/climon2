# Linux runtime and lifecycle guarantees

Research for [#126](https://github.com/jackgeek/climon2/issues/126), under
[map #124](https://github.com/jackgeek/climon2/issues/124); informs
[#129](https://github.com/jackgeek/climon2/issues/129). Checked **2026-11-10** against
`origin/main` `153dd5c`. Facts and credible choices only: no policy is chosen, no exception to the
existing contract is approved, nothing is implemented.

**Tags.** **V** = verified in a cited primary source (man page, POSIX, kernel/systemd source at a pinned
tag, repo file at `153dd5c`). **O** = observed read-only on this host (Arch-derived, kernel `7.2.15`,
systemd `262.1`). **I** = inference from V facts. **U** = not established. Nothing was run on Ubuntu
24.04; no signal was sent, no workload started, no system setting changed. Kernel citations use tag
`v6.8` (Ubuntu 24.04 GA kernel line); no cross-kernel diff was done (**U**).

## Material blockers for the Linux port (read first)

1. **`daemon_start_time` cannot be derived stably from procfs.** `/proc/<pid>/stat` field 22 is ticks
   since boot; the only wall-clock bridge is `/proc/stat` `btime`, computed from the *current*
   wall-clock offset. A clock step changes the result for a live process. `list` deletes metadata when
   connect fails with `ENOENT`/`ECONNREFUSED` and `is_same_process` says false, so a step plus an
   unreachable socket deletes a live session's metadata (§1). The exploratory port uses exactly this
   derivation.
2. **Zombies differ from macOS.** Linux zombies keep PGID/SID until reaped, so `getsid` works on them and
   `SIGKILL` is a no-op; the kill loop would exhaust `KILL_ROUNDS` and report "survived SIGKILL" unless
   zombies are excluded (§3).
3. **Master-close hang-up signals only the session leader** (`tty_vhangup` → `__tty_hangup(tty, 0)`);
   the foreground group is not signalled (§4). Other jobs rely on the leader; only the grace-then-SIGKILL
   covers them.
4. **ADR 0003's `/tmp` premise is wrong for Ubuntu 24.04**: it ships `D /tmp 1777 root root 30d`, not
   10 d, and systemd's cleaner skips sockets listed in `/proc/net/unix` and anything it cannot `flock`
   (§7). The exploratory timestamp-refresher is not shown to be necessary.
5. **`/proc` absent looks like "process gone"** (`ENOENT` mapping) (§2).
6. **Test hazards on a shared host:** tests signal raw recorded PIDs after reaping; group-wide pidfd
   signalling needs Linux ≥ 6.9, absent on Ubuntu 24.04 GA 6.8; unprivileged user namespaces are
   AppArmor-restricted on 24.04 (§8).

## Existing contract and exploratory evidence

Read: `AGENTS.md`, `docs/agents/domain.md`, `GLOSSARY.md`, ADRs
[0003](../adr/0003-platform-and-shell-seams.md), [0005](../adr/0005-frozen-session-metadata-core.md),
[0007](../adr/0007-daemon-owned-virtual-terminal.md).

- ADR 0005 freezes `daemon_start_time` as "u64, microseconds since the Unix epoch, from the platform's
  process-start-time seam"; the stale check compares it with the live process; the format "may only grow".
- ADR 0003 lists ten seams and a Linux note: keep `/tmp/climon2-<uid>-<hash8>`; "systemd-tmpfiles sweeps
  `/tmp` entries older than 10 days, sockets included, so the Linux port must keep live sockets fresh".
- Hang-up grace is five seconds (`crates/climon2-core/src/protocol.rs:12`).
- Every seam body is macOS-only with `compile_error!` elsewhere
  (`crates/climon2-platform/src/process.rs:2-6`, `runtime.rs:2-6`, `peer.rs:2-6`, `pty.rs:2-6`,
  `daemon.rs:2-6`); platform tests are `#![cfg(target_os = "macos")]`
  (`crates/climon2-platform/tests/runtime.rs:1`).
- **Exploratory, not contract:** `/home/jack/dev/climon2.worktrees/linux`, `feature/linux-support`
  `3ada49f` plus uncommitted edits, read-only: `process/linux.rs` (`btime` start time, `/proc` scan),
  `SocketFreshness`, `SO_PEERCRED`.

## 1. Process identity as Unix-epoch microseconds

**Current use.** `start_time` returns Unix µs (`process.rs:38-43`; macOS
`pbi_start_tvsec*1_000_000+pbi_start_tvusec`, `:66-71`); the daemon records it once
(`crates/climon2-daemon/src/startup.rs:173-176`); `is_same_process` is exact equality (`process.rs:81-83`);
`list` consults it only after connect error `ENOENT`/`ECONNREFUSED`, and only `Ok(false)` deletes metadata
and socket (`crates/climon2-cli/src/list.rs:244-267`). Errors are deliberately not evidence of death
(`process.rs:80`).

**Kernel facts.**
- `starttime` is "the time the process started after system boot", in clock ticks (V,
  [proc_pid_stat(5)](https://man7.org/linux/man-pages/man5/proc_pid_stat.5.html)); printed as
  `nsec_to_clock_t(timens_add_boottime_ns(task->start_boottime))` (V,
  [`fs/proc/array.c` L586-587](https://github.com/torvalds/linux/blob/v6.8/fs/proc/array.c#L586-L587)),
  with `start_boottime = ktime_get_boottime_ns()` at fork (V,
  [`kernel/fork.c` L2615](https://github.com/torvalds/linux/blob/v6.8/kernel/fork.c#L2615)). Resolution is
  one tick: `USER_HZ` 100 = 10 ms (O: `getconf CLK_TCK` = 100).
- `btime` is "boot time, in seconds since the Epoch" (V,
  [proc_stat(5)](https://man7.org/linux/man-pages/man5/proc_stat.5.html)), printed from
  `getboottime64()` adjusted by `timens_sub_boottime` (V,
  [`fs/proc/stat.c` L95-97,180](https://github.com/torvalds/linux/blob/v6.8/fs/proc/stat.c#L95-L97)), and
  `getboottime64` = `offs_real - offs_boot` (V,
  [`timekeeping.c` L2246-2252](https://github.com/torvalds/linux/blob/v6.8/kernel/time/timekeeping.c#L2246-L2252)).
- **I:** `offs_real` follows `CLOCK_REALTIME`, so a clock step moves `btime` while the process's ticks do
  not; `btime + ticks/HZ` then changes for a live process. A step was not demonstrated (would alter the
  system clock). Whether slewing leaves `offs_real` unchanged is **U**.
- Time namespaces shift both values (V, same sources).
- Exploratory derivation: `process/linux.rs:60-86` (`btime*1e6 + ticks*1e6/CLK_TCK`), no step handling.

**Choices (none made).** A) accept the derivation and document the step hazard (an exception to ADR 0005's
intent). B) compare within a bounded tolerance (policy; PID wrap needed for rapid reuse: `pid_max` default
32768, up to 2²², V [proc_sys_kernel(5)](https://man7.org/linux/man-pages/man5/proc_sys_kernel.5.html);
O: 4194304). C) add optional metadata (boot id `/proc/sys/kernel/random/boot_id` plus raw ticks), allowed by
"may only grow" but needs `protocol-docs`. D) require an extra liveness proof (hello probe, held lock,
pidfd) before deleting metadata; changes the `list.rs:253` rule.

## 2. procfs parsing and access restrictions

- Fields: `comm` in parentheses (≤15 chars, may contain spaces/parentheses, I), `state` (`Z` zombie, `X`
  dead), 5 `pgrp`, 6 `session`, 7 `tty_nr`, 8 `tpgid`, 22 `starttime` (V, proc_pid_stat(5)). Parse after the
  last `) ` (exploratory `process/linux.rs:20-26` does).
- `tty_nr`: minor in bits 31-20 and 7-0, major in 15-8 (V, same page; `new_encode_dev` at
  `fs/proc/array.c` L520). `stat(2)` `st_rdev` uses the glibc encoding, which differs for minors > 255;
  normalise before comparing (I). `Terminal::from_path` stores `st_rdev` as `u32` (`process.rs:195-210`).
- `[PT]` fields read 0 if `ptrace_may_access(PTRACE_MODE_READ_FSCREDS)` fails (V, proc_pid_stat(5);
  `array.c` L490). `state`, `pgrp`, `session`, `tty_nr`, `starttime` are not `[PT]`-marked in the fields
  read, but I did not audit every kernel (U).
- `sid`/`pgrp` are translated into the reader's PID namespace (`task_session_nr_ns`, `task_pgrp_nr_ns`,
  `array.c` L533-535, V).
- `hidepid=1` hides others' `/proc/<pid>` contents, `hidepid=2` hides the directories (V,
  [proc(5)](https://man7.org/linux/man-pages/man5/proc.5.html)). Same-UID climon2 processes are unaffected
  (I). O: this host's `/proc` has no `hidepid`.
- Without `/proc` every read is `ENOENT`, which `missing_process` (`process.rs:73-78`) maps to "dead";
  needs a separate `/proc` usability check (I).
- No kernel-filtered tty/group listing like macOS `proc_listpids`; membership is a `/proc` scan racing
  with exits (I). Cost is U.

## 3. PID/session/group membership, reuse, zombies, reaping

- POSIX: a PID is not reused until process lifetime ends (PID returned to the system); if a group with that
  ID exists, not until the group lifetime ends (last member leaves) (V,
  [POSIX.1-2024 XBD §3.285, §3.283, §4.17](https://pubs.opengroup.org/onlinepubs/9799919799/basedefs/V1_chap04.html)).
  **I:** a recorded group ID is trustworthy only while some verified member remains.
- A zombie "might" be reported as existing by `kill` (V, [kill(2)](https://man7.org/linux/man-pages/man2/kill.2.html)).
  `__unhash_process` detaches `PIDTYPE_PGID`/`SID` only at release after reaping (V,
  [`kernel/exit.c`](https://github.com/torvalds/linux/blob/v6.8/kernel/exit.c)). **I:** `getsid`/`getpgid`
  succeed on Linux zombies, contradicting the macOS comment at `process.rs:299`; counting them makes
  `kill_rounds` (100×10 ms) fail (`process.rs:262-270`). Exploratory code excludes `Z`
  (`process/linux.rs:64,106`); `X` is U.
- `wait`/`waitpid` reap (V, [wait(2)](https://man7.org/linux/man-pages/man2/wait.2.html)); non-children
  give `ECHILD` (test `crates/climon2-platform/tests/process.rs:238-241`). Orphans reparent to PID 1 or the
  nearest subreaper (V,
  [PR_SET_CHILD_SUBREAPER(2const)](https://man7.org/linux/man-pages/man2/PR_SET_CHILD_SUBREAPER.2const.html));
  an init that never reaps leaves zombies (I).
- `pidfd_open` keeps a process identity, including a zombie child before reaping, unless `SIGCHLD` is
  ignored, `SA_NOCLDWAIT` set, or reaped elsewhere (V,
  [pidfd_open(2)](https://man7.org/linux/man-pages/man2/pidfd_open.2.html)); `CLONE_PIDFD` since 5.2 (V,
  [clone(2)](https://man7.org/linux/man-pages/man2/clone.2.html)); `PIDFD_SIGNAL_PROCESS_GROUP` since
  **6.9** (V, [pidfd_send_signal(2)](https://man7.org/linux/man-pages/man2/pidfd_send_signal.2.html)),
  absent on Ubuntu 24.04 GA 6.8 (I).
- Current raw signalling: `signal_group` rejects ≤1 (`process.rs:172-178`), `kill_process` takes
  `NonZeroU32` (`:311-314`), members must be in the recorded session and not the caller (`:289-302`);
  CLI `exec` signals its child's group by PID (`crates/climon2-cli/src/exec.rs:290-301`).

## 4. Controlling-PTY hang-up and escalation

Current: observe, close master, sleep `HANGUP_GRACE`, `try_wait`, SIGKILL remaining members
(`process.rs:331-357`). Tests: `issue19_hangup_escalates_group_after_real_five_seconds` (≥5 s, <8 s, 137),
`issue19_closing_real_controlling_pty_delivers_hangup` (129)
(`crates/climon2-platform/tests/process.rs:244-257,391-398`);
`every_controlling_leave_hangs_up_then_kills_after_real_grace`
(`crates/climon2-daemon/tests/lifecycle.rs:751`).

Kernel (v6.8):
- Master close: `pty_close` sets `TTY_OTHER_CLOSED` and calls `tty_vhangup(tty->link)` (V,
  [`pty.c`](https://github.com/torvalds/linux/blob/v6.8/drivers/tty/pty.c)); `tty_vhangup` =
  `__tty_hangup(tty, 0)` (V, [`tty_io.c` L697-701](https://github.com/torvalds/linux/blob/v6.8/drivers/tty/tty_io.c#L697-L701)).
- `tty_signal_session_leader` clears `signal->tty` for session members on that tty, sends `SIGHUP` +
  `SIGCONT` to the session leader task, and `SIGHUP`s the foreground group only if `exit_session` ≠ 0 (V,
  [`tty_jobctrl.c` L185-232](https://github.com/torvalds/linux/blob/v6.8/drivers/tty/tty_jobctrl.c#L185-L232);
  the comment says "and its process group", the code does not). **Fact:** at master close only the leader is
  signalled; other jobs get nothing directly.
- Leader exit: `do_exit` → `disassociate_ctty(1)` (`kernel/exit.c` L869); for PTYs it `SIGHUP`s the
  foreground group, then `session_clear_tty` removes the controlling terminal from all members (V,
  [`tty_jobctrl.c` L265-](https://github.com/torvalds/linux/blob/v6.8/drivers/tty/tty_jobctrl.c#L265)).
  A tty-based scan finds nothing afterwards; job groups must be recorded while the leader lives, as on
  macOS (`process.rs:212-214,230-238`).
- Child setup: portable-pty 0.9.0 resets signal dispositions, `setsid`, `TIOCSCTTY`, closes fds in
  `pre_exec` (V, `portable-pty-0.9.0/src/unix.rs:234-290`; pinned in
  `crates/climon2-daemon/Cargo.toml:15`); helper exec at `crates/climon2-daemon/src/pty.rs:218-241`.
- Processes that `setsid` away are spared today (`tests/process.rs:273-295`; lifecycle `daemonize`
  fixture `lifecycle.rs:2145-2166`); Linux does not change that.
- The scan-to-`kill(pid)` window exists on Linux as on macOS (I); pidfd closes it only for held
  descriptors. SIGKILL promptness for `D`-state members is U.

## 5. PTY EOF and error behaviour

- Master read with no queued input returns `-EIO` when `TTY_OTHER_CLOSED` (queued data first); after a
  hang-up it returns 0 (V,
  [`n_tty.c` L2243-2248](https://github.com/torvalds/linux/blob/v6.8/drivers/tty/n_tty.c#L2243-L2248));
  write with the slave gone returns `-EIO` (`n_tty.c` L2370-2375, V).
- The daemon classifier treats 0 bytes or `EIO` as end, `WouldBlock` as idle
  (`crates/climon2-daemon/src/session/command.rs:7-36`, test `:131`): consistent with Linux (I).
- The Darwin allocation lock exists for `EREDRIVEOPEN` (`pty.rs:1-4`); no Linux evidence either way (U).
- Slave name via `ttyname_r` (`portable-pty unix.rs:108`) → `/dev/pts/N`.

## 6. Peer credentials and AF_UNIX paths

- `SO_PEERCRED` returns `ucred` captured at `connect`/`listen`/`socketpair` (V,
  [unix(7)](https://man7.org/linux/man-pages/man7/unix.7.html)); macOS uses `getpeereid`
  (`peer.rs:135-142`). Exploratory Linux body `peer.rs:34-68`.
- `sun_path` is 108 bytes on Linux (V, unix(7)); macOS 104. Current bound 103 (`runtime.rs:51,91`; macOS
  test `tests/runtime.rs:60`). The real path is at most 74 bytes (`/tmp/climon2-<10-digit uid>-<8>/<36>.sock`,
  computed from `layout.rs:158-163`). Keeping 103 or a Linux limit is a choice.
- Linux honours directory permissions for socket creation, and connecting needs write permission on the
  socket (V, unix(7)). `umask(0o077)` bind and directory owner/mode checks (`runtime.rs:69-102`) are
  portable; only the `/tmp` root is platform-specific (`:40`). The macOS `/private` ownership test
  (`tests/runtime.rs:63-92`) needs a Linux fixture (I).
- Abstract sockets avoid cleanup but conflict with `socket_path` "absolute path" (ADR 0005); noted only.

## 7. Keeping live `/tmp` sockets and directories safe from cleanup

- Upstream v255 `tmp.conf`: `q /tmp 1777 root root 10d` (V,
  [tmp.conf](https://github.com/systemd/systemd/blob/v255/tmpfiles.d/tmp.conf)).
- **Ubuntu 24.04 (`255.4-1ubuntu8.11`): `D /tmp 1777 root root 30d`** "in addition to cleaning at boot" (V,
  [Launchpad patch](https://git.launchpad.net/ubuntu/+source/systemd/plain/debian/patches/debian/Bring-tmpfiles.d-tmp.conf-in-line-with-Debian-defaul.patch?h=applied/ubuntu/noble-updates)).
- This host: `q /tmp … 10d`, `/tmp` tmpfs, no override (O); `systemd-tmpfiles-clean.timer` `static` (O).
- Age uses atime/mtime/ctime (ctime ignored for directories) (V,
  [tmpfiles.d(5)](https://www.freedesktop.org/software/systemd/man/latest/tmpfiles.d.html)).
- v255 cleaner **skips sockets present in `/proc/net/unix`** (path-string match; assumes alive if the file
  is unreadable), skips sticky-bit files, and skips any directory/file it cannot `flock(LOCK_EX|LOCK_NB)`;
  non-empty directories are not removed (V,
  [`tmpfiles.c` L440-497, 739-742, 780-784, 799-802, 827-833](https://github.com/systemd/systemd/blob/v255/src/tmpfiles/tmpfiles.c#L799-L802)).
  The manual documents the lock exemption (V, tmpfiles.d(5) "Age"). `unix_socket_alive` exists in v219-v255
  (grep per tag); introduction version and 262.1 behaviour U.
- **I:** a bound live socket survives systemd cleanup on Ubuntu 24.04 and Arch when `/proc/net/unix` is
  readable and paths match; the exploratory 1-hour `utimensat` refresher (`runtime.rs:28-73`) is not shown
  necessary. The cleaner was not run (destructive).
- Only the socket and its directory live in `/tmp`; the Darwin `climon2-<uid>-pty` lock directory
  (`pty.rs:186-190`) would too if reused.
- A removed socket gives `ENOENT`, one of the two errors that trigger the stale check
  (`list.rs:244-249`); with §1 this is how a live session's metadata could be removed. No rebind code
  found in `crates/climon2-daemon/src` (I). Non-systemd cleaners not inspected (U).
- Mechanisms (none chosen): rely on live-socket skip; hold `flock` on the runtime directory/socket
  (documented); refresh timestamps; `$XDG_RUNTIME_DIR` (rejected by ADR 0003); rebind on `ENOENT` (contract
  change).

## 8. Avoiding signals to unrelated processes on a shared host

- Raw-PID signals after possible reaping: `tests/process.rs:285-288`; `lifecycle.rs:225,517,796,1564,1612-1614`;
  `crates/climon2-client/tests/bridge.rs` (e.g. 545, 603, 682, 700); `climon2-cli/tests/exec.rs:173,180`.
- `signal_group(i32::MAX, Kill)` as a "no real group" value (`tests/process.rs:267-268`) stays safe: Linux
  `pid_max` ≤ 2²² (V, proc_sys_kernel(5)) (I).
- Options: pidfd for processes the harness spawns (≥5.2); re-verify `/proc/<pid>` identity immediately
  before signalling (narrows, does not close the window); PID namespace (needs userns; AppArmor-restricted
  by default on 24.04, V
  [release notes](https://discourse.ubuntu.com/t/ubuntu-24-04-lts-noble-numbat-release-notes/39890);
  usability U); cgroup v2 `cgroup.kill` (≥5.14, delegation U; would change the setsid-escape rule);
  dedicated UID/container/VM (removes cross-user, not same-user risk; kill(2) permission rule V);
  `PR_SET_CHILD_SUBREAPER` in the harness to reap orphans.
- Mutation tests mutate guards such as `group <= 1` (`process.rs:174`); existing tests avoid passing those
  values to the real syscall (`tests/process.rs:259-262`); `kill(-1)` hits every permitted process (V,
  kill(2)). A port must preserve this.

## 9. Ubuntu 24.04 vs Arch

| Item | Ubuntu 24.04 baseline | Arch / this host |
|---|---|---|
| Kernel | GA 6.8 (HWE newer) | rolling; O `7.2.15` |
| systemd | 255.4 | O 262.1 |
| `/tmp` rule | `D … 30d` (V) | `q … 10d` (O) |
| `/tmp` fs | U | O tmpfs |
| `PIDFD_SIGNAL_PROCESS_GROUP` | absent on 6.8 (V+I) | available (I) |
| Unprivileged userns | AppArmor-restricted (V) | not tested |
| `/proc` options | U | O no `hidepid` |
| `yama.ptrace_scope` | U | O 1 |

An Arch-only pass cannot evidence the Ubuntu baseline for the rows that differ (I).

## Unresolved questions for later decision tickets

1. Start-time representation (§1, options A-D) and whether a tolerance is an ADR 0005 exception.
2. Zombie daemon/leader: same process or dead?
3. Leader-only `SIGHUP` at master close vs the glossary's **Hang-up grace** wording: should the daemon also
   signal the foreground group?
4. Membership scope: session+recorded groups vs cgroup/PID-namespace; setsid escapes.
5. Runtime-directory protection (§7) and correcting ADR 0003's Ubuntu "10 days, sockets included".
6. Socket path limit 103 vs a Linux value (≤107).
7. Support for missing/restricted `/proc`; mapping to "uncertain".
8. Test-isolation mechanism before lifecycle/mutation tests run on a shared account.
9. Need for the PTY allocation lock on Linux (U).
10. Kernel-version differences beyond v6.8 (U).

## Scope

No Linux code, ADR edit, contract exception, map/ticket edit, or exploratory-branch change is authorized or
made. #129 remains open.

## Sources

Repo at `153dd5c`: ADRs 0003/0005, `GLOSSARY.md`, `crates/climon2-platform/src/{process,runtime,peer,pty,daemon}.rs`,
`crates/climon2-platform/tests/{process,runtime}.rs`, `crates/climon2-daemon/src/{pty,startup}.rs`,
`crates/climon2-daemon/src/session/command.rs`, `crates/climon2-daemon/tests/lifecycle.rs`,
`crates/climon2-cli/src/{list,exec}.rs`, `crates/climon2-core/src/{layout,protocol}.rs`. Exploratory (unmerged):
`/home/jack/dev/climon2.worktrees/linux` `3ada49f`. External: man7.org pages cited inline; POSIX.1-2024 XBD;
Linux v6.8 `fs/proc/{array,stat}.c`, `kernel/{fork,exit}.c`, `kernel/time/timekeeping.c`,
`drivers/tty/{pty,tty_io,tty_jobctrl,n_tty}.c`; systemd v255 `tmpfiles.c`, `tmp.conf`, tmpfiles.d(5); Ubuntu
`systemd` noble-updates Launchpad tree; Ubuntu 24.04 release notes; portable-pty 0.9.0.
