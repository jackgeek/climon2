# Platform seams as cfg modules; shells only in tests

The runtime supports macOS 15+ on Apple Silicon and Intel, and x86_64
GNU/Linux with an Ubuntu 24.04/glibc 2.39 release baseline. Arch Linux is a
native development host. Windows ConPTY remains future work. Each
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

Clients reach daemons over AF_UNIX stream sockets on every OS, Windows included (Win10 1809+, the same minimum as ConPTY). Named pipes are not used, so session metadata keeps a socket path.

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
- Native process and peer implementations live in separate source files so
  coverage-backed complexity and mutation run on the operating system that
  actually compiles them. Shared source is qualified on the Linux development
  host; native CI retains runtime and archive coverage on both platforms.

## Notes for future ports

- **Windows peer check:** Windows AF_UNIX has no peer credentials. Access control comes only from the runtime directory: its owner SID is checked, and its DACL grants only the current user. The per-connection peer check is a no-op.
- **Windows runtime directory:** under `%TEMP%`, using the same `<hash8>` scheme, within AF_UNIX's 108-byte path limit.
- **Windows hang-up:** ClosePseudoConsole (which delivers CTRL_CLOSE_EVENT), then 5 s, then TerminateProcess. portable-pty's `kill()` terminates immediately, so it can't provide the grace period.
- **Windows launcher:** it can't `exec`, so it spawns the CLI, waits, exits with the child's code, and ignores Ctrl+C. Its contract freezes when Windows first ships (ADR 0002).
- **Windows exit status:** the Exit frame's `code` must carry a raw `u32` code (for example `0xC000013A`); 128+signal is Unix-only. The post-hello protocol is version-specific (ADR 0001), so widening it later is cheap.

## Shell profiles

Adding zsh, sh, pwsh, powershell or cmd is purely a test-harness change: a new shell profile. Known controlled-startup flags: bash `--norc --noprofile`; zsh `-f`; sh with `ENV` unset (macOS `/bin/sh` is bash 3.2 in POSIX mode, Ubuntu's is dash); pwsh and powershell `-NoProfile`; cmd `/D`.

## Considered options

- **Platform traits with runtime dispatch:** rejected. They add speculative abstraction for implementations that don't exist yet.
- **Named pipes on Windows:** rejected in favour of one transport everywhere.
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
