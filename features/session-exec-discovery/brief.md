# Brief: session-exec-discovery

> Input for the Contract Author. This is user prose, not an approved contract.

## What I want

`climon2 exec <command> [args…]` starts a climon2 session running `<command>` with `[args…]` (e.g. `climon2 exec bash`, `climon2 exec copilot --resume`) and makes the user's terminal interactive with it immediately. `climon2 list` shows the sessions that are currently running. Everything ships in the climon2 install layout from day one.

## Behaviour agreed so far

### Install layout
- Package layout: `climon2` launcher, `current` (one line: SemVer), `versions/<v>/{climon2-cli, climon2-daemon, climon2-client}` (`.exe` on Windows).
- The launcher runs the current version's CLI.
  - Unix: it replaces itself with the CLI using `exec`.
  - Windows: it spawns the CLI on the same console in a kill-on-close job object, ignores Ctrl+C itself, and propagates the CLI's exit code.
  - It preserves arguments exactly, including spaces and Unicode, and passes through the environment and stdio.
  - If `current` is missing or invalid, or the version folder is missing, it exits with the climon2 error code and names the problem.
- The CLI finds its companion executables in its own real directory, never via PATH.
- Installing and upgrading are manual: extract the package and add it to PATH.

### exec
- Starts a detached session daemon owning a real PTY (Unix) or ConPTY (Windows), running the command with its exact arguments in the current working directory.
- The CLI then launches the same version's client, which attaches as the first and controlling client:
  - Command: `climon2-client --protocol 1 attach --metadata <path>`.
  - The client starts with the user's current terminal size and forwards resizes.
- Environment:
  - Everything passes through.
  - `CLIMON2_SESSION_ID` is set to the session GUID.
  - `CLIMON2_SESSION_DEPTH` is set to the session depth: 1 at top level, otherwise the enclosing session's depth plus 1. A malformed inherited value is treated as 1 for the enclosing session.
  - `TERM` defaults to `xterm-256color` if unset.
- Nesting is allowed. A nested start prints `Starting climon2 session nested for the Nth time` to stderr, where N = depth − 1, written as an English ordinal (1st, 2nd, 3rd, …).
- Exit:
  - When the command exits, the session ends and `climon2 exec` exits with the command's exit code, or 128 + signal number.
  - Climon2's own failures use a distinct climon2 error code.
  - If the client disconnects, the session ends. Feature 2 will amend this.
- If there's no TTY, `exec` fails clearly with the climon2 error code.

### Session metadata
- Session metadata is stored at `<climon2 home>/sessions/<guid>.pty`. Climon2 home defaults to `~/.climon2`, and `CLIMON2_HOME` overrides it.
- Metadata is JSON with these fields:
  - `format_version`, `session_id`, `socket_path`
  - `daemon_pid`, `daemon_start_time`
  - `command`, `args`, `cwd`, `depth`
  - `created_at`, `climon2_version`
- Metadata writes are atomic, and the file is readable only by the user.

### Session daemon connection
- The session daemon listens on a Unix domain socket in a per-user runtime directory:
  - Unix: `$XDG_RUNTIME_DIR`, or `/tmp/climon2-<uid>/`.
  - Windows: `%LOCALAPPDATA%\climon2\run\`, with hashed socket names.
- Only the session's own user may connect:
  - The runtime directory must be user-only (mode 0700 on Unix, a user-SID-only ACL on Windows). A directory that's too permissive is refused.
  - The daemon checks the connecting user: peer UID on Unix, peer SID on Windows.
  - Other users are rejected.
- Protocol:
  - Messages are versioned and length-prefixed, with a 1 MiB maximum.
  - The hello handshake has a 2-second deadline and returns the session ID.
  - Unknown message types are rejected.

### list
- `climon2 list` shows only live sessions. A session is live if its daemon answers the hello handshake with the matching session ID.
- Stale metadata is hidden, not deleted.
- Table columns:
  - SESSION: the full GUID
  - CREATED: local time
  - COMMAND
  - ARGS: shell-quoted, truncated with `…`
- Sessions are listed newest first.
- If there are no live sessions, `list` prints `No climon2 sessions running.`
- Control characters are escaped in the table.
- `climon2 list --json` output is untruncated, includes `depth`, and JSON-escapes control characters.

## Platforms and shells

- Windows Server 2025: `powershell.exe`, `pwsh.exe`, `cmd.exe`
- Ubuntu 24.04: `bash`, `sh`
- macOS: `zsh`, `bash`, `sh`

Every platform also runs the generic terminal fixture. Each shell gets an interactive smoke test (marker, discovery, exit, cleanup), with controlled shell startup.

## Out of scope

- Surviving client disconnect, attaching to an existing session, reclaiming control, and restoring the screen are features 2 to 5.
- Commands to install a new version, switch versions, or clean up old versions (feature 6).
- Fish, zsh on Linux, and tests with real Copilot.

## Open questions for the contract

- The exact numeric value of the climon2 error code.
- Whether Windows Server 2025 supports `SIO_AF_UNIX_GETPEERPID`. If not, pause for a contract amendment instead of weakening the peer check.
