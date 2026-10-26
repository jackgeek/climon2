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
The client whose input and terminal size drive the session.
_Avoid_: Primary client, owner

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

**Climon2 home**:
The directory where climon2 keeps its data; `~/.climon2` unless `CLIMON2_HOME` overrides it.
_Avoid_: Data dir, state dir, config dir

**Climon2 error code**:
The single exit code climon2 uses for its own failures, distinct from the command's exit codes.
_Avoid_: Internal error code, failure code
