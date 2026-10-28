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

**Attachment**:
One client's connection to a session for interactive use, whether or not that client currently controls it.
_Avoid_: Session, control

**Detach**:
End one client's attachment without ending the session or its command.
_Avoid_: Exit the session, kill, stop

**Client escape prefix**:
The key that begins a client-side action instead of being sent directly to the session's command.
_Avoid_: Hotkey, command prefix

**Take-control key**:
The configurable key a non-controlling client uses to request control of a session.
_Avoid_: Space-to-control, hotkey

**Controlling client**:
The attached client whose input and terminal size drive the session; only one client controls a session at a time.
When it leaves, control returns to the attached client that held control most recently, and the leave policy applies only if none remains.
_Avoid_: Primary client, owner

**Control period**:
One uninterrupted period in which a particular client controls a session; regaining control begins a new period.
_Avoid_: Attachment, client identity

**Monitor**:
A program that shows one session at a time in the user's terminal and switches between sessions, keeping every session's screen current through its own clients.
_Avoid_: Dashboard, multiplexer, switcher

**Virtual terminal**:
A model of a session's screen, kept current from the session's output; the session daemon's is the source of truth, and a monitor's is a copy.
_Avoid_: Screen buffer, emulator, scrollback

**Leave policy**:
What a session daemon does with its session when the last client leaves.
_Avoid_: Disconnect handler, kill policy

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

**Detached session**:
A live session with no attached clients that continues after deliberate detach.
_Avoid_: Disconnected session, completed session

**Stale metadata**:
Session metadata whose session is no longer live.
_Avoid_: Dead session, orphan

**Session metadata**:
The per-session record in climon2 home that describes a session and how to reach its daemon.
_Avoid_: Session file, PTY file

**Runtime directory**:
The private, per-user directory holding the sockets that clients use to reach session daemons, one per climon2 home.
_Avoid_: Socket dir, run dir, temp dir

**Session log**:
The diagnostic records climon2 keeps about its own processes in one session; never about the session's content.
_Avoid_: Transcript, history, session output

**General log**:
Diagnostic records from a climon2 process that are not tied to any session, such as `list` or `exec` before its session exists.
_Avoid_: Global log, main log

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
_Avoid_: Setup, `climon2 install`

**Release archive**:
The single downloadable file for one climon2 version, holding the installer, the launcher and that version's folder.
_Avoid_: Bundle, package, tarball

**Install script**:
The script published with each release that downloads, verifies and unpacks that release's archive and runs its installer.
_Avoid_: Installer, bootstrap, setup script

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

**Shell profile**:
The test harness's description of how to start one shell in a controlled way and drive it in tests; the only place climon2 knows about specific shells.
_Avoid_: Shell config, shell adapter

**Smoke test**:
The single end-to-end check that a freshly installed climon2 can start a bash session from bash, be listed, exit with the command's status, and leave nothing behind.
_Avoid_: Sanity test, canary

**Release gate**:
A check that must pass before a version can be released.
_Avoid_: Quality gate, CI check
