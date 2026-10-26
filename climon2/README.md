# climon2

A terminal session consists of a detached session daemon, its PTY (ConPTY on
Windows), and a controlling client. This first runtime ends the session when
the command exits or the controlling client disconnects.

## Package layout

```text
climon2
current
versions/0.1.0/climon2-cli
versions/0.1.0/climon2-daemon
versions/0.1.0/climon2-client
```

Executable names end in `.exe` on Windows. `current` contains a strict SemVer
version, optionally followed by one LF or CRLF. Add the package directory to
PATH; selection is relative to the launcher's real executable, not the working
directory. Each CLI uses only companions beside its own real executable.

```text
climon2 exec <command> [args...]
climon2 list
climon2 list --json
```

`exec` requires terminal stdin and stdout. Commands run without an implicit
shell, except Windows CMD/BAT files, which use the inherited ComSpec.
Terminal input, output, initial dimensions, resize events, and command exit
status pass through. The CLI restores the invoking terminal even if its
controlling client is killed. Climon2 errors use status 125 and a `climon2: `
stderr diagnostic; a command's own status 125 remains unchanged.

## State and discovery

A nonempty CLIMON2_HOME overrides the state directory, relative to the invoking
working directory when necessary. Otherwise the directory is `.climon2`
beneath HOME (Unix) or USERPROFILE (Windows); there is no account-database
fallback. Session records are atomically published as private UTF-8 JSON files
at `sessions/<guid>.pty`.

AF_UNIX sockets reside in XDG_RUNTIME_DIR, or `/tmp/climon2-<uid>` on Unix,
and LOCALAPPDATA/climon2/run on Windows. An existing runtime directory must
already be user-owned and user-only. Accepted peers must have the daemon's
UID or token SID; Windows peer-PID verification is mandatory and fails closed.
Startup checks both connected socket ends before starting a command. Windows
may return a valid peer PID without updating the ioctl byte count; authorization
still requires opening that process and verifying its token SID. An absent or
invalid PID fails closed; ACL-only authorization is not a fallback.
Windows sockets receive a user-only default DACL at bind time using a temporary
thread token, without changing the process token or repairing runtime-directory
permissions. Listing explicitly opens the session directory with list access
and checks its DACL, so an access-denied empty directory cannot be mistaken
for missing state even when Windows enumeration or backup semantics bypass
the list-access denial.

Discovery sends a protocol-1 hello in a four-byte big-endian length-prefixed
JSON frame. Frames are limited to 1 MiB, and each handshake has one total
two-second deadline. A record is listed only if filename, record, and live
hello session IDs agree. Discovery never attaches or deletes stale state.
Table values are control-escaped; JSON preserves full metadata values.

## Implementation

`protocol` owns bounded framing and live probes; `discovery` owns metadata and
listing; `session` coordinates startup, attachment, output draining and cleanup.
`unix` and `windows` provide native terminals, process lifetime management,
private filesystem state, peer authentication and local-time conversion.
Each executable's role is fixed by its Cargo binary target, independent of its
filename; runtime and companion lookup still use its real executable location.
Windows command startup uses a ConPTY-connected helper so Rust's process
creation can report launch failures before session publication; the helper and
command are placed in a separate kill-on-close job before either can execute.
Only the startup acknowledgement handle is inherited by the helper; the daemon's
CLI control pipes are excluded. The command's standard streams are explicitly
opened on CONIN$/CONOUT$, rather than inheriting the detached daemon's pipes.
ConPTY output is drained concurrently with console creation and command startup,
so synchronous console writes cannot block the startup acknowledgement.
Daemon startup requests job breakaway only when the enclosing job permits it;
a directly invoked CLI also works without a launcher-created breakaway job.
The detached daemon stays outside the launcher's CLI/client job so it can
perform cleanup after launcher closure. The launcher handles Ctrl+C locally
and clears any inherited ignore flag before creating the CLI, so ignoring the
interrupt does not disable delivery to its child.
