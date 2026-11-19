# Platform seams as cfg modules; shells only in tests

The runtime supports macOS 15+ on Apple Silicon and Intel, and x86_64
GNU/Linux with an Ubuntu 24.04/glibc 2.39 release baseline. Arch Linux is a
native development host. Windows (x86_64 MSVC, Windows 11) is specified, not
implemented; its I/O and transport decisions are under
[Windows I/O and transport](#windows-io-and-transport-agreed-not-implemented). Each
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

More seams may emerge after the first release. Everything else counts as platform-neutral.

## Transport

Clients reach daemons over AF_UNIX stream sockets on every OS, Windows 11 included (through socket2; the minimum build belongs to the [Windows platform contract](https://github.com/jackgeek/climon2/issues/212)). Named pipes are not used, so session metadata keeps a socket path.

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

## Windows I/O and transport (agreed, not implemented)

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
- **Access control.** The `<hash8>` directory is created with a protected DACL
  holding one ACE: the current user's SID, full control. Before each use it
  must not be a reparse point, its owner must be that SID, and its DACL must be
  exactly that. Otherwise preparation fails with the Unix "unsafe runtime
  directory" error. The socket file inherits the DACL. Windows AF_UNIX has no
  peer credentials, so the per-connection peer check is a no-op. **This is
  weaker than Unix:** `SO_PEERCRED` rejects root, while a Windows
  administrator can bypass the DACL with `SeRestorePrivilege`. That is
  accepted, because an administrator can already read the daemon's memory.
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
  Ctrl+Break and Shift+Enter, are a documented limitation. Output enables
  `ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN` and the
  UTF-8 code page, and writes raw bytes with `WriteFile`, never splitting a
  UTF-8 character across writes (a pure `climon2-core` boundary function, used
  only on Windows). Every exit path restores both console modes and code pages.

## Notes for future ports

- **Windows peer check and runtime directory:** superseded by [Windows I/O and transport](#windows-io-and-transport-agreed-not-implemented).
- **Windows hang-up:** ClosePseudoConsole (which delivers CTRL_CLOSE_EVENT), then 5 s, then TerminateProcess. The daemon calls the bundled `conpty.dll` directly, not portable-pty. Timing belongs to the [Windows platform contract](https://github.com/jackgeek/climon2/issues/212).
- **Windows launcher:** it can't `exec`, so it spawns the CLI, waits, exits with the child's code, and ignores Ctrl+C. Its contract freezes when Windows first ships (ADR 0002).
- **Windows exit status:** the Exit frame's `code` must carry a raw `u32` code (for example `0xC000013A`); 128+signal is Unix-only. The post-hello protocol is version-specific (ADR 0001), so widening it later is cheap.

## Shell profiles

Adding zsh, sh, pwsh, powershell or cmd is purely a test-harness change: a new shell profile. Known controlled-startup flags: bash `--norc --noprofile`; zsh `-f`; sh with `ENV` unset (macOS `/bin/sh` is bash 3.2 in POSIX mode, Ubuntu's is dash); pwsh and powershell `-NoProfile`; cmd `/D`.

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
  Windows retains platform-neutral contract tests, not runtime qualification.
- Local full quality gates must pass for the exact release commit before
  tagging; this maintainer-owned check is not duplicated or certified by CI.
- One release publishes both platform archives only after both native gates
  pass; see [distribution](../distribution.md).
