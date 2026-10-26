# Platform seams as cfg modules; shells only in tests

The first release targets macOS only, but Windows ConPTY and Ubuntu 24.04 must be addable without a redesign. Each platform-specific concern is therefore its own module, with platform-neutral function signatures and `#[cfg(target_os = …)]` bodies. There are no trait objects or runtime platform dispatch. Only the macOS bodies exist; other targets hit `compile_error!`, which is confined to the platform modules and the daemon and client binaries. The product never knows which shell it runs, because any PTY program is supported. Shells exist only in the test harness as shell profiles.

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

More seams may emerge after the first release. Everything else counts as platform-neutral.

## Transport

Clients reach daemons over AF_UNIX stream sockets on every OS, Windows included (Win10 1809+, the same minimum as ConPTY). Named pipes are not used, so session metadata keeps a socket path.

## Notes for future ports (none built now)

- **Windows peer check:** Windows AF_UNIX has no peer credentials. Access control comes only from the runtime directory: its owner SID is checked, and its DACL grants only the current user. The per-connection peer check is a no-op.
- **Windows runtime directory:** under `%TEMP%`, using the same `<hash8>` scheme, within AF_UNIX's 108-byte path limit.
- **Windows hang-up:** ClosePseudoConsole (which delivers CTRL_CLOSE_EVENT), then 5 s, then TerminateProcess. portable-pty's `kill()` terminates immediately, so it can't provide the grace period.
- **Windows launcher:** it can't `exec`, so it spawns the CLI, waits, exits with the child's code, and ignores Ctrl+C. Its contract freezes when Windows first ships (ADR 0002).
- **Windows exit status:** the Exit frame's `code` must carry a raw `u32` code (for example `0xC000013A`); 128+signal is Unix-only. The post-hello protocol is version-specific (ADR 0001), so widening it later is cheap.
- **Linux runtime directory:** keep `/tmp/climon2-<uid>-<hash8>`, not `$XDG_RUNTIME_DIR`. XDG isn't guaranteed over ssh, in containers or on CI, and is removed at last logout. Ubuntu's systemd-tmpfiles sweeps `/tmp` entries older than 10 days, sockets included, so the Linux port must keep live sockets fresh.

## Shell profiles

Adding zsh, sh, pwsh, powershell or cmd is purely a test-harness change: a new shell profile. Known controlled-startup flags: bash `--norc --noprofile`; zsh `-f`; sh with `ENV` unset (macOS `/bin/sh` is bash 3.2 in POSIX mode, Ubuntu's is dash); pwsh and powershell `-NoProfile`; cmd `/D`.

## Considered options

- **Platform traits with runtime dispatch:** rejected. They add speculative abstraction for implementations that don't exist yet.
- **Named pipes on Windows:** rejected in favour of one transport everywhere.
- **Shell awareness in the product:** rejected. climon2 runs any PTY program.

## Consequences

- A cross-OS CI job builds and unit-tests the platform-neutral code (protocol framing, hello, metadata, CLI argument parsing) on Linux and Windows. The launcher joins the Linux job (its Unix `exec` branch already works there) but stays out of the Windows job until Windows lands. The full test matrix stays macOS-only.
