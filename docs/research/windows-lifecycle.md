# Windows process lifecycle and identity

Research for [Research Windows process lifecycle and identity (#207)](https://github.com/jackgeek/climon2/issues/207),
under [Chart Windows support specification (#205)](https://github.com/jackgeek/climon2/issues/205). It informs
[#212](https://github.com/jackgeek/climon2/issues/212). Checked **2026-11-14** against `main` `0b42d32`. This note
records facts and credible options only. It chooses no policy, approves no contract exception and implements nothing.

**Scope.** Windows 11 only (the developer's scope change of 2026-11-14 dropped Windows 10). Differences are
noted across Windows 11 builds and against GitHub's `windows-2022` and `windows-2025` runners.

**Tags.** **V** means verified in a cited primary source: Microsoft Learn (cited by its rendered URL and read from the
`MicrosoftDocs` source repositories), Rust std source at tag `1.96.0` (the repo toolchain), or `microsoft/terminal` at
`f6de9e0`. **I** means an inference from V facts. **U** means not established; each U names a CI probe.
**S** means a secondary source, named. Nothing was run on Windows. This host has no Windows, and CI is the only
Windows execution environment.

## Build landscape (read first)

| Target | OS build | Source |
| --- | --- | --- |
| Windows 11 23H2 | 22631 | End of updates for every edition (V, [release info](https://learn.microsoft.com/windows/release-health/windows11-release-information)) |
| Windows 11 24H2 / LTSC 2024 | 26100 | Serviced; GA Home/Pro end of updates, Enterprise to 2027-10-12 (V, same) |
| Windows 11 25H2, 26H1, 26H2 | 26200, 28000, 26300 | Serviced (V, same) |
| GitHub `windows-2022` | Server 2022, 10.0.20348 | V, [runner-images `Windows2022-Readme.md`](https://github.com/actions/runner-images/blob/95e6f51a068507e8f43d061d2166a39d0ef36ce8/images/windows/Windows2022-Readme.md) |
| GitHub `windows-2025` | Server 2025, 10.0.26100 | V, [runner-images `Windows2025-Readme.md`](https://github.com/actions/runner-images/blob/95e6f51a068507e8f43d061d2166a39d0ef36ce8/images/windows/Windows2025-Readme.md) |

**I:** every serviced Windows 11 build is 26100 or later. `windows-2025` has the same build number as 24H2, but
`windows-2022` (20348) is older than every Windows 11 release. Two lifecycle behaviours change at 26100 (§3), so
**`windows-2022` does not model the qualified Windows 11 baseline for hang-up**. `windows-2025` does, as far as
build number goes. A Server SKU is still not a client SKU (U: Server-only policy, no MSIX Windows Terminal by default).

## Material findings

1. **Breakaway is opt-in by the job that holds you, and an ancestor in a nested chain can capture you.**
   `CREATE_BREAKAWAY_FROM_JOB` works only if the immediate job sets `JOB_OBJECT_LIMIT_BREAKAWAY_OK`. With nesting,
   the child moves up the chain only until it reaches a job that forbids breakaway (§1). The daemon can check its own
   job's limits before spawning (`QueryInformationJobObject(NULL, …)`), which gives a clean, pre-emptive "125" path.
2. **There is an atomic job assignment at creation.** `PROC_THREAD_ATTRIBUTE_JOB_LIST` is supported on every
   Windows 11 build and on Server 2016 and later. Combined with `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` and
   `…_HANDLE_LIST`, one `CreateProcessW` call can create the command in its pseudoconsole and session job with exactly
   the handles it needs. The daemon-side exec helper then becomes unnecessary on Windows (§2, §4). Stable Rust std
   cannot pass attribute lists: `spawn_with_attributes` is unstable, so this needs a direct `CreateProcessW` (§4).
3. **Hang-up timing changed at build 26100.** From Windows 11 24H2, `ClosePseudoConsole` returns immediately.
   Earlier builds, including `windows-2022`, wait indefinitely unless the output pipe is drained or closed. Also from
   about 26100, CSRSS waits 5 s after `CTRL_CLOSE_EVENT` and force-kills each attached client. That is the same
   length as climon2's grace, so the two race (§3).
4. **The exit status under `CTRL_CLOSE_EVENT` is not documented by Microsoft.** A credible secondary source says the
   default handler exits with `STATUS_CONTROL_C_EXIT` (0xC000013A). The status of a process that returns TRUE, or that
   CSRSS force-kills, is U. Shells (cmd, pwsh, powershell) install their own handlers, so each needs a CI measurement (§3).
5. **PID + creation time is a sound identity, with one undocumented edge.** Creation time is an absolute UTC FILETIME
   (100 ns units, but quantised to the clock tick, ≤ 15.625 ms) that `GetProcessTimes` reports. Microsoft does not
   document that it survives a wall-clock step (U, but with no Linux-style boot-offset derivation). An open handle
   pins a PID against reuse (§5).
6. **Rust std does not implement PATH × PATHEXT.** It appends only `.exe`, searches the app directory and the
   system directories *before* the parent's PATH, and never the current directory. It already refuses (InvalidInput)
   `.bat`/`.cmd` arguments containing CR, LF or NUL, and script paths containing `"` or ending in `\`. It escapes `%` and
   quotes everything else (CVE-2024-24576, completed by CVE-2024-43402) (§6).
7. **A waiting parent must not ignore Ctrl+C with `SetConsoleCtrlHandler(NULL, TRUE)`.** That attribute is
   *inherited* by children, so the command would stop receiving Ctrl+C. A handler routine that returns TRUE is
   per-process (§4).

## 0. Current Unix contract that each mechanism replaces

- Daemon spawn: `setsid()` in `pre_exec`, plus a CLOEXEC-cleared readiness pipe fd passed as `--readiness-fd`.
  The pipe carries a framed `R` / `F code len msg` reply, codes 125–127, with a deadline
  (`crates/climon2-platform/src/daemon.rs:84-117,166-180`).
- Hang-up: observe, close master, sleep `HANGUP_GRACE` (5 s), SIGKILL the remaining session/job groups, then report
  the leader's status (`crates/climon2-platform/src/process.rs:283-309`).
- Three `exec` hand-offs: launcher → CLI (`crates/climon2-launcher/src/handoff.rs:44-55`); CLI → client
  (`crates/climon2-cli/src/exec.rs:231`, `attach.rs:23`); daemon PTY helper → command, where an exec failure writes
  126/127 back over a socket (`crates/climon2-daemon/src/pty.rs:360-398`). 127 is "NotFound in a usable cwd";
  every other failure is 126.
- Identity: `is_same_process(pid, start) = start_time(pid)? == Some(start)`, and errors are not evidence of death
  (`process.rs:55-58`). ADR 0005 freezes `daemon_start_time` as epoch microseconds.
- ADR 0003, "Notes for future ports": hang-up is ClosePseudoConsole → 5 s → TerminateProcess; the launcher spawns,
  waits, exits with the child's code and ignores Ctrl+C; the Exit frame carries a raw `u32`.

## 1. Spawn and detach

**Flags (V, [Process Creation Flags](https://learn.microsoft.com/windows/win32/procthread/process-creation-flags)).**
- `DETACHED_PROCESS` (0x8): a console child "does not inherit its parent's console". It cannot be combined with
  `CREATE_NEW_CONSOLE`.
- `CREATE_NEW_PROCESS_GROUP` (0x200): the child is the root of a new console process group, and "CTRL+C signals will
  be disabled for all processes within the new process group". `CreateProcess` performs an implicit
  `SetConsoleCtrlHandler(NULL,TRUE)` for it, and CTRL+BREAK is not disabled
  (V, [CreateProcessW remarks](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw)).
- `CREATE_NO_WINDOW` "is ignored … if it is used with either CREATE_NEW_CONSOLE or DETACHED_PROCESS".
- `CREATE_BREAKAWAY_FROM_JOB` (0x01000000): "If the calling process is not associated with a job, this constant has
  no effect. If the calling process is associated with a job, the job must set the JOB_OBJECT_LIMIT_BREAKAWAY_OK limit."
- Detached processes are not affected by console Ctrl+C/Ctrl+Break
  (V, [CTRL+C and CTRL+BREAK Signals](https://learn.microsoft.com/windows/console/ctrl-c-and-ctrl-break-signals)).
  Console `CTRL_CLOSE_EVENT` goes to processes *attached* to the console
  (V, [CTRL+CLOSE](https://learn.microsoft.com/windows/console/ctrl-close-signal)).
  **I:** a `DETACHED_PROCESS` daemon receives no close event when the host terminal closes. Only job membership can
  kill it.
- Rust std's internal `detach` uses exactly `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` ("stolen from the libuv
  code"), but it is not exposed. The public route is `CommandExt::creation_flags` (stable since 1.16)
  (V, [`sys/process/windows.rs` L320-L323](https://github.com/rust-lang/rust/blob/1.96.0/library/std/src/sys/process/windows.rs#L320-L323),
  [`os/windows/process.rs` L181](https://github.com/rust-lang/rust/blob/1.96.0/library/std/src/os/windows/process.rs#L181)).

**Breakaway semantics (V, [Job Objects](https://learn.microsoft.com/windows/win32/procthread/job-objects),
[Nested Jobs](https://learn.microsoft.com/windows/win32/procthread/nested-jobs),
[JOBOBJECT_BASIC_LIMIT_INFORMATION](https://learn.microsoft.com/windows/win32/api/winnt/ns-winnt-jobobject_basic_limit_information)).**
- By default, children of a job member join the job. With `BREAKAWAY_OK`, a child created with
  `CREATE_BREAKAWAY_FROM_JOB` does not. With `SILENT_BREAKAWAY_OK`, *every* child is outside the job, and no flag is
  needed.
- Nesting: "If the immediate job object allows breakaway, the child process breaks away from the immediate job object
  and from each job in the parent job chain, moving up the hierarchy until it reaches a job that does not permit
  breakaway. If the immediate job object does not allow breakaway, the child process does not break away even if jobs
  in its parent job chain allow it."
  **I:** if a host has job H (no breakaway) with a nested job beneath it that allows breakaway, the daemon can leave
  the inner job yet stay in H. Succeeding at breakaway therefore does not prove that the daemon is free of every job.
- "After a process is associated with a job, the association cannot be broken."
- Job membership survives `KILL_ON_JOB_CLOSE`: "closing the last job object handle terminates all associated
  processes". This is the mechanism that kills a non-broken-away daemon when a host exits.
- **How a forbidden breakaway fails (U).** Microsoft documents only the requirement, not the error.
  `ERROR_ACCESS_DENIED` from `CreateProcess` is widely reported but was not found in a primary source.
  *CI probe:* in a job without `BREAKAWAY_OK`, spawn with the flag and record `GetLastError`.
- **Pre-flight (V):** `QueryInformationJobObject(NULL, JobObjectExtendedLimitInformation, …)` reads "the job
  associated with the calling process … If the job is nested, the immediate job"
  ([QueryInformationJobObject](https://learn.microsoft.com/windows/win32/api/jobapi2/nf-jobapi2-queryinformationjobobject)).
  `IsProcessInJob(process, NULL, &r)` tests membership in *any* job
  ([IsProcessInJob](https://learn.microsoft.com/windows/win32/api/jobapi/nf-jobapi-isprocessinjob)).
  **I:** the immediate job's flags cannot reveal ancestors that forbid breakaway. After spawning, `IsProcessInJob`
  on the daemon handle is the decisive post-check.

**Options for the "breakaway or 125" contract (no choice made).**
- (a) Pre-check the immediate job's flags, then spawn with `CREATE_BREAKAWAY_FROM_JOB` when in a job, then post-check
  `IsProcessInJob(daemon, NULL)`. If still in a job, kill the daemon and fail with 125.
- (b) Spawn with the flag only and treat a `CreateProcess` failure as 125. This misses the nested-ancestor capture above.
- (c) As (a), but also accept "in a job without `KILL_ON_JOB_CLOSE`". The flags of ancestor jobs are not observable
  (U), so this cannot be proven.
- **Host behaviour (U).** Windows Terminal's `ConptyConnection` creates the shell with only
  `EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT` and no job
  (V, [`ConptyConnection.cpp` L166-L172](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/cascadia/TerminalConnection/ConptyConnection.cpp#L166-L172)).
  It is an MSIX-packaged app, and Desktop Bridge descendants have a separate policy,
  `PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY`
  (V, [UpdateProcThreadAttribute](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)),
  whose job effects on our process are U. VS Code (node-pty/ptyHost) and the GitHub Actions runner were not examined.
  *Harness probe:* in each host and on each runner, log `IsProcessInJob(NULL)` and the immediate job's `LimitFlags`
  from a shell.

**Readiness handle in place of `--readiness-fd` (options).**
- Inherited pipe: "inherited handles have the same value and access rights as the original handles"
  (V, CreateProcessW `bInheritHandles`), so the handle value can be passed as a decimal argument. A plain
  `bInheritHandles=TRUE` leaks *every* inheritable handle in the process. `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` restricts
  inheritance to a listed set, and the listed handles "must be created as inheritable"
  (V, UpdateProcThreadAttribute; CreateProcessW remarks on multi-threaded spawners).
- Rust std serialises its own spawns with a `CREATE_PROCESS_LOCK` because "once an inheritable handle is created,
  *any* spawned child will inherit that handle"
  (V, [`windows.rs` L328-L341](https://github.com/rust-lang/rust/blob/1.96.0/library/std/src/sys/process/windows.rs#L328-L341)).
  An inheritable readiness handle used with std `Command` would therefore leak into concurrent std spawns made outside
  that lock. std's `inherit_handles(false)` is unstable (issue 146407), and so is `spawn_with_attributes` (issue 114854)
  (V, [`os/windows/process.rs` L342-L381](https://github.com/rust-lang/rust/blob/1.96.0/library/std/src/os/windows/process.rs#L342-L381)).
- No inheritance: the CLI creates a per-launch AF_UNIX socket or named endpoint and passes its path. This is the
  pattern the daemon already uses for its exec confirmation socket (`pty.rs:300-325`).
- `DuplicateHandle` into a suspended child. The value still has to reach the child (I: an argument fixed at creation
  cannot carry it, so it would have to go through a later channel).

## 2. One job object per session

- **Atomic assignment.** `PROC_THREAD_ATTRIBUTE_JOB_LIST` takes "a list of job handles to be assigned to the child
  process, in the order specified". It is supported on Windows 10 and later and on Server 2016 and later, so on every
  Windows 11 build and both runners
  (V, [UpdateProcThreadAttribute](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)).
  The fallback is `CREATE_SUSPENDED`, then `AssignProcessToJobObject`, then `ResumeThread`
  (V, [AssignProcessToJobObject](https://learn.microsoft.com/windows/win32/api/jobapi2/nf-jobapi2-assignprocesstojobobject)
  says `JOB_OBJECT_SECURITY_ONLY_TOKEN` requires creation suspended). Assignment fails if "the job or any of its parent
  jobs … is terminating".
- **Termination.** `TerminateJobObject(hJob, uExitCode)` terminates every process in the job and its child jobs.
  `uExitCode` is "the exit code to be used by all processes and threads", and members cannot "postpone or handle the
  termination" (V, [TerminateJobObject](https://learn.microsoft.com/windows/win32/api/jobapi2/nf-jobapi2-terminatejobobject)).
  **I:** passing `0xC000013A` gives the map's "terminated after grace" status directly. A leader that already exited
  keeps its own code. `TerminateProcess` is asynchronous, so wait on the process handle to confirm it
  (V, [TerminateProcess](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-terminateprocess)).
- **Nested climon2.** For an inner `c2 exec` inside a session to keep its daemon running, the session job needs
  `JOB_OBJECT_LIMIT_BREAKAWAY_OK`, and the inner CLI must spawn with `CREATE_BREAKAWAY_FROM_JOB` (V, §1). **I:** this
  lets *any* member opt out the same way, so a deliberate breakaway by any program also escapes the "remaining jobs"
  sweep. `SILENT_BREAKAWAY_OK` would leave *all* descendants outside the job and must not be set. If the outer daemon
  itself sits in no job, the session job is top-level, and a breakaway from it lands in no job.
- **Do not set `KILL_ON_JOB_CLOSE` casually.** The daemon's last handle closing would kill the session
  (V, Job Objects). That is fail-safe if the daemon crashes, but it conflicts with any design where something other
  than the daemon must outlive it. No choice is made here.
- **Observing the job empty.**
  - Completion port `JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO`, but "delivery … is not guaranteed" except for
    notification limits. Associate the port while the job is inactive to avoid missing messages. Messages carrying a
    PID are only trustworthy while you hold a handle
    (V, [JOBOBJECT_ASSOCIATE_COMPLETION_PORT](https://learn.microsoft.com/windows/win32/api/winnt/ns-winnt-jobobject_associate_completion_port)).
  - Polling `JobObjectBasicAccountingInformation.ActiveProcesses`
    (V, [JOBOBJECT_BASIC_ACCOUNTING_INFORMATION](https://learn.microsoft.com/windows/win32/api/winnt/ns-winnt-jobobject_basic_accounting_information)).
    Its wording says the count is decremented "when the terminated process exits and all references to the process
    are released". Whether the daemon's own open handle to the leader keeps the count above zero is **U**.
    *CI probe:* hold the leader handle, terminate it and read `ActiveProcesses`.
  - `JobObjectBasicProcessIdList` enumerates members (V, QueryInformationJobObject).
  - The job object is signalled only for end-of-job time limits, not when it empties (V, Job Objects, "Managing Job
    Objects"), so waiting on the job handle cannot detect emptiness.

## 3. Hang-up: ClosePseudoConsole → grace → termination

- `ClosePseudoConsole` "will send CTRL_CLOSE_EVENT to each client application that is still connected". "Starting
  Windows 11 24H2 (build 26100) ClosePseudoConsole will return immediately … Earlier versions will wait indefinitely
  for the pseudoconsole to exit." On earlier builds, do not call it on the thread that reads the output pipe unless
  that pipe is already closed (V, [ClosePseudoConsole](https://learn.microsoft.com/windows/console/closepseudoconsole)).
  **I:** `windows-2022` (20348) has the blocking behaviour; serviced Windows 11 and `windows-2025` do not. Code that
  serves both must drain or close output on another thread.
- conhost sends the close once when the signal pipe or input pipe closes
  (V, [`VtIo::SendCloseEvent`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/host/VtIo.cpp#L300-L322)).
  It calls `EndTask` for each attached process. The source says: "Win 8 – Win 11 26100: Aborts on an already-dead
  process", and the abort logic "was removed around the Windows 11 26100 time frame, because CSRSS (which handles
  EndTask) now waits 5s and then force-kills the process for us"
  (V, [`input.cpp` L335-L363](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/host/input.cpp#L335-L363)).
  **I:** on pre-26100 inbox conhost (`windows-2022`), one already-dead client can stop later clients from getting the
  close event. The OS conhost build (not the GitHub `main`) determines this, U per build.
- Handler contract (V, [HandlerRoutine](https://learn.microsoft.com/windows/console/handlerroutine)): the default
  handler calls `ExitProcess`. For `CTRL_CLOSE_EVENT`, returning TRUE means "the system terminates the process". The
  timeout is `SPI_GETHUNGAPPTIMEOUT`, 5000 ms. Ctrl+C/Break have no timeout.
  **I:** CSRSS's 5 s equals `HANGUP_GRACE` (5 s), so whether the command dies by CSRSS or by `TerminateJobObject` is a
  race. The reported status can differ between runs.
- **Exit status under CTRL_CLOSE_EVENT.** Microsoft documents only "calls ExitProcess" and does not name the code (V,
  above). S: Eryk Sun, [bugs.python.org msg335488](https://bugs.python.org/msg335488): "the default console control
  handler calls ExitProcess(STATUS_CONTROL_C_EXIT)", which is 0xC000013A. The status after a handler returns TRUE,
  after the CSRSS force-kill, and for cmd.exe, pwsh, powershell 5.1, vim and edit.exe is **U**.
  *CI probe:* per shell profile, run in ConPTY, `ClosePseudoConsole`, and record `GetExitCodeProcess` for the leader
  and a background child on `windows-2022` and `windows-2025`.
- Processes in the session job that are *not* attached to the pseudoconsole (detached, GUI, or with their own
  console) get no close event. Only `TerminateJobObject` after the grace reaches them (I, from §1 and the CTRL+CLOSE doc).
- "When the system is terminating a process, it does not terminate any child processes that the process has created"
  (V, [Terminating a Process](https://learn.microsoft.com/windows/win32/procthread/terminating-a-process)). This is why
  the job sweep is needed.

## 4. Replacing the three `exec` hand-offs

**Spawn-and-wait, and exit width.**
- `Child::wait` uses `WaitForSingleObject` then `GetExitCodeProcess` and keeps the raw `u32`. `ExitStatus::code()`
  returns `Some(self.0 as i32)` (V, [`windows.rs` L705-L762](https://github.com/rust-lang/rust/blob/1.96.0/library/std/src/sys/process/windows.rs#L705-L762)).
- `std::process::exit(i32)` calls `ExitProcess(code as u32)` on Windows
  (V, [`sys/exit.rs` L133](https://github.com/rust-lang/rust/blob/1.96.0/library/std/src/sys/exit.rs#L133)).
  **I:** `exit(status.code().unwrap())` round-trips all 32 bits. The stable public `ExitCode` is only `From<u8>`
  ([`process.rs` L2200](https://github.com/rust-lang/rust/blob/1.96.0/library/std/src/process.rs#L2200)), and
  `ExitCodeExt::from_raw(u32)` is unstable (issue 111688), so `main -> ExitCode` cannot carry 0xC000013A on stable Rust.
- `STILL_ACTIVE` (259) is a legal exit code that is indistinguishable from "running" through `GetExitCodeProcess`
  alone (V, [GetExitCodeProcess](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-getexitcodeprocess)).
  Wait on the handle first, as std does.

**Ctrl+C in the waiting parent.**
- Ctrl+C/Break goes "to all processes sharing that console"
  (V, [SetConsoleCtrlHandler](https://learn.microsoft.com/windows/console/setconsolectrlhandler)). The launcher, the
  CLI and the client all receive it alongside the child.
- `SetConsoleCtrlHandler(NULL, TRUE)` sets an attribute that "is inherited by child processes". **I:** using it in
  the launcher would make the CLI, the client and any directly spawned command ignore Ctrl+C. A registered
  `HandlerRoutine` returning TRUE for `CTRL_C_EVENT`/`CTRL_BREAK_EVENT` is per-process ("does not affect the list of
  handler functions for other processes"). This is the means compatible with ADR 0003's "ignores Ctrl+C".
- A console in raw mode without `ENABLE_PROCESSED_INPUT` reports Ctrl+C "as keyboard input rather than as a signal"
  (V, same). **I:** while the client holds raw mode, no Ctrl+C signal is generated for anyone on that console, while
  Ctrl+Break always is.
- On `CTRL_CLOSE_EVENT` the waiter is terminated anyway once its handler returns or 5 s passes (V, HandlerRoutine).
  Option: the waiter's handler blocks until the child exits, then calls `ExitProcess(child_code)` within that window.
  The resulting status when it loses the race is U.
- `AttachConsole`/`AllocConsole`/`FreeConsole` reset the handler table (V, SetConsoleCtrlHandler).

**Daemon → command without a helper (option).** One `CreateProcessW` with `EXTENDED_STARTUPINFO_PRESENT` and an
attribute list holding `PSEUDOCONSOLE` + `JOB_LIST` + `HANDLE_LIST`
(V, [Creating a Pseudoconsole session](https://learn.microsoft.com/windows/console/creating-a-pseudoconsole-session);
UpdateProcThreadAttribute) creates the command in its terminal and job atomically, and reports launch errors
synchronously. That removes the reason for the Unix `--exec-command` helper and its confirmation socket. It needs
direct `windows-sys` calls, because std's attribute support is unstable (§1).

**Launch failures → 126/127.**
- `CreateProcess` "returns before the process has finished initialization. If a required DLL cannot be located or
  fails to initialize, the process is terminated", and that surfaces as an *exit code*, not a spawn error
  (V, CreateProcessW returns). **I:** such failures arrive as a raw NTSTATUS (for example `STATUS_DLL_NOT_FOUND`), not
  as 126.
- std maps `ERROR_FILE_NOT_FOUND`/`ERROR_PATH_NOT_FOUND`/`ERROR_INVALID_DRIVE`/`ERROR_BAD_NET*` to `NotFound`,
  `ERROR_ACCESS_DENIED` to `PermissionDenied` and `ERROR_DIRECTORY` to `NotADirectory`. `ERROR_BAD_EXE_FORMAT` is
  uncategorised (V, [`io/error/windows.rs` L15-L66](https://github.com/rust-lang/rust/blob/1.96.0/library/std/src/sys/io/error/windows.rs#L15-L66)).
  std's resolver returns `NotFound "program not found"` *before* `CreateProcess` when a bare name is not found
  (V, [`windows.rs` L464-L522](https://github.com/rust-lang/rust/blob/1.96.0/library/std/src/sys/process/windows.rs#L464-L522)).
- Which error a nonexistent or inaccessible `lpCurrentDirectory` yields is **U** (`ERROR_DIRECTORY` is the common
  report). The existing Unix rule, "127 only for NotFound in a usable cwd, else 126", carries over if climon2 resolves
  the program itself (§6) and validates the cwd first, as `validate_cwd` does today.

## 5. Identity: PID + creation time

- `GetProcessTimes` returns creation time as a FILETIME, "points in time … since midnight on January 1, 1601 at
  Greenwich". It needs only `PROCESS_QUERY_LIMITED_INFORMATION`
  (V, [GetProcessTimes](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes)).
- Resolution: kernel system time "based on the latest tick of the system clock timer … typically in the range of
  500 microseconds to 15.625 milliseconds" unless the precise variant is used
  (V, [KeQuerySystemTimePrecise remarks](https://learn.microsoft.com/windows-hardware/drivers/ddi/wdm/nf-wdm-kequerysystemtimeprecise)).
  Which variant stamps process creation is **U**. **I:** epoch µs = (FILETIME − 116444736000000000) / 10 is
  deterministic, so a writer and a reader that apply it to the same stored value compare exactly. A FILETIME earlier
  than 1970 (clock set back before the epoch) is not representable in the frozen u64.
- **Clock steps.** Microsoft does not state whether a later `SetSystemTime` changes a live process's reported
  creation time (**U**). There is no documented boot-relative derivation such as Linux's `btime + ticks`. **I:** the
  value is a stored timestamp, so the Linux failure mode (§1 of the Linux research) is not indicated.
  *CI probe (runners are admin):* read the creation time, step the clock ±1 h with `SetSystemTime`, re-read and
  restore. Run it on both runners.
- **PID reuse.** Learn says an identifier "is valid from the time the process is created until the process has been
  terminated" ([Process Handles and Identifiers](https://learn.microsoft.com/windows/win32/procthread/process-handles-and-identifiers)).
  It also says you "cannot guarantee … that the identifier has not been recycled … unless you maintain an open handle
  to the process" (V, JOBOBJECT_ASSOCIATE_COMPLETION_PORT). A process object outlives termination while handles remain
  open (V, TerminateProcess remarks; Terminating a Process). **I:** a false "same process" needs the PID reused *and*
  an identical tick-quantised creation time. That is possible only after a backward clock step that lands on exactly
  the same 100 ns value, or when a reuse happens within the same tick. Stale cleanup also needs a refused connect
  (ADR 0005). This is the same residual risk as macOS.
- **Liveness without procfs (options).** `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, pid)` then:
  - failure on a nonexistent PID: the error is **U** (Learn documents `ERROR_INVALID_PARAMETER` only for PID 0);
  - `ERROR_ACCESS_DENIED` for protected/system processes (V, [OpenProcess](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-openprocess)),
    and plausibly for a same-user elevated daemon from a non-elevated `list` (**U**). Under ADR 0005, errors are not
    evidence of death;
  - `WaitForSingleObject(h, 0) == WAIT_OBJECT_0` means exited but the object is still alive (someone holds a handle).
    This is the "zombie" analogue: the PID is not reusable yet, and `GetProcessTimes` still reports the creation time
    plus a valid exit time.
  - Do not use `GetExitCodeProcess == STILL_ACTIVE` as liveness (§4).

## 6. PATH × PATHEXT and argument quoting

- **CreateProcess search** (when `lpApplicationName` is NULL): it appends `.exe` only when there is no extension and
  no path, then searches the app directory, the parent's current directory, System32, the 16-bit system directory,
  the Windows directory and finally PATH. It does not use PATHEXT or App Paths
  (V, CreateProcessW `lpCommandLine`).
- **cmd.exe** does its own resolution. It uses `.;%PATH%` unless `NeedCurrentDirectoryForExePath` returns FALSE, which
  happens when `NoDefaultCurrentDirectoryInExePath` exists and the name has no backslash
  (V, [NeedCurrentDirectoryForExePathW](https://learn.microsoft.com/windows/win32/api/processenv/nf-processenv-needcurrentdirectoryforexepathw)).
  No Microsoft Learn primary was found that specifies PATHEXT's order or default (`.COM;.EXE;.BAT;.CMD;…` per S
  sources). Treat it as U and read it from the environment.
- **Rust std resolver** (V, `windows.rs` L464-L560): it never consults PATHEXT. For a bare name it tries `<name>.exe`
  (if there is no `.` in the name) in: the *child's* PATH if the Command changed PATH, then the directory of
  `current_exe()`, then System32, then the Windows directory, then the parent's PATH. The current directory is never
  searched. **I:** for PATHEXT parity (finding `foo.cmd`, `foo.ps1` and so on), climon2 must resolve to an absolute
  path itself and pass that to `Command`. std's app-directory-first order would otherwise prefer a same-named tool
  next to `climon2-daemon.exe` over PATH.
- **MSVC CRT parsing** (V, [Parsing C command-line arguments](https://learn.microsoft.com/cpp/c-language/parsing-c-command-line-arguments)):
  `argv[0]` has special rules (quotes allowed, no escapes). Elsewhere `\` is literal unless it precedes `"`: 2n
  backslashes + `"` gives n backslashes and a delimiter, 2n+1 gives n backslashes and a literal `"`, and `""` inside
  quotes is a literal `"`. std's `append_arg` implements the 2n+1 rule, quotes empty arguments and arguments with
  space or tab, and always quotes argv0
  (V, [`args/windows.rs` L173-L216](https://github.com/rust-lang/rust/blob/1.96.0/library/std/src/sys/args/windows.rs#L173-L216);
  `make_command_line`, `windows.rs` L858-L877). Programs that do not use the CRT parser may split differently (I).
- **`.bat`/`.cmd` (BatBadBut).** CVE-2024-24576 (fixed in 1.77.2): std "did not properly escape arguments when
  invoking batch files" and now returns `InvalidInput` "when it cannot safely escape"
  (V, [Rust advisory](https://blog.rust-lang.org/2024/04/09/cve-2024-24576/)). CVE-2024-43402 (fixed in 1.81.0): names
  with trailing spaces or dots bypassed the check (V, [Rust advisory](https://blog.rust-lang.org/2024/09/04/cve-2024-43402/)).
  std now runs `GetFullPathNameW` before testing the extension
  (V, [`windows.rs` L289-L316](https://github.com/rust-lang/rust/blob/1.96.0/library/std/src/sys/process/windows.rs#L289-L316)).
  In 1.96.0 std:
  - runs `cmd.exe /e:ON /v:OFF /d /c "…"` from the system directory;
  - **refuses** any argument containing CR or LF, any NUL, and script paths containing `"` or ending in `\`
    (`InvalidInput`);
  - quotes every argument containing ASCII outside `[A-Za-z0-9#$*+-./:?@\_]` or any control character, doubles
    embedded `"`, and rewrites `%` as `%%cd:~,%` to defeat expansion;
  - quotes arguments ending in `\`
    (V, [`args/windows.rs` L219-L345](https://github.com/rust-lang/rust/blob/1.96.0/library/std/src/sys/args/windows.rs#L219-L345)).
  - `raw_arg` bypasses all of it (V, advisory).
  **I:** the map's "usage error (2) for arguments cmd cannot safely represent" maps onto std's `InvalidInput` set
  (CR, LF, NUL, `"` / trailing `\` in the script path). climon2 could test that set up front, before spawning the
  daemon, rather than parse a spawn error. std's own comment says no fully correct escaping exists, so the CI corpus
  should round-trip `%`, `^`, `&`, `|`, `!`, `"` and trailing `\` through a real `.cmd` that echoes `%*`.

## Unresolved (for decision tickets, e.g. #212)

- Exact `CreateProcess` error for a forbidden breakaway; host job state for Windows Terminal (MSIX), VS Code and the
  Actions runner.
- Exit codes under `CTRL_CLOSE_EVENT` per shell profile and app. The CSRSS-vs-grace race at 5 s.
- Whether the daemon's open leader handle holds `ActiveProcesses` above zero.
- Creation-time stability across `SetSystemTime`; which clock stamps process creation.
- `OpenProcess` error for a vanished PID; cross-integrity access for `list`.
- Policy choices: breakaway pre-check vs post-check; `KILL_ON_JOB_CLOSE` on the session job; readiness transport;
  helper-free command spawn; own PATH×PATHEXT resolver and its search order (including whether the current directory
  is searched); `.cmd` refusal set.
- CI parity: `windows-2022` differs from every serviced Windows 11 build on ClosePseudoConsole blocking and conhost
  EndTask. `windows-2025` matches 24H2 by build number only.
