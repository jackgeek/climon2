# Platform seams as cfg modules; shells only in tests

The runtime supports macOS 15+ on Apple Silicon and Intel, and x86_64
GNU/Linux with an Ubuntu 24.04/glibc 2.39 release baseline. Arch Linux is a
native development host. Windows (x86_64 MSVC, Windows 11 build 26100 or
newer) is built to these decisions; its I/O and transport decisions are under
[Windows I/O and transport](#windows-io-and-transport),
and its process lifecycle under
[Windows lifecycle](#windows-lifecycle). Each
platform-specific concern is its own module, with platform-neutral function
signatures and `#[cfg(target_os = …)]` bodies. There are no trait objects or
runtime platform dispatch. Unsupported targets hit `compile_error!` inside
the platform modules and runtime binaries. The product never knows which
shell it runs, because any PTY program is supported. Shells exist only in
the test harness as shell profiles.

## Platform seams

1. Launcher hand-off
2. Runtime directory and socket
3. Peer check
4. Hanging up the session
5. Daemon spawn and detach
6. PID liveness
7. Client raw mode and resize notification
8. Installer default directory and same-version clobber
9. Exit-status mapping
10. Process start time (for the stale-metadata PID-reuse check, ADR 0005)
11. Monitor link sockets: a TCP listener and dialler that keep connect, accept
    and read deadlines, the dialler also taking a host name as typed (#501)
12. Key storage: the owner-only `links/` folder, atomic writes, and a damaged
    file as an error
13. Randomness from the operating system's generator
14. The clock: monotonic time for deadlines and wall time for dates
15. The home lock: a lock one process at a time holds on a file in a climon2
    home, released when its holder ends
16. The host name, which each side of a pairing suggests as its name (#501)

Seams 11 to 16 serve monitor links
([#488](https://github.com/jackgeek/climon2/issues/488)). The link code
reaches none of them: its callers do, and pass time and randomness in. Each
has one contract suite, `tests/seam_*.rs` in `climon2-platform`, that every
platform's implementation passes by itself. Sockets, key storage, the home
lock and the host name are Unix-only until monitor links on Windows are specified.

More seams may emerge after the first release. Everything else counts as platform-neutral.

## Transport

Clients reach daemons over AF_UNIX stream sockets on every OS, Windows 11 included (through socket2; build 26100 or newer, per the [Windows platform contract](https://github.com/jackgeek/climon2/issues/212)). Named pipes are not used, so session metadata keeps a socket path.

## Linux implementation

- Keep `/tmp/climon2-<uid>-<hash8>`, not `$XDG_RUNTIME_DIR`: the latter is
  optional and tied to login lifetime. Ubuntu 24.04's systemd-tmpfiles 255
  exempts live Unix sockets discovered through `/proc/net/unix`; the earlier
  claim that it unconditionally removes bound sockets after ten days was
  incorrect. The daemon additionally refreshes socket/directory timestamps
  hourly for age-based cleanup policies. Explicit removal by an administrator
  is not prevented.
- Use `SO_PEERCRED` for same-user peer authentication, in addition to private
  runtime-directory permissions. Retain the conservative 103-byte socket-path
  limit on both Unix platforms.
- Use procfs for process groups, controlling terminals and process identity.
  Exclude zombies from live candidates. A clock-independent boot ID and start
  ticks establish daemon identity; epoch start time remains a frozen metadata
  field, not Linux's stale-cleanup authority (ADR 0005).
- Procfs inspection errors are not evidence of process death. All participants
  sharing session metadata must share a PID namespace. Cross-namespace shared
  homes are not supported.
- Hanging up closes the PTY master, as closing a terminal window does. Linux
  wakes terminal readers before it signals the session leader, so a leader
  blocked reading the terminal can end with its own status (an interactive
  shell's 0, `cat`'s 1) rather than SIGHUP's 129; under CPU contention this is
  the usual outcome. macOS signals first. climon2 relays the command's actual
  status and does not emulate the macOS order, which would send a surviving
  leader a second SIGHUP.
- Native process and peer implementations live in separate source files so
  coverage-backed complexity and mutation run on the operating system that
  actually compiles them. Shared source is qualified on the Linux development
  host; native CI retains runtime and archive coverage on both platforms.

## Windows I/O and transport

Decided in [Define Windows I/O and transport architecture](https://github.com/jackgeek/climon2/issues/211) (owner, 2026-11-19). Linux and macOS behaviour does not change.

- **Seams.** The ADR 0003 seams in `climon2-platform` grow to cover socket
  streams and listeners, waiting for readiness, terminal handles and PTY I/O.
  Client `bridge`/`transport`, daemon `pty`/`session`/`connection` and CLI
  `list` stop calling `libc` directly. Unix bodies move unchanged. Windows
  bodies are registered native files, so the tick-loop logic stays shared and
  qualified on the Linux development host.
- **Multiplexing.** One thread per blocking handle: ConPTY output and input,
  console input and console output. Sockets stay on `WSAPoll` inside the
  existing tick loops. Thread channels are bounded to the byte limits the Unix
  loops already enforce, and a full channel behaves like a full Unix buffer.
  Nothing relies on `CancelSynchronousIo`: the console input thread waits on
  its handle with a timeout and a stop flag, the ConPTY reader ends on a broken
  pipe, and writers end when their channel closes.
- **ConPTY.** A pinned, bundled `conpty.dll` and `OpenConsole.exe` (Microsoft's
  `Microsoft.Windows.Console.ConPTY` package, stable releases only) live in
  each version folder beside `climon2-daemon.exe`. The inbox engine re-renders
  output and answers queries itself, which ADR 0007 forbids, so it is never
  used. Only the daemon loads `conpty.dll`, by full path from its own folder
  with `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32`. If
  either file is missing, `exec` fails with 125, because `conpty.dll` would
  otherwise fall back silently to inbox `conhost.exe`. The daemon calls the DLL
  through `windows-sys`; portable-pty stays the Unix backend.
- **End of output.** The daemon releases the pseudoconsole
  (`ConptyReleasePseudoConsole`) after starting the command. `ERROR_BROKEN_PIPE`
  on the output pipe replaces Unix `EIO`.
- **Runtime directory.** `%LOCALAPPDATA%\c2\<hash8>\<uuid>.sock`, where
  `<hash8>` is the climon2 home hash as on Unix and there is no uid segment.
  `%TEMP%` is rejected because Disk Cleanup runs daily and deletes week-old
  files and directories there, and Storage Sense deletes temp files not in use.
  Nothing ages `%LOCALAPPDATA%`, so Windows has no freshness refresh. The path
  is 77 bytes plus the profile folder name, within AF_UNIX's 107 usable bytes.
  Over budget, `exec` fails with 125. Core's runtime layout takes the uid as
  optional, so Unix names do not change.
- **Access control.** The `<hash8>` directory is created with the private
  descriptor (see Private files and folders): its owner is set explicitly to
  the current user's SID, and its protected DACL holds one ACE, that SID with
  full control. Setting the owner explicitly is an amendment from
  [Define secure creation of Windows log and metadata files](https://github.com/jackgeek/climon2/issues/307):
  an administrator's full token makes `BUILTIN\Administrators` the owner of
  anything it creates without naming an owner, and the folder would then fail
  the owner check (see *Why the owner is explicit* below). Before each use it
  must not be a reparse point, its owner must be that SID, and its DACL must be
  exactly that. Otherwise preparation fails with the Unix "unsafe runtime
  directory" error. The socket file inherits the DACL. Windows AF_UNIX has no
  peer credentials, so the per-connection peer check is a no-op. **This is
  weaker than Unix:** `SO_PEERCRED` rejects root, while a Windows
  administrator can bypass the DACL with `SeRestorePrivilege`. That is
  accepted, because an administrator can already read the daemon's memory.
- **Private files and folders.** Decided in
  [Define secure creation of Windows log and metadata files](https://github.com/jackgeek/climon2/issues/307)
  (owner, 2026-11-21), after measurements on a Windows 11 VM. Windows has no
  `O_NOFOLLOW` and no file modes, so `0700`, `0600` and `O_NOFOLLOW` map as
  follows. Unix rules do not change.
  - *Scope.* Every file climon2 creates: all logs, including the client
    library's log at its caller's path; session metadata and its temporary
    file; the monitor preference file and its temporary file. Every folder it
    manages: `logs`, `logs\general`, `logs\<id>` and `sessions`. The climon2
    home itself is treated as on Unix: created when missing, canonicalised and
    not checked.
  - *Private descriptor.* Each managed folder and created file gets an explicit
    security descriptor at creation: owner set to the process token's user
    SID, and a protected DACL with one ACE, that SID with full control
    (inheritable on folders). SYSTEM and Administrators are not listed; an
    administrator can still bypass the DACL, as accepted above.
  - *Why the owner is explicit.* A new file or folder is owned by the SID its
    descriptor names or, when it names none, by the creating token's default
    owner. Inheritance never supplies the owner. Measured on 2026-11-22 with
    [the owner probe](https://github.com/jackgeek/climon2/blob/0feb0122cb217396b715ea15c849ee2be6d61d12/probes/elevated-owner/probe.ps1)
    ([CI run](https://github.com/jackgeek/climon2/actions/runs/45003024593)):
    every administrator token at high integrity defaults to
    `BUILTIN\Administrators`. That covers the elevated half of a UAC split
    token, any administrator while UAC is off (as on the owner's Windows 11
    VM), and the built-in Administrator (RID 500) that the `windows-2022` and
    `windows-2025` runners use, whose token UAC does not filter. Only a
    filtered, medium-integrity token defaults to its user. A descriptor that
    carries a DACL but no owner still gets `BUILTIN\Administrators`. Setting
    `nodefaultadminowner` ("System objects: Default owner for objects created
    by members of the Administrators group") to 0 or 1 without a reboot
    changed no fresh logon's default owner. With the owner named in the
    descriptor, every token tested got its user's SID.
  - *Final component.* Every create goes through one `climon2-platform` helper
    that calls `CreateFileW` with `CREATE_NEW | FILE_FLAG_OPEN_REPARSE_POINT`
    and the private descriptor. Plain std `create_new` is never used: it follows
    a dangling symbolic link and creates the link's target, wherever that is.
    With the flag, any existing name, including a dangling link, fails. Readers
    follow links, as on Unix, relying on the private folders. Metadata and the
    preference file are still published by renaming a temporary file, which
    replaces a link at the destination rather than writing through it.
  - *Parent folders.* Check, then use, as on Unix. Before each use a managed
    folder must not be a reparse point of any tag (symbolic link, junction,
    mount point, cloud placeholder), must be a directory, must be owned by the
    user's SID and must hold exactly the private DACL. Folders above `logs` and
    `sessions` are not checked. Handle-relative creation (`NtCreateFile` with
    a root directory handle) was rejected: only the user and administrators can
    write into an owner-only folder, which is the guarantee Unix relies on too.
  - *Unsafe existing folders.* Refused without repair, with the Unix "unsafe
    runtime directory" error naming the path and the problem. A folder made by
    hand, or by an elevated tool, inherits the profile's ACL and is refused
    until the user deletes it.
  - *Client library log.* The final-component rule applies at the caller's
    path; the caller's folder is not checked. A refusal is the existing
    "log_path cannot be created as a new file" failure.
  - *Evidence.* Tests in registered `windows.rs` files: a dangling file link
    at the final name is refused and nothing appears at its target; an
    existing file is refused; a junction or directory link standing in for
    `logs` is refused and pruning leaves its target alone; a folder with the
    inherited descriptor (owned by Administrators on the elevated CI runner)
    and one with an extra ACE are each refused and left unrepaired; created
    files and folders carry exactly the private descriptor; the client
    library refuses a dangling link at its log path. Cross-account denial and
    the log-privacy scenario are in the Windows quality and qualification
    documents.
- **Stale sockets.** The Unix rule applies: a NotFound or ConnectionRefused
  connect plus proven-dead daemon identity permits cleanup. A socket is
  recognised by its reparse tag `IO_REPARSE_TAG_AF_UNIX`, since std reports it
  as a regular file. A stale file is deleted before re-binding.
- **Client console.** `exec` and `attach` require stdin and stdout to be
  console handles; redirected stdio and mintty are refused like a missing TTY.
  Input is plain VT: the console is put in
  `ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_WINDOW_INPUT` with processed input
  off, and `ReadConsoleInputW` supplies key characters and
  `WINDOW_BUFFER_SIZE_EVENT` resizes. Keys VT cannot express, such as
  Shift+Enter, are a documented limitation. Ctrl+Break reaches the session
  through a console control handler instead, and a Break request to the
  daemon ([#326](https://github.com/jackgeek/climon2/issues/326))
  ([Windows lifecycle](#windows-lifecycle)). Output enables
  `ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN` and the
  UTF-8 code page, and writes raw bytes with `WriteFile`, never splitting a
  UTF-8 character across writes (a pure `climon2-core` boundary function, used
  only on Windows). Every exit path restores both console modes and code pages.

## Windows lifecycle

Decided in [Define Windows platform support and lifecycle contract](https://github.com/jackgeek/climon2/issues/212) (owner, 2026-11-21), after measurements on a Windows 11 VM. The full contract and its exceptions are in [Windows platform and lifecycle contract](../for-humans/windows/lifecycle.md).

- **Platform.** Windows 11 build 26100 or newer, documented and not checked at
  run time. Windows 10 is not supported.
- **Daemon spawn and detach.** A raw `CreateProcessW` with `DETACHED_PROCESS`
  and, inside a job, `CREATE_BREAKAWAY_FROM_JOB`. If the daemon is still in a
  job afterwards, `exec` exits 125; there is no fallback route.
- **Hanging up.** One job object per session, allowing breakaway and killing
  on close. Hang-up closes the pseudoconsole, waits the full five seconds, then
  terminates the job with `0xC000013A`.
- **Launcher hand-off.** Every Unix `exec` hand-off, including the monitor's,
  becomes spawn and wait, passing the child's raw `u32` status through.
- **Exit-status mapping.** Statuses are full-width; there is no
  128 + signal on Windows.
- **Process start time.** `GetProcessTimes` creation time in the frozen
  `daemon_start_time`.

## Notes for future ports

- **Windows peer check and runtime directory:** superseded by [Windows I/O and transport](#windows-io-and-transport).
- **Windows hang-up, launcher and exit status:** superseded by [Windows lifecycle](#windows-lifecycle). The launcher's contract freezes when Windows first ships (ADR 0002).

## Shell profiles

Adding zsh, sh, pwsh, powershell or cmd is purely a test-harness change: a new shell profile. Known controlled-startup flags: bash `--norc --noprofile`; zsh `-f`; sh with `ENV` unset (macOS `/bin/sh` is bash 3.2 in POSIX mode, Ubuntu's is dash); pwsh and powershell `-NoProfile`; cmd `/D`.

The Windows shell profiles, the host models and the scenarios they run are specified in [Windows host and shell qualification](../for-humans/windows/qualification.md).

## Considered options

- **Platform traits with runtime dispatch:** rejected. They add speculative abstraction for implementations that don't exist yet.
- **Named pipes on Windows:** rejected in favour of one transport everywhere.
- **Inbox ConPTY on Windows:** rejected; it re-renders output and answers terminal queries, so ADR 0007 could not hold, and it differs per Windows build.
- **A Windows per-connection peer check through `SIO_AF_UNIX_GETPEERPID`:** rejected; it is undocumented, exposed to PID reuse, and adds no boundary against administrators.
- **`WSAEventSelect`, IOCP or the `polling` crate as the Windows waiter:** rejected; every option still needs ConPTY threads, and these add edge-triggered semantics, a pipe rewrite or undocumented AFD calls.
- **Windows sibling files in each crate:** rejected in favour of `climon2-platform` seams, which keep one copy of the loop logic.
- **win32-input-mode input on Windows:** rejected; it adds a Windows-only input dialect for clients and the monitor.
- **`%TEMP%` as the Windows runtime root:** rejected; Windows cleans it automatically.
- **Shell awareness in the product:** rejected. climon2 runs any PTY program.

## Consequences

- Native CI runs full runtime/terminal and archive qualification on Linux
  x86_64 and both macOS architectures. The dedicated Linux development host
  owns shared/Linux CRAP <= 5 and zero-survivor mutation; CI qualifies only
  registered macOS-exclusive source at the same thresholds. OS-only bodies,
  including installer exchange and daemon identity adapters, live in native
  source files so this split cannot omit them. Native-only CI still exercises
  real workspace callers for coverage and the unchanged-worker baseline.
  Windows CI runs the runtime and archive tests that build on Windows on
  `windows-2025`, and the Windows-only CRAP and mutation gate qualifies
  registered Windows-exclusive source at the same thresholds
  ([Windows quality and CI qualification](../for-humans/windows/quality.md)).
- Local full quality gates must pass for the exact release commit before
  tagging; this maintainer-owned check is not duplicated or certified by CI.
- One release publishes the Linux, macOS and Windows archives only after every
  native gate passes (#330); see [distribution](../for-humans/distribution.md).
