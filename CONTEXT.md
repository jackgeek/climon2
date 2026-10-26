# Climon2

Climon2 provides terminal sessions that clients can discover and attach to. The factory builds and validates climon2.

## Climon2 language

**Session daemon**:
The detached climon2 process that owns a session's terminal and makes the session available to clients.
Shipped as the `climon2-daemon` executable.
_Avoid_: Session server, server, slave, host

**Session**:
A running terminal environment containing a shell or another command, owned by a session daemon.
Its lifetime is independent of client connections.

**Client**:
A climon2 process that relays a user's terminal to and from one session.
_Avoid_: Host, slave, viewer

**CLI**:
The short-lived `climon2-cli` command that interprets user commands and launches session daemons and clients.
Each runtime version includes its own CLI.

**Launcher**:
The stable `climon2` command users invoke, which runs the CLI of the current runtime version.
_Avoid_: Shim, wrapper

**Current version**:
The runtime version whose CLI the launcher runs; new sessions start with it.

**Monitor**:
A future interactive terminal application for browsing sessions and mirroring a selected one through a client.
_Avoid_: Server, dashboard

**Controlling client**:
The single client currently entitled to send input to a session and display its terminal output.

**Displaced client**:
An attached client that no longer has control and displays a reclaim notice instead of session output.

**Reclaim**:
An immediate transfer of control from the controlling client to a displaced client.

**Runtime version**:
One installed, self-contained set of CLI, session daemon and client executables sharing a version.
A session's clients always use its session daemon's runtime version.

**Session metadata**:
The per-session discovery record, named by the session's GUID, that lets clients find a session daemon.
A metadata record alone does not make a session live.

**Climon2 home**:
The directory holding climon2's state, defaulting to `.climon2` in the user's home directory.

**Session depth**:
How many climon2 sessions enclose a session, counting itself; a top-level session has depth 1.
_Avoid_: Nesting level

## Factory language

**Factory**:
The system that builds and validates climon2 from approved feature contracts.
Climon2 is its output, not the factory itself.

**Feature**:
A single, independently deliverable aspect of climon2's behavior.
_Avoid_: Module (reserved for code structure)

**Feature contract**:
The user-approved description of a feature's required behavior and its acceptance scenarios.

**Acceptance tests**:
Executable black-box tests derived from a feature contract and approved by the user before implementation.

**Feature run**:
One pass of the factory delivering a single feature, from contract approval to pull request.

**Candidate**:
A specific commit proposed by the Implementer as satisfying a feature's acceptance tests.

**Platform gate**:
The requirement that a candidate pass all acceptance tests on Windows, Linux, and macOS.

**Negative control**:
A deliberately incorrect program used to prove that an acceptance test rejects the defect it targets.

**Terminal fixture**:
A harness-owned test program that reports its terminal environment so tests can verify real PTY/ConPTY behavior.

**Contract amendment**:
A user-approved, versioned change to an already-delivered feature's contract and acceptance tests.

**Roadmap**:
The user-owned ordered list of features and their dependencies.

**Contract Author**:
The agent role that drafts a feature contract from the user's prose and surfaces ambiguities as questions.

**Test Author**:
The agent role that turns an approved feature contract into acceptance tests.

**Implementer**:
The agent role that writes climon2 code to satisfy approved acceptance tests, without modifying them.

**Reviewer**:
The read-only agent role that adversarially critiques contracts and acceptance tests before approval, and candidates after implementation.

**Artifact**:
Any content-addressed output of a factory step, recorded together with the hashes of the artifacts it was derived from.

**Stale artifact**:
An artifact whose recorded input hashes no longer match the current upstream artifacts; it cannot be used to advance a feature run.

**Approval**:
A user's explicit acceptance of one specific artifact hash.
