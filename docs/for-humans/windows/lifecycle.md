# Windows platform and lifecycle contract (agreed, partly implemented)

This document says how climon2's processes start, run and end on Windows: the
daemon, the session's command, the hand-offs between climon2's own programs,
the client's console, and how a session is identified and cleaned up. It is
for contributors. Built so far: the launcher's hand-off (see [Hand-offs](#hand-offs);
[#315](https://github.com/jackgeek/climon2/issues/315)), the daemon's
identity and liveness check (see [Identity and liveness](#identity-and-liveness);
[#316](https://github.com/jackgeek/climon2/issues/316)), and starting the
daemon, its session job and its command for `exec --headless` (see
[Starting the daemon](#starting-the-daemon) and
[Starting the command](#starting-the-command);
[#317](https://github.com/jackgeek/climon2/issues/317)), and `c2 kill`, which
hangs the session up through the client and its library (see
[Session job and hang-up](#session-job-and-hang-up);
[#318](https://github.com/jackgeek/climon2/issues/318)), and the CLI's hand-offs
to the client for `exec`, `attach` and `kill` and the client's console (see
[Hand-offs](#hand-offs) and [Client console](#client-console);
[#321](https://github.com/jackgeek/climon2/issues/321)), and command
resolution, quoting and the launch failure statuses (see
[Command resolution and arguments](#command-resolution-and-arguments);
[#322](https://github.com/jackgeek/climon2/issues/322)), and closing the
host, the leave policy and the statuses clients exit with after a hang-up (see
[Session job and hang-up](#session-job-and-hang-up);
[#323](https://github.com/jackgeek/climon2/issues/323)). Ctrl+C reaches the
session as byte `0x03`; Ctrl+Break is not passed on yet
([#326](https://github.com/jackgeek/climon2/issues/326)). It was
agreed in
[Define Windows platform support and lifecycle contract](https://github.com/jackgeek/climon2/issues/212),
using the VM measurements from
[Probe Windows lifecycle facts on a Windows 11 VM](https://github.com/jackgeek/climon2/issues/300),
and is the specification the Windows port implements.

Neighbouring decisions live elsewhere:

- I/O, transport, the runtime directory and the bundled ConPTY:
  [ADR 0003](../../adr/0003-platform-and-shell-seams.md#windows-io-and-transport-agreed-not-implemented).
- Which hosts, shells and scenarios qualify a release:
  [Windows host and shell qualification](qualification.md).
- The monitor and the client library:
  [Define Windows monitor library and specification integration](https://github.com/jackgeek/climon2/issues/216).
- Packaging, the installer and the launcher's `current` retry:
  [Define Windows packaging and upgrade contract](https://github.com/jackgeek/climon2/issues/215).

Linux and macOS behaviour does not change.

## Terms

- A **console** is the Windows object a command-line program reads keys from
  and writes text to. Processes that share one console all receive its
  **console control events**: `CTRL_C_EVENT`, `CTRL_BREAK_EVENT`, and
  `CTRL_CLOSE_EVENT` when the console goes away (logoff and shutdown have
  their own events). A process reacts to them through a **handler routine**
  registered with `SetConsoleCtrlHandler`; one that returns TRUE has handled
  the event, and the default handler ends the process.
- A **pseudoconsole** (ConPTY) is a console whose screen and keyboard are
  pipes, which is how the daemon hosts a session. It is Windows' counterpart
  of a [pseudo-terminal](../protocol/TERMS.md#pseudo-terminal-pty).
- A **job object** groups processes. A process started by a member joins the
  same job automatically, and `TerminateJobObject` ends every member at once.
  **Breakaway** is starting a child outside its parent's job, which the job
  must allow (`BREAKAWAY_OK`).

## Platform

x86_64 Windows 11, build 26100 or newer. The minimum is documented, not
checked at run time. CI qualifies on `windows-2025`, which shares build
26100's kernel and console stack. Windows 10 is not supported.

## Starting the daemon

`exec` starts the daemon with one raw `CreateProcessW`:

- `DETACHED_PROCESS`, plus `CREATE_BREAKAWAY_FROM_JOB` when the CLI is itself
  in a job. Never `CREATE_NEW_PROCESS_GROUP`, which would make the command
  ignore Ctrl+C, and never `CREATE_NEW_CONSOLE`, which Windows 11 can hand to
  Windows Terminal as a new window.
- Standard handles are null, which Rust treats as `NUL`: only the readiness
  pipe may be inherited, so there is no `NUL` handle to pass. The working
  directory is the daemon's own version folder. The environment is the
  session environment `exec` builds, as on Unix: the CLI's own, with the
  session's variables set and the client startup envelope dropped.
- Readiness comes back on an inheritable pipe, limited to the child by a
  one-entry `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, whose value is passed as an
  argument.
- The daemon starts suspended (`CREATE_SUSPENDED`) and runs only after the
  job check below, so a refused daemon never does anything.

A detached session must survive its host terminal closing. Breakaway only
leaves the innermost job, and an outer job can still hold the daemon, so the
CLI checks afterwards. If the spawn fails while the CLI is in a job, or
`IsProcessInJob(daemon, NULL)` is true after it, the CLI ends the daemon and
`exec` exits 125. There is no fallback route. This happens in a job that
refuses breakaway and under PowerShell's `Start-Process -Wait`. The probes
found no job around shells in Windows Terminal, VS Code terminals or VS Code
tasks.

## Starting the command

The daemon starts the command with one `CreateProcessW`, with no helper
process:

- Its attribute list holds `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` and
  `PROC_THREAD_ATTRIBUTE_JOB_LIST` (the session job).
- Handle inheritance is off and there is no handle list. Windows rejects an
  empty list (error 24), and with inheritance off nothing leaks.
- `STARTF_USESTDHANDLES` with null standard handles, so the command uses the
  pseudoconsole. Without it, the command takes the daemon's `NUL` handles,
  reads end-of-file and exits at once.
- Before the call, the daemon sets `TERM=xterm-256color`,
  `COLORTERM=truecolor` and `CLIMON2_SESSION_ID` in its own environment and
  calls `SetConsoleCtrlHandler(NULL, FALSE)`, so an inherited "ignore Ctrl+C"
  cannot reach the command. The command inherits that environment and gets
  the session's working directory.
- The first command-line token is the command as typed; the application name
  is the resolved absolute path (see
  [Command resolution and arguments](#command-resolution-and-arguments)). The
  program sees `argv[0]` as typed, as on Unix. A batch file's program is
  `cmd.exe`, which is given the script's resolved path.

A working directory over 260 characters cannot be used: `CreateProcessW` fails
with error 267 under every long-path setting, as for a missing directory. The
daemon reports it as Unix reports an unusable working directory, and `exec`
exits 126 with "command cannot be executed". There is no separate pre-check.
Windows cannot start `c2` itself in such a directory either, so the scenario
gives the CLI that working directory through its `CLIMON2_CLI_CWD` test hook,
which only test builds contain.

## Session job and hang-up

Each session has one job object with `JOB_OBJECT_LIMIT_BREAKAWAY_OK` and
`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, never `SILENT_BREAKAWAY_OK`. Breakaway is
allowed so a nested `c2 exec` can start its own daemon outside the session.

The daemon learns that the command ended by waiting on the leader's process
handle. It never polls the job for emptiness.

Hang-up, whether from the [leave policy](../../../GLOSSARY.md) or `c2 kill`:

1. `ConptyClosePseudoConsole`. Every process on the session's console gets
   `CTRL_CLOSE_EVENT`. In the probes every shell and app exited 5–30 ms later.
2. A full five-second grace, never shortened.
3. `TerminateJobObject(job, 0xC000013A)` for anything left in the job.
4. The leader's status, as it stands, is the session's status.

The probes recorded the leader's status after a close as `0xC000013A` for
`pwsh`, `powershell`, `cmd.exe` and Edit, and `0x1` for vim. Windows' own
five-second kill of console processes after `CTRL_CLOSE_EVENT` races the
grace; either way the processes are gone after about five seconds, and
scenarios assert outcomes, never which mechanism won.

When the host closes while a client is attached, the client, the CLI and the
launcher end with 129 (see [Hand-offs](#hand-offs)). The daemon is not on the
host's console, so it only loses the client, and the
[leave policy](../../../GLOSSARY.md) applies as on Unix: an ordinary session
whose last client is lost hangs up and ends, a headless one keeps running, and
a session already detached is untouched. An ordinary session that nobody
attaches to within five seconds of its start ends the same way. After
`c2 kill`, every attached client exits with the leader's status at its full
32 bits, which the CLI and the launcher pass on unchanged.

The Store build of `pwsh` is in the session job, but the processes it starts
are in no job. Those on the session's console still end at step 1. A program
it starts detached from the console (hidden, in its own window, or a GUI app)
can outlive the session. This is accepted rather than hunting descendants by
parent PID, which Windows reuses. Unix has a similar gap for programs that
detach with `setsid`.

## Hand-offs

Windows cannot replace a running process, so each Unix `exec` hand-off becomes
spawn and wait:

| Parent | Child |
| --- | --- |
| `c2.exe` (the launcher) | `climon2-cli.exe` |
| `climon2-cli.exe` for `exec` and `attach` | `climon2-client.exe` |
| `climon2-cli.exe` for `monitor` | `climon2-monitor.exe` |

The daemon start above is the fourth, and its parent does not wait.

Each waiting parent:

- exits with the child's raw `u32` status, so full-width codes such as
  `0xC000013A` pass through unchanged. The launcher ends itself with
  `TerminateProcess`, not `std::process::exit`: `exit` sets the status only
  after unloading the process's libraries, and a close event arriving
  meanwhile replaced it with `0xC000013A` in about one run in sixteen;
- registers a handler routine that returns TRUE for `CTRL_C_EVENT` and
  `CTRL_BREAK_EVENT`, never `SetConsoleCtrlHandler(NULL, TRUE)`, which children
  would inherit;
- on close, logoff or shutdown, waits up to three seconds for its child, then
  exits with the child's status, or 129 if the wait ran out.

Hosts deliver close events in different orders. Bundled ConPTY tells the
newest process first; a Windows Terminal tab tells the oldest first, so the
launcher's wait can run out before its child is told. Both end with 129.

The launcher contract must be right in the first Windows release, because an
old launcher can stay installed indefinitely
([#215](https://github.com/jackgeek/climon2/issues/215)).

## Client console

Raw mode clears `ENABLE_PROCESSED_INPUT`, `ENABLE_LINE_INPUT` and
`ENABLE_ECHO_INPUT`, and sets `ENABLE_VIRTUAL_TERMINAL_INPUT`,
`ENABLE_WINDOW_INPUT` and `ENABLE_EXTENDED_FLAGS` without quick edit. The
client reads key-down records only: key-up records are ignored. Resize
arrives as `WINDOW_BUFFER_SIZE_EVENT`. Close, logoff and shutdown end the
client with 129. The monitor uses the same console rules.

With VT input on, the console turns the host's focus reports into the
typed bytes `CSI I` and `CSI O` once the program reading it turns focus
reporting on (`CSI ? 1004 h`), and drops them until then. It makes no focus
records in this mode; they come only with VT input off. So a session that
turns focus reporting on, and the monitor, which always does, read the
host's focus reports as on Unix, under both host models
([#325](https://github.com/jackgeek/climon2/issues/325)).

The session's pseudoconsole starts its output with its own negotiation with
a host: `ESC [ 1 t`, `ESC [ c`, `CSI ? 1004 h` and `CSI ? 9001 h`. The daemon
is no host, so it drops these from the start of the output, and neither its
clients nor the screen it restores for them ever see them. From the
command's first output on, everything passes unchanged. The pseudoconsole
takes the first device attributes answer it reads as its host's, so the
daemon writes one (`ESC [ ? 6 c`, the answer it gives commands) before any
input. Otherwise the pseudoconsole would take the answer to the command's
own first request, and a program that waits for it, such as Microsoft Edit,
would wait forever ([#328](https://github.com/jackgeek/climon2/issues/328)).

Output adds `ENABLE_VIRTUAL_TERMINAL_PROCESSING` and
`DISABLE_NEWLINE_AUTO_RETURN`, and both code pages are UTF-8 while the client
runs. Every exit path, a panic included, restores both modes and both code
pages. A Ctrl+C that reaches the client as `CTRL_C_EVENT`, as it does in kill
mode, where the console is not raw, ends the client with 130, as SIGINT does
on Unix; the CLI waiting for it then exits 130 too.

The client ends itself with `TerminateProcess` and its full 32-bit status,
for the reason the launcher does (see [Hand-offs](#hand-offs)).

## Ctrl+C and Ctrl+Break

Ctrl+C arrives in the client as byte `0x03` and is forwarded as input, as on
Unix; the pseudoconsole turns it into `CTRL_C_EVENT` for the session.

Ctrl+Break has no VT byte. The client's handler routine receives
`CTRL_BREAK_EVENT` and asks the daemon to break. The daemon runs a short-lived
detached helper that calls `AttachConsole(leader PID)`, registers a
TRUE-returning handler, calls `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0)`
and exits. In the probes a default-handler program ended with `0xC000013A` and
the shell survived, with the helper inside or outside the session job.
Windows Terminal also sends a key-up record carrying `0x03` with Ctrl+Break;
ignoring key-up records keeps it from arriving as Ctrl+C.

Injecting a win32-input-mode Ctrl+Break record into ConPTY input was set
aside, because it may hold back a later lone Esc.

The VS Code terminal delivers nothing for Ctrl+Break, neither an event nor a
record, so Ctrl+Break does not work there.

## Identity and liveness

A daemon is identified by its PID plus its creation time from
`GetProcessTimes`, as epoch microseconds in the frozen `daemon_start_time`
([ADR 0005](../../adr/0005-frozen-session-metadata-core.md)). The probes showed
creation time unchanged across clock steps.

Liveness opens the process with
`PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE` and waits on it with a zero
timeout. Only "no such process" (`ERROR_INVALID_PARAMETER`) means the daemon
is gone; any other error keeps the session. A wait that succeeds at once
means the process has exited while something still holds a handle to it, as
a Unix zombie awaits its parent, so that daemon is gone too.

## Command resolution and arguments

climon2 resolves the command itself: each `PATH` directory in order, and in
each the extensions of `PATHEXT` in order. The current directory is not
searched, and neither is a relative `PATH` entry, such as `.` or an empty one,
which would name it. Rust std is not used for this: it ignores `PATHEXT` and
searches its own folder and the system folders first. The daemon resolves
with the session's `PATH` and `PATHEXT`, which it inherits from `exec`.

- A name that has an extension, such as `node.exe`, is tried as typed before
  the `PATHEXT` extensions are added. A name without one is never tried bare,
  so `npm` finds `npm.cmd`, not the extensionless shell script beside it.
- Quotes around a `PATH` entry are removed, as `cmd.exe` removes them.
- Without `PATHEXT`, or with an empty one, the extensions are `cmd.exe`'s
  own: `.COM;.EXE;.BAT;.CMD`.
- A command naming a folder or drive (holding `\`, `/` or `:`) is not looked
  up on `PATH`; it is found from the session's working directory, with the
  same extensions.
- Only a file counts; a folder with a matching name is passed over.

A command not found exits 127, as on Unix. A file that is found but cannot be
started, such as one that is not a program, exits 126 with "command cannot be
executed".

Arguments are quoted by the MSVC C runtime rules. `.bat` and `.cmd` commands
run through `cmd.exe /e:ON /v:OFF /d /c "…"`, with `cmd.exe` taken from the
system folder and the resolved script path in quotes, using Rust std 1.96.0's
batch escaping: an argument holding anything but letters, digits and
``#$*+-./:?@\_``, or ending in `\`, or empty, is quoted; inside it a quote
doubles, and `%` becomes `%%cd:~,%`, which `cmd.exe` expands back to `%`, so
no variable is expanded. std also refuses a script path containing `"` or
ending in `\`; a resolved file's path can do neither. An argument that
`cmd.exe` cannot pass unchanged (one holding CR, LF or NUL) is a usage error:
`exec` resolves the command as the daemon will and exits 2, without touching
the climon2 home or starting the daemon. Should the daemon still meet one, it
reports the command as one that cannot be executed (126), because the
readiness report carries only 125, 126 and 127.

## Self-attach and nested sessions

The daemon sets `CLIMON2_SESSION_ID` for the command, as on Unix, so the
existing refusal of a session attaching to itself works unchanged. Nested
sessions work through breakaway from the session job.

## Sign-out and shutdown

Both end every process the user owns, so sessions end with them. No exit
status is promised. Stale metadata is cleaned up by the next `list` through
the identity check.

## Exceptions to the Unix contract

| # | Exception | Evidence |
| --- | --- | --- |
| 1 | Exit statuses are full-width `u32`, passed through every hand-off unchanged. climon2's own 2, 125, 126, 127 and 129 are unchanged. | Automated: `0xC000013A`, `259` and `0xFFFFFFFF` round-trip through every hand-off |
| 2 | There are no signal-derived statuses. Hang-up and kill report the command's close status (`0xC000013A`, or the program's own, such as vim's `0x1`); a process the session job ends reports `0xC000013A`. | Automated: the close-status scenarios in [Windows host and shell qualification](qualification.md#windows-only-scenarios) |
| 3 | `exec` exits 125 when the daemon cannot leave the caller's job: a job that refuses breakaway, or `Start-Process -Wait`. | Automated: the three job-shape scenarios |
| 4 | Under Store `pwsh`, programs started detached from the session's console can outlive the session. | Manual: an observation recorded with the checklist |
| 5 | A working directory over 260 characters makes `exec` exit 126. | Automated scenario |
| 6 | Ctrl+Break reaches the session in Windows Terminal through the attach helper; in VS Code it does nothing. | Automated: the Windows Terminal model scenario. The VS Code gap is documented only |
| 7 | Sessions end at sign-out and shutdown, with no status promised. | Documented only; it cannot be tested without ending the run |
| 8 | Windows 11 build 26100 or newer, documented and not checked at run time. | CI runs `windows-2025` |

Self-attach refusal, nested sessions, 127 for a missing command, the
five-second grace and the leave policy keep their Unix behaviour.

## Limitations the README will publish

When Windows ships, the README's limitations gain:

- exceptions 4 to 7 above;
- that Windows Defender SmartScreen or Smart App Control may block the
  unsigned binaries, and that a browser-downloaded zip needs `Unblock-File`
  ([#215](https://github.com/jackgeek/climon2/issues/215));
- what the bundled ConPTY rewrites, drops or cannot express. That wording is
  written from observed behaviour when the Windows I/O is implemented.
