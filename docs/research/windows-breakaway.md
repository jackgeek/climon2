# Windows daemon breakaway from host job objects

Research for the Windows `c2 exec` contract: the session daemon that `exec` starts must survive the host terminal
closing, and if it cannot escape the host's job objects, `exec` fails with exit 125. Checked **2026-11-19**. This note
records facts and options only. It chooses no policy and implements nothing. Nothing was run on Windows.

It extends [`windows-lifecycle.md` §1](https://github.com/jackgeek/climon2/blob/research/windows-lifecycle/docs/research/windows-lifecycle.md)
(branch `research/windows-lifecycle`) and the job section of
[`windows-hosts.md` §3](https://github.com/jackgeek/climon2/blob/research/windows-hosts/docs/research/windows-hosts.md)
(branch `research/windows-hosts`), and does not repeat them.

**Terms.** A *job object* ("job") is a kernel object that groups processes. A child process joins its parent's job by
default. Jobs can nest, and the jobs that hold a process form its *job chain*. *Breakaway* means creating a child
outside the parent's job. A job allows it only with the `JOB_OBJECT_LIMIT_BREAKAWAY_OK` limit (the child must ask, with
`CREATE_BREAKAWAY_FROM_JOB`) or `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK` (every child leaves, without asking).
`KILL_ON_JOB_CLOSE` kills every member when the last handle to the job closes. That is the limit that would kill the
daemon.

**Tags.** **V**: verified in a primary source, cited by URL at a pinned commit. **I**: an inference from V facts.
**U**: not established, with a CI probe. **S**: a secondary source, named.

## Material findings

1. **cargo puts everything it runs into a job that forbids breakaway (V).** `cargo` (0.97.2, shipped with Rust 1.96)
   creates a job with only `KILL_ON_JOB_CLOSE` and assigns itself to it. `cargo test` and `cargo run` children
   therefore sit in a job without `BREAKAWAY_OK`. **I:** under the hard contract, `c2 exec` run beneath `cargo test`
   (including `climon2-e2e` in `check.yml`) or `cargo run` *always* exits 125 on Windows. CI must launch the system
   under test from outside cargo's job tree, or use one of the routes in §2.
2. **None of the three hosts adds a job to the shell's chain in its own code (V), but each still has an outer
   unknown (U).** Windows Terminal and node-pty create the shell with a plain `CreateProcessW` (prior research). VS
   Code's ptyHost is an Electron utility process. Chromium puts an unsandboxed utility process in a job only for the
   network service. The libuv job that Node.js uses has `SILENT_BREAKAWAY_OK`, so the shell is never in it. The
   Actions runner creates no job objects, and .NET 8 `Process.Start` adds none. What remains open is the job, if any,
   that the OS or launcher puts around `WindowsTerminal.exe`, `Code.exe` and `Runner.Listener` (§1).
3. **Two routes are documented to bypass the caller's job.** `Win32_Process.Create` (WMI): "Child processes created
   using Win32_Process.Create are not associated with the job" (V). `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`: the child
   inherits the job object of the *specified* parent, not the creator's (V). Task Scheduler and COM out-of-process
   activation start processes from a service, so they escape by inference only (I) (§2).
4. **The error for a forbidden breakaway is still undocumented (U).** Microsoft's own code says `ERROR_ACCESS_DENIED`
   (5) (S). Rust std maps 5 to `ErrorKind::PermissionDenied` (V). Server silos and app silos are jobs in the hierarchy
   (V), so `IsProcessInJob(…, NULL)` is expected to report TRUE inside a Windows container (I) (§3).
5. **PCA's compatibility job is documented only for Vista and Windows 7 (V).** On Windows 11 it is unestablished (U).
   PcaSvc still ships, and on Server only with Desktop Experience (V). Microsoft's guidance is that an executable
   manifest with a UAC run level excludes a program from PCA (S) (§4).

## 1. Do the hosts put the shell in a job?

| Host / launcher | Job it creates | Limits | Effect on a shell or `c2` below it | Tag |
| --- | --- | --- | --- | --- |
| Windows Terminal (`ConptyConnection`) | none | none | Inherits the job chain of `WindowsTerminal.exe` | V (prior) |
| node-pty `startProcess` (conpty) | none | none | Inherits the ptyHost's chain, minus libuv's job (below) | V (prior) |
| node-pty `conpty_console_list_agent` | via `child_process.fork` → libuv job | as libuv | Puts the *ptyHost itself* into libuv's job (below) | V |
| libuv `uv_spawn` (Node, Electron, VS Code) | one global job per process | `BREAKAWAY_OK \| SILENT_BREAKAWAY_OK \| DIE_ON_UNHANDLED_EXCEPTION \| KILL_ON_JOB_CLOSE` | Direct non-detached children die with the parent. Grandchildren are not in the job | V |
| Electron `utilityProcess` (VS Code ptyHost) | none (not the network service) | n/a | No job added | V |
| GitHub Actions runner (`Runner.Worker` → step shell) | none | n/a | No job. Orphans are killed at job end by environment tag | V |
| cargo (`cargo test`, `cargo run`) | one job per cargo | `KILL_ON_JOB_CLOSE` only | Breakaway forbidden for everything below | V |

**Windows Terminal.**
- WT is a full-trust MSIX app: `runFullTrust` and `Windows.FullTrustApplication`
  (V, [`Package.appxmanifest` L161, L238-L239](https://github.com/microsoft/terminal/blob/f88b209f1e57e1d8ca35977260c3cddacccaeb31/src/cascadia/CascadiaPackage/Package.appxmanifest#L161)).
- `PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY` applies "only to win32 applications that have been converted to UWP
  packages by using the Desktop Bridge". `PROCESS_CREATION_DESKTOP_APP_BREAKAWAY_ENABLE_PROCESS_TREE`: "The process
  being created will create any child processes outside of the desktop app runtime environment. This behavior is the
  default for processes for which no policy has been set"
  (V, [UpdateProcThreadAttribute](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)).
  Learn does not say whether this "runtime environment" is a job. MSIX pages that say a packaged app's children run
  "inside the container" do not mention jobs either (prior research).
- **U:** the job state of `WindowsTerminal.exe` and its shells.
  *Probe:* on `windows-2025`, install the WT msixbundle and launch a profile that runs a probe binary. The binary
  records `IsProcessInJob(GetCurrentProcess(), NULL)`, the immediate job's `LimitFlags` from
  `QueryInformationJobObject(NULL, JobObjectExtendedLimitInformation)`, and `GetCurrentPackageFullName` (whether the
  shell has package identity). If the runner cannot show WT, repeat the probe manually on a Windows 11 client.
- **I:** in a default-terminal hand-off, the shell was created by whoever launched it (Explorer, for example), not by
  WT, so it carries that creator's job chain.

**VS Code (ptyHost, node-pty).**
- On the desktop, the ptyHost is started with `utilityProcess.fork` (V, vscode
  [`electronPtyHostStarter.ts` L59](https://github.com/microsoft/vscode/blob/4153a447c20dc476ab5b16fb51fd0a639779afaf/src/vs/platform/terminal/electron-main/electronPtyHostStarter.ts#L59),
  [`utilityProcess.ts` L261](https://github.com/microsoft/vscode/blob/4153a447c20dc476ab5b16fb51fd0a639779afaf/src/vs/platform/utilityProcess/electron-main/utilityProcess.ts#L261)).
  Electron launches it as a `kNoSandbox` Node service
  (V, Electron [`node_service.mojom` L41](https://github.com/electron/electron/blob/b3afd52d4d62f5b7cb62eec1cf2be850f6cdc071/shell/services/node/public/mojom/node_service.mojom#L41),
  [`electron_api_utility_process.cc` L233](https://github.com/electron/electron/blob/b3afd52d4d62f5b7cb62eec1cf2be850f6cdc071/shell/browser/api/electron_api_utility_process.cc#L233)).
- Chromium launches an unsandboxed process in a `KILL_ON_JOB_CLOSE` job only when `ShouldUnsandboxedRunInJob()` is
  true (V, [`sandbox_win.cc` L614-L652](https://github.com/chromium/chromium/blob/37a246ec832f97c04ea9aab4bb9691b65f793d43/sandbox/policy/win/sandbox_win.cc#L614-L652)).
  That function returns false by default
  ([`sandboxed_process_launcher_delegate.cc` L52-L54](https://github.com/chromium/chromium/blob/37a246ec832f97c04ea9aab4bb9691b65f793d43/content/browser/sandboxed_process_launcher_delegate.cc#L52-L54)),
  and for utility processes it is true only for the network service
  ([`utility_sandbox_delegate_win.cc` L537-L544](https://github.com/chromium/chromium/blob/37a246ec832f97c04ea9aab4bb9691b65f793d43/content/browser/service_host/utility_sandbox_delegate_win.cc#L537-L544)).
- libuv creates its global job with the limits shown in the table and **assigns the calling process to it** on its first
  non-detached spawn. It then assigns each non-detached child. Detached spawns use
  `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` and deliberately do not set `CREATE_BREAKAWAY_FROM_JOB`
  (V, libuv [`process.c` L69-L121, L1117-L1167](https://github.com/libuv/libuv/blob/f08538cca14f73e30c89625e473fea3f5f55e300/src/win/process.c#L69-L121)).
- node-pty's `kill()` with `useConptyDll` false forks `conpty_console_list_agent` through `child_process.fork`. The
  agent attaches to the shell's console, lists its processes, and the ptyHost then kills each PID it returns
  (V, node-pty [`windowsPtyAgent.ts` L206-L252](https://github.com/microsoft/node-pty/blob/35c28c7d68a192699e70b1e6a142908bdccd9212/src/windowsPtyAgent.ts#L206-L252)).
  **I:** a `DETACHED_PROCESS` daemon is not attached to that console, so this sweep does not reach it.
- The remote/server ptyHost is started with Node `child_process.fork` (V,
  [`nodePtyHostStarter.ts` L45](https://github.com/microsoft/vscode/blob/4153a447c20dc476ab5b16fb51fd0a639779afaf/src/vs/platform/terminal/node/nodePtyHostStarter.ts#L45)),
  so it sits in the server's libuv job.
- `SILENT_BREAKAWAY_OK` lets "any process associated with the job … create child processes that are not associated with
  the job". In a nested chain the child moves up until it reaches a job that forbids breakaway (V,
  [JOBOBJECT_BASIC_LIMIT_INFORMATION](https://learn.microsoft.com/windows/win32/api/winnt/ns-winnt-jobobject_basic_limit_information)).
  **I:** a shell that node-pty creates is never in a libuv job, and it keeps only the jobs above that libuv itself
  inherited. The same applies to `c2` started from any Node-based tool: npm scripts, VS Code tasks, agent CLIs.
- **U:** whether `Code.exe` itself starts inside a job. That depends on its launcher, such as Explorer, the Start menu
  or a `code` CLI run from a shell in a job. *Probe:* the same probe binary run from a VS Code integrated terminal
  (manual, or with `@vscode/test-electron` on `windows-2025`). A cheaper partial probe: a Node script on the runner calls
  `child_process.fork()` once, then uses node-pty to spawn the probe binary, and checks that the binary is in no job.

**GitHub Actions runner.**
- The runner source contains no job-object API. Its only native process calls are `NtQueryInformationProcess`, console
  control and `kill` (V, by search of actions/runner at
  [`7e211dc`](https://github.com/actions/runner/tree/7e211dc69c347b78fc398e630e74facb11a13235)). Steps are started with
  .NET `Process.Start` and `UseShellExecute=false`
  ([`ProcessInvoker.cs` L230-L231](https://github.com/actions/runner/blob/7e211dc69c347b78fc398e630e74facb11a13235/src/Runner.Sdk/ProcessInvoker.cs#L230-L231)).
  .NET 8 sets only `CREATE_NO_WINDOW` and `CREATE_UNICODE_ENVIRONMENT`
  (V, dotnet/runtime [`Process.Windows.cs` L505-L512](https://github.com/dotnet/runtime/blob/8d6803926e5c9d407b1d71fd33b46cae5a1e0fca/src/libraries/System.Diagnostics.Process/src/System/Diagnostics/Process.Windows.cs#L505-L512)).
- **Orphan cleanup (V).** Unless `process.clean` is false, the worker sets `RUNNER_TRACKING_ID=github_<guid>`. At the end
  of the job it kills every new process whose environment carries that value
  ([`JobExtension.cs` L588-L600, L888-L935](https://github.com/actions/runner/blob/7e211dc69c347b78fc398e630e74facb11a13235/src/Runner.Worker/JobExtension.cs#L588-L600)).
  Cancelling a step kills the tree by walking `InheritedFromUniqueProcessId`
  ([`ProcessInvoker.cs` L667](https://github.com/actions/runner/blob/7e211dc69c347b78fc398e630e74facb11a13235/src/Runner.Sdk/ProcessInvoker.cs#L667)).
  **I:** a daemon survives the step that started it but not the end of the job (it inherits the variable). The tree
  walk on cancel follows parent PIDs, so it reaches the daemon only while every process between the step and the
  daemon is still alive.
- **U:** whether the hosted image starts `Runner.Listener` inside a job. *Probe:* the first step of a `windows-2025`
  job runs the probe binary from `pwsh` and from `cmd`.

**cargo.** `setup()` creates a job, sets `LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` and calls
`AssignProcessToJobObject(job, GetCurrentProcess())`. On a normal exit, `Drop` clears the limits, so children are not
killed (V, [`job.rs` L81-L146 at cargo 0.97.2](https://github.com/rust-lang/cargo/blob/0.97.2/src/cargo/util/job.rs#L81-L146)).
**I:** `cargo test` children cannot break away and cannot leave the job by any `CreateProcess` flag.

## 2. Starting a process outside the caller's job chain

| Route | Documented job effect | Main caveats | Tag |
| --- | --- | --- | --- |
| `CREATE_BREAKAWAY_FROM_JOB` | Leaves jobs only while each allows breakaway | Fails under cargo, or a job without `BREAKAWAY_OK` | V (prior) |
| `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS` | Child inherits the **specified parent's** job | Needs `PROCESS_CREATE_PROCESS` on a suitable parent; token and handles also come from it | V |
| WMI `Win32_Process.Create` | "not associated with the job" (of the caller) | Lands in the WMI provider-host job unless it breaks away; COM/WMI latency; often flagged | V / I |
| Task Scheduler on-demand task | Not documented | Registration; default 72 h kill, battery rules, `IgnoreNew`; environment and cwd come from the task | I / U |
| COM out-of-proc server / Explorer `ShellExecute` | Not documented | Registration or a running Explorer; child carries the server's or Explorer's chain | I / S |
| Windows service | Not documented | Creating a service needs an administrator | V (rights) |

**`PROC_THREAD_ATTRIBUTE_PARENT_PROCESS` (V,
[UpdateProcThreadAttribute](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)).**
"The handle for the process used must have the PROCESS_CREATE_PROCESS access right. Attributes inherited from the
specified process include handles, the device map, processor affinity, priority, quotas, the process token, and job
object. (Note that some attributes such as the debug port will come from the creating process…)".
- **Job:** the child joins the specified parent's job, not the creator's (V). If that parent is itself in a
  no-breakaway job, the problem only moves (I).
- **Token and integrity:** the child gets the parent's token (V). An elevated `c2` that names Explorer as the parent
  gets an unelevated daemon (I).
- **Handles:** inheritable handles come from the specified parent (V). **I:** a readiness pipe would first have to be
  duplicated into that parent. A named endpoint path passed on the command line avoids this.
- **Choosing a parent:** the shell process found through `GetShellWindow`/`GetWindowThreadProcessId` is one option (I).
  Explorer may not exist on Server Core or in a service session (U).
- **Rust:** std cannot pass attribute lists on stable (`raw_attribute` is unstable; prior research), so this needs a
  direct `CreateProcessW`.
- **Availability:** listed with no minimum OS beyond Vista (V).
- **Policy:** "parent PID spoofing" is MITRE ATT&CK technique
  [T1134.004](https://attack.mitre.org/techniques/T1134/004/), and EDR products commonly watch for it (S, MITRE).

**WMI `Win32_Process.Create`.**
- [Job Objects](https://learn.microsoft.com/windows/win32/procthread/job-objects): "(Child processes created using
  Win32_Process.Create are not associated with the job.)" (V,
  [source L32](https://github.com/MicrosoftDocs/win32/blob/bc82a65f74ef2c4f2f44a3d7db5f7cfebfdc3583/desktop-src/ProcThread/job-objects.md?plain=1#L32)).
- The method's own page: "Processes created with the Win32_Process.Create method are limited by the job object unless
  the CREATE_BREAKAWAY_FROM_JOB flag is specified. For more information, see Win32_ProcessStartup and
  __ProviderHostQuotaConfiguration" (V,
  [Create method](https://learn.microsoft.com/windows/win32/cimwin32prov/create-method-in-class-win32-process)).
  The quota class configures "the job object limits for the host job object" (V,
  [__ProviderHostQuotaConfiguration](https://learn.microsoft.com/windows/win32/wmisdk/--providerhostquotaconfiguration)).
  **I:** "the job" here is the WMI provider host's job. Setting `Win32_ProcessStartup.CreateFlags =
  CREATE_BREAKAWAY_FROM_JOB` (16777216, V) is the documented way out of it.
- **U:** the provider-host job's limits (`KILL_ON_JOB_CLOSE`?), and whether the breakaway flag succeeds there.
- **Other limits:** "cannot be used to start an interactive process remotely" (V). The environment and current
  directory come from `Win32_ProcessStartup.EnvironmentVariables` and the `CurrentDirectory` argument (V). The session
  and token of a local call are U.
- WMI process creation is ATT&CK [T1047](https://attack.mitre.org/techniques/T1047/) (S, MITRE).
- Winmgmt is always installed and starts automatically on Server (V,
  [Server services guide](https://learn.microsoft.com/windows-server/security/windows-services/security-guidelines-for-disabling-system-services-in-windows-server),
  Server 2016 page).
- *Probe (`windows-2025`):* from inside a cargo job, call `Invoke-CimMethod Win32_Process Create` with and without
  the breakaway flag. Record `IsProcessInJob`, the job's `LimitFlags`, session ID, token integrity, environment and
  latency.

**Task Scheduler (I/U).**
- Documented defaults (V, [ExecutionTimeLimit](https://learn.microsoft.com/windows/win32/taskschd/tasksettings-executiontimelimit) and sibling Task Scheduler reference pages): `ExecutionTimeLimit` "By default, a task will be stopped 72 hours
  after it starts" (`PT0S` means unlimited). `DisallowStartIfOnBatteries` defaults to True. `MultipleInstancesPolicy`
  defaults to `IgnoreNew`, so a second run of the same task is dropped unless it is `Parallel`. The `InteractiveToken`
  logon type runs "only in an existing interactive session". `RegisteredTask.RunEx` passes per-run parameters.
- The Schedule service is always installed and starts automatically on Server (V, Server services guide).
- **U:** whether tasks run in a job, and whether stopping a task kills the process tree. *Probe:* register a
  per-user on-demand task that runs the probe binary, record its job state and parent, then stop the task and check
  whether a grandchild survives.

**COM out-of-process server and Explorer `ShellExecute` (I/S).**
- "The DCOMLAUNCH service launches COM and DCOM servers in response to object activation requests" (V, Server
  services guide). **I:** a `LocalServer32` server starts outside the activator's job chain, but registration and
  activation identity are extra moving parts.
- `IShellDispatch2::ShellExecute`, reached through the desktop's `IShellWindows`, runs the command from Explorer.
  Raymond Chen documents this as the way to launch an unelevated process from an elevated one (S, *The Old New Thing*,
  Microsoft DevBlogs). **I:** the child inherits Explorer's environment, token and job chain, not the caller's.

**Windows services.** By default only Administrators hold `SC_MANAGER_CREATE_SERVICE`, while local authenticated users
get connect, enumerate and query (V,
[Service Security and Access Rights](https://learn.microsoft.com/windows/win32/services/service-security-and-access-rights)).
Not available to a normal `c2 exec`.

## 3. Failure signal and query limits

- **Forbidden breakaway (U).** No Microsoft documentation names the error. `microsoft/conductor` handles it as
  "Win32 ERROR_ACCESS_DENIED — the error code raised when CreateProcess is called with CREATE_BREAKAWAY_FROM_JOB and the
  parent's job object has JOB_OBJECT_LIMIT_BREAKAWAY_OK cleared" (S,
  [`bg_runner.py` L146-L149](https://github.com/microsoft/conductor/blob/c0ce6e1907afa1c281ddd0a8678e9ba036e5682a/src/conductor/cli/bg_runner.py#L146-L149)).
  A Microsoft support blog traces error 5 from `CreateProcessAsUser` to binding the child to its parent's job (S,
  [alejacma, 2012](https://learn.microsoft.com/archive/blogs/alejacma/createprocessasuser-fails-with-error-5-access-denied-when-using-jobs)).
  Rust std maps `ERROR_ACCESS_DENIED` to `PermissionDenied`
  (V, [`sys/io/error/windows.rs` L20](https://github.com/rust-lang/rust/blob/1.96.0/library/std/src/sys/io/error/windows.rs#L20)).
  **I:** error 5 also has other causes (file ACLs), so it alone does not identify a refused breakaway.
  *Probe:* inside a test-created job without `BREAKAWAY_OK`, and under `cargo test`, spawn with the flag and record
  `GetLastError`.
- **`QueryInformationJobObject(NULL, …)`** reads only the immediate job, and `IsProcessInJob(h, NULL)` tests "any job"
  (V, prior research). Neither page mentions silos or containers (V, pages read in full).
- **Silos are jobs (V).** `PsGetJobSilo` "returns the first job in the hierarchy that is a Silo … both app silos and
  server silos" (V,
  [PsGetJobSilo](https://learn.microsoft.com/windows-hardware/drivers/ddi/ntddk/nf-ntddk-psgetjobsilo)).
  **I:** inside a process-isolated Windows container, the post-check reports "in a job" even though the container,
  not a terminal, is the boundary, so `exec` would exit 125. **U:** the silo job's breakaway limits, and whether MSIX
  packaged apps run in an app silo. *Probe:* if `windows-2025` has a container runtime, run the probe binary in a
  process-isolated `servercore` container.
- **Terminal Services rule (V):** "All processes within a job must run within the same session as the job"
  ([AssignProcessToJobObject](https://learn.microsoft.com/windows/win32/api/jobapi2/nf-jobapi2-assignprocesstojobobject)).
  This matters for any route that crosses sessions.

## 4. Program Compatibility Assistant (PCA)

- The PCA compatibility job is documented only for "Windows 7, Windows Server 2008 R2, Windows Server 2008 and Windows
  Vista": "If the process is being monitored by the Program Compatibility Assistant (PCA), it is placed into a
  compatibility job. Therefore, the process must be created using CREATE_BREAKAWAY_FROM_JOB before it can be placed in
  another job. Alternatively, you can embed an application manifest that specifies a User Account Control (UAC) level"
  (V, [AssignProcessToJobObject](https://learn.microsoft.com/windows/win32/api/jobapi2/nf-jobapi2-assignprocesstojobobject)).
  The Windows 8 PCA scenarios page does not mention jobs (V,
  [PCA scenarios](https://learn.microsoft.com/windows/compatibility/pca-scenarios-for-windows-8)).
  That page says that TRUSTINFO and COMPATIBILITY manifest sections make PCA treat an app as designed for the current
  Windows.
- PcaSvc: "PCA monitors programs installed and run by the user". It is installed on Server only with Desktop
  Experience, with Automatic startup (V, Server services guide).
- PcaSvc itself called `AssignProcessToJobObject` on a customer's process on Windows 7. The process was excluded by a
  UAC manifest, the `ExecutablesToExclude` registry value or the "Turn Off Program Compatibility Assistant" policy (S,
  [alejacma, 2012](https://learn.microsoft.com/archive/blogs/alejacma/why-is-my-process-in-a-job-if-i-didnt-put-it-there)).
- **U:** whether PCA wraps user processes in a job on Windows 11, and that job's breakaway limits. Since Windows 8,
  jobs nest, so a PCA job would no longer block other job assignment (I). *Probe:* run an unmanifested and a
  manifested build of the probe binary from Explorer on a Windows 11 client (manual), and on `windows-2025`.

## Unresolved items and CI probes

One small **probe binary** answers most of these. It prints `IsProcessInJob(self, NULL)`, the immediate job's
`LimitFlags`, `GetCurrentPackageFullName`, session ID, token integrity and parent PID. With `--spawn` it also tries
`CREATE_BREAKAWAY_FROM_JOB | DETACHED_PROCESS` and prints `GetLastError` and the child's `IsProcessInJob`.

| Item | Probe |
| --- | --- |
| Error for a forbidden breakaway | `windows-2025`: run `--spawn` inside a job without `BREAKAWAY_OK`, and under `cargo test` |
| Actions runner outer job | First step of a `windows-2025` job: run the probe from `pwsh` and `cmd` |
| cargo capture (confirms §1) | Run `--spawn` from a `cargo test` integration test; expect a failure or a still-in-job child |
| VS Code / node-pty | Node script: `fork()` once, then node-pty spawns the probe; expect no job. Real VS Code: manual or `@vscode/test-electron` |
| Windows Terminal / MSIX | Install the WT msixbundle on `windows-2025` and run the probe from a profile; otherwise manual on a Windows 11 client |
| WMI route | `Invoke-CimMethod Win32_Process Create` from inside a cargo job, with and without `CreateFlags=0x01000000`; record job, limits, session, token, environment, latency |
| PARENT_PROCESS route | From inside a no-breakaway job, create the probe with Explorer (or the runner's `Runner.Listener`) as parent; record job, token, handle inheritance |
| Task Scheduler route | Register an on-demand per-user task that runs the probe, then stop the task; record job, parent and survival |
| Silos / containers | Run the probe in a process-isolated `servercore` container, if the runner has a container runtime |
| PCA on Windows 11 | Run unmanifested and manifested probes from Explorer on a Windows 11 client (manual) |

## Sources (pinned)

Microsoft Learn pages as cited, read from
[MicrosoftDocs/win32 `bc82a65`](https://github.com/MicrosoftDocs/win32/tree/bc82a65f74ef2c4f2f44a3d7db5f7cfebfdc3583),
[MicrosoftDocs/sdk-api `656e965`](https://github.com/MicrosoftDocs/sdk-api/tree/656e9656bfaef34dd29cd7ee9e1146aec17e924c) and
[MicrosoftDocs/windows-driver-docs-ddi `7515063`](https://github.com/MicrosoftDocs/windows-driver-docs-ddi/tree/7515063cea4c9e98db6a92986c5b4ddb0463fd16).
Code: microsoft/terminal `f88b209`, microsoft/node-pty `35c28c7`, microsoft/vscode `4153a44`, libuv `f08538c`,
chromium `37a246e`, electron `b3afd52`, actions/runner `7e211dc`, dotnet/runtime `release/8.0` `8d68039`,
rust-lang/cargo `0.97.2`, rust-lang/rust `1.96.0`.
