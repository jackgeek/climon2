# climon2-launcher

The dependency-free `c2` executable implements ADR 0002's frozen contract. It
is the only command name (#303); the rename did not change that contract.
It canonicalizes its own path, reads sibling `current`, validates an exact
SemVer version (optional single LF or CRLF), then hands off to
`<executable home>/<version>/climon2-cli` (`climon2-cli.exe` on Windows).

On Unix it execs the CLI. Arguments (including argv[0]), environment, cwd and
inherited stdio are unchanged. The CLI's status or signal is therefore the
launcher's status or signal. Neither PATH, `CLIMON2_EXE`, nor cwd selects
companions. Failures emit one `climon2: error: …` line and exit 125.

Windows cannot exec, so `src/handoff/windows.rs` spawns the CLI with the same
arguments, environment, cwd and handles, waits, and exits with its raw 32-bit
status (#315, docs/windows-lifecycle.md). It retries a missing or denied
`current` for up to one second, handles Ctrl+C and Ctrl+Break with a handler
routine, and on close, logoff or shutdown waits three seconds for the CLI
before exiting 129. It declares its few kernel32 calls itself, so it stays
dependency-free.

`tests/launcher.rs` exercises the real binary for #17/#33 and ADR 0002, including
non-UTF-8 argv/environment, empty arguments, spaces/Unicode, symlink invocation,
stdin/stdout/stderr, status/signals and malformed/missing layout entries.
`tests/windows.rs` exercises the real `c2.exe` on Windows, with its own test
binary as the CLI and console events in a pseudoconsole of their own.
The workspace architecture test enforces the empty dependency list.
