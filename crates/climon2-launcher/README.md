# climon2-launcher

The dependency-free `climon2` executable implements ADR 0002's frozen contract.
It canonicalizes its own path, reads sibling `current`, validates an exact
SemVer version (optional single LF or CRLF), then Unix-execs
`<executable home>/<version>/climon2-cli`.

Arguments (including argv[0]), environment, cwd and inherited stdio are
unchanged. The CLI's status or signal is therefore the launcher's status or
signal. Neither PATH, `CLIMON2_EXE`, nor cwd selects companions.
Failures emit one `climon2: error: …` line and exit 125.

`tests/launcher.rs` exercises the real binary for #17/#33 and ADR 0002, including
non-UTF-8 argv/environment, empty arguments, spaces/Unicode, symlink invocation,
stdin/stdout/stderr, status/signals and malformed/missing layout entries.
The workspace architecture test enforces the empty dependency list.
