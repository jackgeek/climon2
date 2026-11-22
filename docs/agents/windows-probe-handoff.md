# Windows lifecycle probes: handoff for a session on a Windows 11 VM

## Start here

You are running inside a Windows 11 VM. Your job is to **measure** a set of
Windows behaviours and record what you see. You decide no policy, implement no
product code and resolve no decision ticket.

The measurements feed
[Define Windows platform support and lifecycle contract (#212)](https://github.com/jackgeek/climon2/issues/212),
a decision ticket on the
[Windows support map (#205)](https://github.com/jackgeek/climon2/issues/205).
Your own ticket is
[Probe Windows lifecycle facts on a Windows 11 VM (#300)](https://github.com/jackgeek/climon2/issues/300).
Assign it to the developer before any work (`gh issue edit 300 --add-assignee @me`).

Read `AGENTS.md` first. The originating session ran on Linux and has never
executed anything on Windows: every "expected" below comes from reading source
and documentation, and may be wrong. A result that contradicts an expectation is
the most valuable thing you can report. Do not adjust a probe to make it agree.

## Background you need

climon2 is a terminal session manager. `c2 exec <command>` (the CLI) starts a
detached **session daemon**, which hosts the command in a pseudoconsole and
outlives the terminal it was started from. A **client** in the user's terminal
attaches to the daemon. On Windows none of this exists yet; the design below is
agreed on paper.

Prior research, each on a throwaway branch (read the file, don't redo it):

| Branch | File | Covers |
| --- | --- | --- |
| `research/windows-lifecycle` | `docs/research/windows-lifecycle.md` | jobs, hang-up, identity, PATH and quoting |
| `research/windows-breakaway` | `docs/research/windows-breakaway.md` | host jobs, cargo's job, escape routes |
| `research/windows-ctrl-events` | `docs/research/windows-ctrl-events.md` | Ctrl+C and Ctrl+Break through ConPTY |

`docs/adr/0003-platform-and-shell-seams.md` on `main` ("Windows I/O and
transport") records the already-merged I/O decisions.

## Decisions the probes test

The developer accepted these in live grilling on 2026-11-19 and 2026-11-20.
They are **not yet recorded anywhere else**: #212 is still open, and its final
round (the exceptions list, evidence and where to record) awaits these results.
Treat them as the design under test.

1. **Platform.** x86_64 Windows 11 build 26100 or later, documented, not
   enforced at runtime.
2. **Daemon spawn.** A raw `CreateProcessW` with `DETACHED_PROCESS`, plus
   `CREATE_BREAKAWAY_FROM_JOB` when the CLI is in a job. No
   `CREATE_NEW_PROCESS_GROUP` (it would make the command ignore Ctrl+C) and no
   `CREATE_NEW_CONSOLE`. Standard handles are NUL. The daemon's cwd is its own
   version folder. Its environment is inherited (NULL block).
3. **Breakaway.** If the spawn fails while the CLI is in a job, or
   `IsProcessInJob(daemon, NULL)` is true afterwards, the CLI kills the daemon
   and exits 125. No fallback route (no WMI, no reparenting). This reopens if a
   qualified host holds shells in a job that forbids breakaway.
4. **Readiness.** An inheritable pipe, restricted by
   `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, its value passed as an argument.
5. **Command spawn.** One `CreateProcessW` whose attribute list holds
   `PSEUDOCONSOLE`, `JOB_LIST` (the session job) and an empty `HANDLE_LIST`.
   Before it, the daemon sets `TERM` and `COLORTERM` in its own environment and
   calls `SetConsoleCtrlHandler(NULL, FALSE)`; the command inherits the
   environment through a NULL block and gets the session cwd as
   `lpCurrentDirectory`.
6. **Session job.** `BREAKAWAY_OK` and `KILL_ON_JOB_CLOSE`; never
   `SILENT_BREAKAWAY_OK`.
7. **ConPTY.** The bundled `conpty.dll` + `OpenConsole.exe` from the
   `Microsoft.Windows.Console.ConPTY` NuGet package, never the inbox engine.
   The daemon calls `ConptyReleasePseudoConsole` right after starting the
   command.
8. **Hang-up.** `ConptyClosePseudoConsole`, a full five-second grace that is
   never shortened, then `TerminateJobObject(job, 0xC000013A)`, then the
   leader's status as it stands. The daemon learns the command ended by waiting
   on the leader's process handle; the job is never polled for emptiness.
9. **Hand-offs.** The launcher spawns the CLI and the CLI spawns the client;
   each waits and exits with the child's raw `u32`. Each waiter registers a
   handler routine returning TRUE for `CTRL_C_EVENT` and `CTRL_BREAK_EVENT`.
   On close, logoff or shutdown it waits for its child with a bound shorter
   than five seconds, then exits with the child's code, or 129 if the bound
   expires.
10. **Client.** Raw mode clears `ENABLE_PROCESSED_INPUT`, `LINE` and `ECHO`,
    and sets `ENABLE_VIRTUAL_TERMINAL_INPUT`, `ENABLE_WINDOW_INPUT` and
    `ENABLE_EXTENDED_FLAGS` without quick edit. Close, logoff and shutdown end
    it with 129. Ctrl+C arrives as byte `0x03` and is forwarded as input.
11. **Ctrl+Break.** The client's handler asks the daemon to break. The daemon
    runs a short-lived detached helper that calls `AttachConsole(leader pid)`,
    registers a TRUE-returning handler, calls
    `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0)` and exits. The alternative,
    injecting `ESC[3;70;0;1;8;1_` into ConPTY input, was set aside because it
    may leave a lone `Esc` held back.
12. **Identity.** PID plus `GetProcessTimes` creation time as epoch
    microseconds. Liveness is `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION |
    SYNCHRONIZE)` and a zero-timeout wait. Only "no such process" means gone;
    any other error keeps the session.
13. **Command resolution.** PATH × PATHEXT by climon2's own resolver, no
    current-directory search. `.bat`/`.cmd` run through
    `cmd.exe /e:ON /v:OFF /d /c "…"` with Rust std 1.96.0's batch escaping.

## Environment

Record all of this at the top of the results file before running anything:

- `winver` build and edition (`[System.Environment]::OSVersion`, and
  `Get-ComputerInfo | select OsName, OsBuildNumber, OsHardwareAbstractionLayer`).
  It must be a Windows 11 **client**, build 26100 or later. If it is a Server
  image or older, say so and continue; the host-job probes lose most of their
  value.
- Windows Terminal version and whether it is the Store/MSIX package
  (`Get-AppxPackage Microsoft.WindowsTerminal`).
- VS Code version (`code --version`).
- `rustc -Vv`, and whether the session is elevated.
- The ConPTY package version you downloaded and the SHA-256 of `conpty.dll`
  and `OpenConsole.exe` (x64). Use the latest stable
  `Microsoft.Windows.Console.ConPTY` from nuget.org.
- Whether you have an interactive desktop, and whether you can send real key
  presses to a window (needed for H3).

If something is missing (no Rust, no Windows Terminal), install it if you can
and record that you did. If you cannot, record the probes it blocks and run the
rest.

## How to work

- Work in a Git worktree at `..\climon2.worktrees\windows-probes`, on a new
  branch `research/windows-probes` from `origin/main`. Push the branch. Open no
  PR; it is never merged.
- Write the probes as one small Rust crate **outside the cargo workspace**
  (for example `probes/windows-lifecycle/`, with its own `Cargo.toml` and an
  empty `[workspace]` table), using `windows-sys`. It is throwaway measuring
  code: the repo's quality gates do not apply to it, and it must not touch any
  product crate.
- **Never run a probe executable through `cargo run` or `cargo test`.** cargo
  puts its children in a job that forbids breakaway, which changes several
  results. Build with `cargo build --release`, then run the `.exe` directly.
  Probe J3 is the one deliberate exception.
- Give the probe program subcommands, one per probe, each printing one JSON
  object per line to stdout. Keep raw values: `GetLastError` numbers, exit
  codes in hex, flag masks in hex, timings in milliseconds.
- Run each probe at least three times. Report variation instead of averaging
  it away; several of these are races.
- Probes that step the clock or close windows are disruptive. Run them last,
  and restore the clock.

## Probes

Each probe lists what to do, what to record, and the expectation it tests.
"Self-report" means the probe prints `IsProcessInJob(self, NULL)`, the
immediate job's `LimitFlags` from
`QueryInformationJobObject(NULL, JobObjectExtendedLimitInformation)`, the
session ID, token integrity level, parent PID and
`GetCurrentPackageFullName`.

### Host jobs (the highest-value group)

**H1 – Do the hosts hold shells in a job?**
Run the self-report from each context below, and in each also run the spawn
test: create a child with `DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB` (omit
the breakaway flag when not in a job), and have the parent print the
`CreateProcessW` result, `GetLastError`, and `IsProcessInJob(child, NULL)`.

- Windows Terminal: `pwsh`, `powershell` and `cmd` profiles.
- VS Code integrated terminal: the same three shells.
- VS Code task (`tasks.json` shell task) and an npm script.
- Classic conhost (`conhost.exe cmd.exe`), for comparison only.
- A shell started over SSH or however you are connected, for comparison.

Launch these without a human where possible, writing output to a file, for
example `wt.exe -- cmd /c "probe.exe self-report > C:\probes\wt-cmd.json"` and
`code --wait` with a task. If a context needs a human, say so.

*Expected:* in no job, or in a job the child leaves, in every context.
*Decides:* decision 3. A host whose shell sits in a no-breakaway job reopens it.

**H2 – Does the detached child survive the host closing?**
From a Windows Terminal tab and from a VS Code terminal, start the spawn test's
child so it writes a heartbeat line to a file every second. Close the tab, then
close the whole terminal application. Record whether the heartbeat continues,
for 30 seconds each time.

*Expected:* it continues. *Decides:* the "survives its host closing" contract.

**H3 – What does each host deliver for Ctrl+Break and Ctrl+C?**
Run a client-shaped probe in each host: it sets the raw mode from decision 10,
registers a console handler that logs every event type with a timestamp, and
logs every record `ReadConsoleInputW` returns. Press Ctrl+C, Ctrl+Break, and
Esc, each a few times. This needs real key presses; if you can only synthesise
them (`SendInput`), say which method you used.

*Expected:* Ctrl+C is a key record yielding `0x03` and no signal; Ctrl+Break is
a `CTRL_BREAK_EVENT` and no key-down record. What VS Code sends for Ctrl+Break
is unknown.
*Decides:* decision 11 and whether VS Code has a documented limitation.

### Jobs and breakaway

**J1 – The error for a refused breakaway.**
Create a job without `BREAKAWAY_OK`, put a process in it, and from that process
spawn with `CREATE_BREAKAWAY_FROM_JOB`. Record the result and `GetLastError`.
*Expected:* failure, `ERROR_ACCESS_DENIED` (5).

**J2 – Capture by an outer job.**
Nest two jobs: outer without `BREAKAWAY_OK`, inner with it. From a process in
the inner job, spawn with the breakaway flag. Record whether the spawn succeeds
and `IsProcessInJob(child, NULL)`.
*Expected:* the spawn succeeds and the child is still in a job. This is the
case the post-check exists for.

**J3 – cargo's job.**
Run the self-report and spawn test from inside a `cargo test` integration test
and from `cargo run`.
*Expected:* in a job with `KILL_ON_JOB_CLOSE` and no `BREAKAWAY_OK`; breakaway
refused.

**J4 – Nested climon2.**
A process in no job creates a session-style job (`BREAKAWAY_OK |
KILL_ON_JOB_CLOSE`) and starts a child in it with
`PROC_THREAD_ATTRIBUTE_JOB_LIST`. That child spawns a grandchild with
`DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB`. Record
`IsProcessInJob(grandchild, NULL)`, then close the job handle and record which
processes die.
*Expected:* the grandchild is in no job and survives; the child dies.

### Command spawn and hang-up (use the bundled ConPTY)

Load `conpty.dll` by full path from the folder holding both files, and use its
`ConptyCreatePseudoConsole`, `ConptyReleasePseudoConsole`,
`ConptyResizePseudoConsole` and `ConptyClosePseudoConsole` exports. The host
process for these probes must itself be detached (start it with
`DETACHED_PROCESS`, output to a file), as the daemon will be. Drain the output
pipe on its own thread throughout.

**S1 – One-call spawn.**
Start `cmd.exe /D` with a single `CreateProcessW` carrying `PSEUDOCONSOLE`,
`JOB_LIST` and an empty `HANDLE_LIST`. Record success, that the process is in
the session job, and that it inherited no handle it should not have (compare
its handle count with a control started the same way after opening ten extra
inheritable handles in the parent).

**S2 – Release, then end of output.**
After S1, call `ConptyReleasePseudoConsole`, then have the command exit.
Record whether `ReadFile` on the output pipe returns `ERROR_BROKEN_PIPE`, and
how long after the exit. Repeat with a background child still attached to the
console (`start /b ping -t localhost` then `exit`) and record whether the pipe
breaks while that child lives.

**S3 – Close status per shell profile.**
For each of `pwsh -NoProfile`, `powershell -NoProfile`, `cmd /D`, `vim` and
`edit.exe` (install what is missing; record what you could not): start it as in
S1, release, wait two seconds, then call `ConptyClosePseudoConsole` and start a
timer. Do **not** terminate the job. Record for the leader and for a background
child (`ping -t localhost` started from the shell): the exit code in hex, and
the time from close to exit. Record whether `ConptyClosePseudoConsole` returned
promptly, and that close after release still delivers the close event.

Then repeat with the real sequence: close, sleep exactly 5000 ms,
`TerminateJobObject(job, 0xC000013A)`, and record each process's exit code.
Run this ten times per shell; it is a race against the OS's own five-second
kill.

*Expected:* the leader reports `0xC000013A` or the shell's own code; processes
are gone by about five seconds either way.
*Decides:* decision 8 and the set of statuses the contract may promise.

**S4 – A process with no console in the session job.**
In the session, start a detached child (`DETACHED_PROCESS`) that stays in the
job. Run the close-then-terminate sequence. Record that it gets no close event
and dies only at `TerminateJobObject`, and its exit code.

**S5 – Leader handle and job accounting.**
While holding the leader's process handle, let the leader exit with a
background child still running. Record that the handle signals, the leader's
exit code, and `JobObjectBasicAccountingInformation.ActiveProcesses` before and
after the background child exits. Informational only.

**S6 – DLL load failure.**
Start a program whose required DLL is missing. Record whether `CreateProcessW`
succeeds and the exit code.
*Expected:* success, then exit `0xC0000135`.

**S7 – Working directory length.**
Create a directory whose full path is longer than 260 characters and pass it as
`lpCurrentDirectory`. Record the result and `GetLastError`, with the system
long-path policy both off and on (`LongPathsEnabled`), and with and without a
`longPathAware` manifest in the probe. Also pass a nonexistent directory and a
file, and record those errors.

### Ctrl+C and Ctrl+Break through ConPTY

**C1 – Ctrl+C as a byte.**
In a session from S1 running `ping -t localhost`, write `0x03` to the input
pipe. Record that `ping` stops and its exit code. Repeat with a command that
has processed input off (a small raw-mode reader) and record that it reads
`0x03`.

**C2 – Inherited "ignore Ctrl+C".**
Repeat C1 three ways and record whether `ping` stops:
(a) the probe host started with `CREATE_NEW_PROCESS_GROUP`;
(b) the probe host's parent called `SetConsoleCtrlHandler(NULL, TRUE)` first;
(c) as (a) and (b), but the host calls `SetConsoleCtrlHandler(NULL, FALSE)`
before starting the command. Record that call's return value and
`GetLastError`, since the host has no console.
*Expected:* (a) and (b) break Ctrl+C; (c) repairs it. *Decides:* decision 2
and 5.

**C3 – Ctrl+Break by attach helper.**
With a shell running a child (`cmd /D` running `ping -t localhost`), start a
detached helper that does `AttachConsole(leader pid)`, registers a
TRUE-returning handler, calls `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0)`,
then `FreeConsole` and exits. Record the helper's results and exit code, which
processes received the event, and that the session is otherwise undisturbed
(type a command afterwards and check the output). Try the helper both inside
and outside the session job.
*Expected:* `ping` ends; the shell survives. *Decides:* decision 11.

**C4 – Ctrl+Break by injected record, and the Esc side effect.**
In a fresh session, write `ESC[3;70;0;1;8;1_` to the input pipe and record
whether the command gets `CTRL_BREAK_EVENT`. Then, in the same session, with a
raw-mode VT reader as the command:
- write a lone `0x1B` in its own `WriteFile` and record whether the reader gets
  it within 500 ms;
- then write `a` and record exactly what bytes the reader gets and when;
- write `ESC[1;5R` and record what the reader gets.
Run the same three writes in a session that never received the injected record,
as the control.
*Expected:* after injection the lone Esc is held until the next byte. If it is
**not** held, injection becomes a viable, simpler alternative to C3.

**C5 – `?9001` at start-up.**
Capture the first bytes ConPTY writes to its output pipe. Record whether it
sends `ESC[?9001h`, and that the session works when nothing answers it.

### Hand-offs and close events

**X1 – Close-event order.**
Build a three-deep chain sharing one console: A spawns B, B spawns C, each
registering a handler that logs the event and a timestamp to its own file. A
and B wait up to 3 s for their child inside the handler, then exit with the
child's code, or 129; C exits 129 at once. Host the chain in a bundled-ConPTY
pseudoconsole and close it; also run it in a real Windows Terminal tab and
close the tab. Record the order and timing of the events and each process's
final exit code as seen by its parent or the host.
*Expected:* unknown. *Decides:* decision 9, whether the launcher can report the
client's 129.

**X2 – Full-width exit codes.**
A child exits with `0xC000013A`, `259` and `0xFFFFFFFF`; the parent waits on
the handle, then exits with `std::process::exit(code as i32)`; a grandparent
records what it sees.
*Expected:* all three round-trip.

**X3 – Ctrl+C with handler routines.**
In a real console (not raw mode), run the A→B→C chain where A and B return TRUE
for `CTRL_C_EVENT` and C uses the default handler. Send Ctrl+C. Record that C
dies and A and B survive to report its code.

### Identity (run last; needs elevation)

**I1 – Creation time across a clock step.**
Read a live process's creation time with `GetProcessTimes`. Step the clock
forward one hour with `SetSystemTime`, read again, step back two hours, read
again, then restore the clock (and re-sync with `w32tm /resync`). Record all
values raw. Stop any time-sync service that fights you, and restart it after.
*Expected:* unchanged.

**I2 – Liveness errors.**
Record `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, …)`
results and `GetLastError` for: a PID that never existed; a PID whose process
exited and was fully released; a process that exited while you hold another
handle to it (then `WaitForSingleObject(h, 0)` and `GetProcessTimes`); an
elevated process of the same user opened from a non-elevated probe; a process
of another user; PID 0 and PID 4.
*Expected:* "no such process" is `ERROR_INVALID_PARAMETER` (87); elevated
same-user is openable with these rights.

**I3 – Creation-time resolution.**
Start 1000 short processes in a tight loop and record the smallest non-zero
difference between creation times, and whether any two share a value.

### Command line (lower priority)

**Q1 – Batch arguments.**
Build the command line as Rust std 1.96.0 does for batch files (run a `.cmd`
that echoes `%*` and each of `%1`…`%9` through `std::process::Command`, which
applies that escaping) and round-trip arguments containing each of
`% ^ & | ! " < > ( )`, a trailing `\`, spaces, and an empty argument. Record
what the script received. Confirm std refuses CR, LF and NUL.

**Q2 – argv[0] as typed.**
Call `CreateProcessW` with `lpApplicationName` set to an absolute path and a
command line whose first token is a bare name. Record what the child sees as
`argv[0]` and from `GetModuleFileNameW`.

## What cannot be measured here

Logoff and shutdown end your own session. Do not attempt them. Note in the
results that they remain a manual check.

## Recording results

1. Write `docs/research/windows-probes.md` on `research/windows-probes`:
   the environment block, then one section per probe ID with what you ran, the
   raw observations, and one line stating whether the expectation held. Mark
   anything not run, with the reason. Commit the probe crate beside it and push.
2. Lead the file with a short "Surprises" list: every result that contradicts
   an expectation or a decision above.
3. Post a resolution comment on #300 linking the commit, repeating the
   "Surprises" list and naming the probes not run. Close #300 only if every
   probe has a result or a recorded reason.
4. Do **not** close or comment a resolution on #212, edit the map (#205), or
   change any ADR. The session that owns #212 reads your results and takes the
   affected decisions back to the developer.
