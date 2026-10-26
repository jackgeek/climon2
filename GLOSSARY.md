# climon2

climon2 runs a terminal command inside a managed session that the user's terminal attaches to, and can list the sessions that are running.

## Sessions

**Session**:
One run of a command inside a terminal that climon2 owns, identified by a GUID.
_Avoid_: Job, instance, terminal

**Session daemon**:
The detached background process that owns a session's terminal and the command running in it.
_Avoid_: Server, host, backend

**Client**:
The process that connects the user's terminal to a session daemon.
_Avoid_: Viewer, frontend, terminal

**Controlling client**:
The attached client whose input and terminal size drive the session; when it leaves, control passes to another attached client, and the session ends only if none remains.
_Avoid_: Primary client, owner

**Hello handshake**:
The first exchange on every connection to a session daemon, in which the daemon identifies its session; frozen across all climon2 versions.
_Avoid_: Ping, version check

**Probe**:
A connection that completes the hello handshake without attaching, used to check whether a session is live.
_Avoid_: Health check, ping

**Session depth**:
How many sessions enclose a session, counting itself; 1 at top level.
_Avoid_: Nesting level, level

**Nested session**:
A session started from inside another session.
_Avoid_: Child session, sub-session

**Live session**:
A session whose daemon answers the hello handshake with the matching session ID.
_Avoid_: Running session, active session

**Stale metadata**:
Session metadata whose session is no longer live.
_Avoid_: Dead session, orphan

**Session metadata**:
The per-session record in climon2 home that describes a session and how to reach its daemon.
_Avoid_: Session file, PTY file

**Runtime directory**:
The private, per-user directory holding the sockets that clients use to reach session daemons, one per climon2 home.
_Avoid_: Socket dir, run dir, temp dir

## Installation

**Launcher**:
The `climon2` executable on PATH that hands off to the current version's CLI.
_Avoid_: Shim, wrapper, stub

**Current version**:
The installed version named by the `current` file in the install layout.
_Avoid_: Active version, default version

**Install layout**:
The packaged directory structure containing the launcher, `current`, and one folder per version.
_Avoid_: Install directory, bundle

**Climon2 executable home**:
The directory holding the install layout: the launcher, `current`, and the version folders.
_Avoid_: Install directory, exe dir, `CLIMON2_EXE`

**Climon2 home**:
The directory where climon2 keeps its data; `~/.climon2` unless `CLIMON2_HOME` overrides it.
_Avoid_: Data dir, state dir, config dir

**Installer**:
The program shipped in a release archive that places that release's version into a climon2 executable home and makes it the current version.
_Avoid_: Setup, install script, `climon2 install`

**Climon2 error code**:
The single exit code climon2 uses for its own failures, chosen by convention to be unlikely to be confused with the command's exit codes.
_Avoid_: Internal error code, failure code

## Testing

**Generic terminal fixture**:
A climon2-owned test program that checks the session contract (args, cwd, environment, exit status, resize) without depending on any shell.
_Avoid_: Test shell, dummy command

**TUI fixture**:
A climon2-owned full-screen test program that exercises alternate screen, cursor queries, mouse, bracketed paste, and resize redraws.
_Avoid_: Fake app, mock TUI

**Smoke test**:
The single end-to-end check that a freshly installed climon2 can start a bash session from bash, be listed, exit with the command's status, and leave nothing behind.
_Avoid_: Sanity test, canary

**Release gate**:
A check that must pass before a version can be released.
_Avoid_: Quality gate, CI check
