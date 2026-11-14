# Windows host terminals for qualification

Research for [#209](https://github.com/jackgeek/climon2/issues/209), under
[map #205](https://github.com/jackgeek/climon2/issues/205); informs
[#213](https://github.com/jackgeek/climon2/issues/213). Checked **2026-11-14** against `main`
`0b42d32`. Facts and options only: no policy is chosen, nothing is implemented, no Windows
machine was used.

**Scope changes received during the research.** Windows 10 is out of scope (Windows 11 only), and
GitHub's `windows-2025` Server runner is accepted as the qualifying environment for Windows 11.
§7 therefore lists the differences between Server and Windows 11 that could make a `windows-2025`
result misleading, instead of looking for Windows 11 runners.

**Tags.** **V** = verified in a cited primary source: source code at a pinned commit, Microsoft
Learn, or a first-party image manifest. **I** = an inference from V facts. **U** = not
established; each U comes with a probe that would settle it.

**Pinned sources.**

| Short | Repository / document | Revision |
| --- | --- | --- |
| `T` | [microsoft/terminal](https://github.com/microsoft/terminal/tree/f6de9e0618015def1c9dda5b82f9d3abf03eb799) | `f6de9e0` (2026-11-11) |
| `N` | [microsoft/node-pty](https://github.com/microsoft/node-pty/tree/35c28c7d68a192699e70b1e6a142908bdccd9212) | `35c28c7` (2026-11-02) |
| `C` | [microsoft/vscode](https://github.com/microsoft/vscode/tree/ff9acdc74d3fbc338a6e50da2c31695eb0176a33) | `ff9acdc` (2026-11-14, `main`; shipped builds may lag) |
| `X` | [xtermjs/xterm.js](https://github.com/xtermjs/xterm.js/tree/f70764dd3da05ebda0390605c6f9adbbea3f0e90) | `f70764d` |
| `RI` | [actions/runner-images](https://github.com/actions/runner-images/tree/95e6f51a068507e8f43d061d2166a39d0ef36ce8) | `95e6f51` (2026-11-13) |
| `R` | [actions/runner](https://github.com/actions/runner/tree/8cfa1049f86781d141056082d73802d56cd2b433) | `8cfa104` (2026-11-11) |
| `E` | [microsoft/edit](https://github.com/microsoft/edit/tree/b7f83220868373945945e2218e852c24cbb4dcf0) | `b7f8322` (v2.0.0) |
| `PP` | [portable-pty 0.9.0](https://docs.rs/crate/portable-pty/0.9.0/source/) (the version in the workspace's `Cargo.toml` files) | crates.io 0.9.0 |
| `UV` | [libuv/libuv](https://github.com/libuv/libuv/tree/fc7475a4dd8144e3eda0bf103892d774d1b675e1) | `fc7475a` |

## Answer in brief

1. **Both first-class hosts use a bundled ConPTY by default, not the inbox one.** Windows Terminal
   links `conptylib` statically and launches its packaged `OpenConsole.exe`. VS Code's
   `terminal.integrated.windowsUseConptyDll` defaults to `true` on `main`, so it loads node-pty's
   bundled `conpty.dll` 1.25.260303002 and that DLL's `OpenConsole.exe`. Both builds come from the
   `microsoft/terminal` source tree. Microsoft publishes the same pair as the
   `Microsoft.Windows.Console.ConPTY` NuGet package (§1).
2. **Modern ConPTY has no win32-input-mode or passthrough creation flag.** The current
   `CreatePseudoConsole` flags are `INHERIT_CURSOR` (0x1), glyph width (0x08/0x10/0x18) and
   `AMBIGUOUS_IS_WIDE` (0x20). Win32-input-mode is negotiated in-band: OpenConsole writes
   `CSI ?9001h` to its host. Windows Terminal honours it. xterm.js honours it only when VS Code's
   `terminal.integrated.enableWin32InputMode` is set, which defaults to `false` (experiment
   `auto`). Windows Terminal passes 0x08 (graphemes) by default and adds 0x1 when it restores or
   restarts a session. node-pty passes 0, or 0x1 when VS Code supplies initial text (§1, §2).
3. **Neither host's own source creates a job object.** Windows Terminal, node-pty, VS Code's
   terminal code and conhost/OpenConsole contain no production `CreateJobObject` or
   `AssignProcessToJobObject`. Whether the *environment* adds one remains **U**: the MSIX package
   context for Windows Terminal, and the Electron/Chromium utility process that hosts VS Code's
   pty. A one-line `IsProcessInJob`/`QueryInformationJobObject` probe in each real app settles it
   (§3).
4. **What happens on close differs between the hosts.**
   - **Windows Terminal:** closing the tab closes the pseudoconsole. OpenConsole then sends
     `CTRL_CLOSE_EVENT` to every attached process through CSRSS `EndTask`. CSRSS waits 5 s
     (`SPI_GETHUNGAPPTIMEOUT`) and then kills the process.
   - **VS Code (default `conpty.dll`):** waits 250 ms for data to flush, closes the input pipe,
     calls `ConptyClosePseudoConsole`, then calls `TerminateProcess(shell, 1)` immediately. Other
     attached processes, such as a climon2 client started from the shell, still get
     `CTRL_CLOSE_EVENT` and the 5 s window.
   - **VS Code with the setting off (inbox ConPTY):** TerminateProcesses (exit code 1) every
     process attached to the console, with no close event.
   - A console process can exit with its own code from inside its `CTRL_CLOSE_EVENT` handler
     (`ExitProcess`), so 129 can be produced (§4).
5. **Most host behaviour can be modelled in `climon2-test-term`; portable-pty cannot model it.**
   A harness on `windows-2025` can load a pinned `conpty.dll` and `OpenConsole.exe`, pass each
   host's flags, run each host's close sequence, and wrap the client in nested job objects of any
   shape. portable-pty 0.9.0 hard-codes flags `0x1|0x2|0x4` and its `kill()` is
   `TerminateProcess(…, 1)`, so neither host can be modelled through it. The rendering, key
   encoding and UI close paths of the real apps, and their job context, stay on a manual
   checklist (§5).
6. **Runner availability.** `windows-latest` is Windows Server 2025 (build 26100). The image
   ships pwsh 7.6.x, Chocolatey and Git for Windows (MSYS2 vim only). Windows PowerShell 5.1 and
   `cmd.exe` are part of Windows itself. Native vim for Windows and Microsoft Edit are **not** on
   the image. Edit 2.0.0 is available as a GitHub release zip, and vim as the
   `vim/vim-win32-installer` zips. Controlled-startup options:
   - pwsh and powershell: `-NoProfile`
   - cmd: `/D`
   - vim: `--clean` (or `-u NONE -i NONE -N`)
   - Edit: no flag; point `APPDATA` at an empty directory (§6).
7. **Server versus Windows 11.** These matter only where the inbox stack is used: inbox
   conhost/ConPTY revision, Windows 11 25H2's inbox `edit.exe` 1.2.1, the default-terminal
   handoff, the runner's session context, and the hosts not being installed on the image.
   Because both hosts bundle their own ConPTY, a harness that pins the bundled ConPTY keeps
   `windows-2025` results representative for the hosts (§7).

## 1. Which ConPTY each host uses

**Windows Terminal: bundled OpenConsole (V).**
- `ConptyConnection` includes `conpty-static.h` and calls `ConptyCreatePseudoConsole`,
  `ConptyReparentPseudoConsole`, `ConptyShowHidePseudoConsole` and `ConptyReleasePseudoConsole`
  (`T` [`ConptyConnection.cpp:7,412-451`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/cascadia/TerminalConnection/ConptyConnection.cpp#L407-L451)).
- The project links `conptylib.lib` statically and defines `CONPTY_IMPEXP=` "so we don't try to
  dllimport conpty" (`T` [`TerminalConnection.vcxproj:84-96`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/cascadia/TerminalConnection/TerminalConnection.vcxproj#L84-L96)).
- The package bundles `OpenConsole.exe`
  (`T` [`WindowsTerminal.vcxproj:193`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/cascadia/WindowsTerminal/WindowsTerminal.vcxproj#L193),
  [`CascadiaPackage.wapproj:86`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/cascadia/CascadiaPackage/CascadiaPackage.wapproj#L86)).
- `winconpty` resolves the host to the `OpenConsole.exe` beside the module, then
  `<arch>\OpenConsole.exe`, and falls back to `System32\conhost.exe` only if neither exists
  (`T` [`winconpty.cpp:40-97`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/winconpty/winconpty.cpp#L40-L97)).
- Latest WT releases: stable `v1.25.3131.0` and preview `v1.26.3133.0`, both published
  2026-11-11 (GitHub releases API).

**VS Code: node-pty with the bundled `conpty.dll` by default (V).**
- `terminal.integrated.windowsUseConptyDll` has `default: true` and describes itself as "the
  conpty.dll (v1.25.260303002) shipped with VS Code"
  (`C` [`terminalConfiguration.ts:496-501`](https://github.com/microsoft/vscode/blob/ff9acdc74d3fbc338a6e50da2c31695eb0176a33/src/vs/workbench/contrib/terminal/common/terminalConfiguration.ts#L496-L501)).
- `useConpty` requires build ≥ 18309, and `useConptyDll = useConpty && windowsUseConptyDll`
  (`C` [`terminalProcess.ts:161-173`](https://github.com/microsoft/vscode/blob/ff9acdc74d3fbc338a6e50da2c31695eb0176a33/src/vs/platform/terminal/node/terminalProcess.ts#L161-L173)).
- winpty has been removed (`:257`).
- node-pty loads `conpty\conpty.dll` from its release directory, otherwise `kernel32`'s
  `CreatePseudoConsole` (`N` [`conpty.cc:166-238`](https://github.com/microsoft/node-pty/blob/35c28c7d68a192699e70b1e6a142908bdccd9212/src/win/conpty.cc#L166-L238)).
- node-pty vendors `third_party/conpty/1.25.260303002/win10-{x64,arm64}/{conpty.dll,OpenConsole.exe}`.
- The default may differ in older shipped VS Code builds (**U** for any specific release; check
  that release's settings).

**Official redistributable (V).** NuGet `Microsoft.Windows.Console.ConPTY` is authored by
Microsoft, has project URL `github.com/microsoft/terminal`, and is described as "allows
applications to host Windows Console sessions". Stable versions include `1.24.261001001`,
`1.25.260930003` and `1.25.261109001`. VS Code's `1.25.260303002` is listed there as
`-preview` (nuget.org v3 flat-container index for `microsoft.windows.console.conpty`). That the
1.25 NuGet line matches Windows Terminal 1.25 is **I**: the version prefixes agree.

## 2. `CreatePseudoConsole` flags and in-band modes

**Flags the current ConPTY understands (V).** `T` [`conpty-static.h:26-38`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/inc/conpty-static.h#L26-L38)
defines `INHERIT_CURSOR` 0x1, `GLYPH_WIDTH__MASK` 0x18 (`GRAPHEMES` 0x08, `WCSWIDTH` 0x10,
`CONSOLE` 0x18) and `AMBIGUOUS_IS_WIDE` 0x20. `_CreatePseudoConsole` maps only these onto
OpenConsole's arguments: `--headless`, `--inheritcursor`, `--ambiguousIsWide`,
`--textMeasurement`, `--width`, `--height`, `--signal` and `--server`
(`T` [`winconpty.cpp:169-205`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/winconpty/winconpty.cpp#L169-L205)).
Bits 0x2 and 0x4 are ignored, and 0x8 now means "graphemes".

**Older flag names still in client code (V).** node-pty still declares `PSEUDOCONSOLE_RESIZE_QUIRK`
(0x2) and `PSEUDOCONSOLE_PASSTHROUGH_MODE` (0x8) but never passes them
(`N` [`conpty.h:26-27`](https://github.com/microsoft/node-pty/blob/35c28c7d68a192699e70b1e6a142908bdccd9212/src/win/conpty.h#L26-L27);
the only flag passed is `inheritCursor ? 1 : 0`, [`conpty.cc:293`](https://github.com/microsoft/node-pty/blob/35c28c7d68a192699e70b1e6a142908bdccd9212/src/win/conpty.cc#L293)).
portable-pty 0.9.0 always passes `INHERIT_CURSOR | RESIZE_QUIRK | WIN32_INPUT_MODE` (0x7)
(`PP` [`src/win/psuedocon.rs`](https://docs.rs/crate/portable-pty/0.9.0/source/src/win/psuedocon.rs)).
Against the bundled ConPTY, that means "inherit cursor" plus two ignored bits (**I** from the
mapping above). How the *inbox* `kernel32` ConPTY on build 26100 interprets 0x2 and 0x4 is
**U**; the inbox source is not public.

**Flags each host sets (V).**
- **Windows Terminal** starts from `_flags = 0`. It adds `INHERIT_CURSOR` when `inheritCursor`
  is set, the glyph-width bits from `textMeasurement`, and `AMBIGUOUS_IS_WIDE`
  (`T` [`ConptyConnection.cpp:262-295`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/cascadia/TerminalConnection/ConptyConnection.cpp#L262-L295)).
  - `textMeasurement` comes from `compatibility.textMeasurement`, whose enum's first (default)
    member is `Graphemes`
    (`T` [`EventArgs.idl:22-27`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/cascadia/TerminalControl/EventArgs.idl#L22-L27),
    [`TerminalPage.cpp:1558-1573`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/cascadia/TerminalApp/TerminalPage.cpp#L1558-L1573)).
  - `inheritCursor` is true when restarting a connection or when a session id is restored
    (`TerminalPage.cpp:1703,3900`).
  - So the default is **0x08**, plus 0x1 on restore or restart (**I**: a settings default of
    `Graphemes` is assumed from the enum order).
- **VS Code** passes `conptyInheritCursor: useConpty && !!shellLaunchConfig.initialText`
  (`C` `terminalProcess.ts:173`), so the flags are **0**, or 0x1 when there is initial text.
  The glyph-width mode is then OpenConsole's default.

**Win32-input-mode and focus events are in-band, not flags (V).** When OpenConsole's VT input
thread starts, it writes `CSI c` (DA1), `CSI ?1004h` and `CSI ?9001h`, and it resets the last two
on shutdown (`T` [`VtIo.cpp:194-204,251-262`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/host/VtIo.cpp#L194-L262)).
- **Windows Terminal** implements DECSET 9001
  (`T` `src/terminal/adapter/adaptDispatch.cpp:1893`) unless forced off
  (`src/terminal/input/terminalInput.cpp:96,231`).
- **xterm.js** honours 9001 only when `vtExtensions.win32InputMode` is set
  (`X` [`InputHandler.ts:2044-2046`](https://github.com/xtermjs/xterm.js/blob/f70764dd3da05ebda0390605c6f9adbbea3f0e90/src/common/InputHandler.ts#L2044-L2046)).
  VS Code feeds that from `terminal.integrated.enableWin32InputMode`: `default: false`, tagged
  `experimental` and `experiment: { mode: 'auto' }`
  (`C` [`terminalConfiguration.ts:623-632`](https://github.com/microsoft/vscode/blob/ff9acdc74d3fbc338a6e50da2c31695eb0176a33/src/vs/workbench/contrib/terminal/common/terminalConfiguration.ts#L623-L632);
  `xtermTerminal.ts:285,627`).
- So a climon2 client typically receives input in two different ways:
  - **Windows Terminal:** win32-input-mode records, which OpenConsole decodes into
    `INPUT_RECORD`s.
  - **VS Code:** plain VT, which OpenConsole parses back into keys, so information such as
    modifier-only keys is lost.

  The VS Code experiment can flip this per user (**I**).
- **"Passthrough" mode** survives only as unused constants (above). Current ConPTY has no
  passthrough path (V: no `passthrough` in `T` `src/host`, `src/winconpty`, `src/inc` or
  `TerminalConnection`).

## 3. Job objects

**Neither host's code creates job objects (V, by exhaustive search of the pinned trees).**
- In `T`, `JobObject`/`JOB_OBJECT` appears only in test code (`src/host/ft_*`,
  `WindowsTerminal_UIATests`).
- In `N/src` and `C/src/vs/platform/terminal` it does not appear at all.
- WT starts the client with `EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT` and
  `bInheritHandles=false` (`T` `ConptyConnection.cpp:166-176`). node-pty uses the same flags
  (`N` `conpty.cc:433-444`).
- Neither uses `CREATE_BREAKAWAY_FROM_JOB` or `CREATE_NEW_PROCESS_GROUP`. A shell and its
  descendants therefore inherit whatever job, if any, the host process itself is in (Learn
  [Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects):
  children "are also associated with the job" by default).

**Classic conhost (V).** conhost/OpenConsole production source creates no job (same search).
`winconpty` starts OpenConsole with `CreateProcessAsUserW(…, EXTENDED_STARTUPINFO_PRESENT, …)`
(`T` `winconpty.cpp:259-270`).

**Job context imposed from outside (U).**
- **Windows Terminal** is an MSIX packaged desktop app. Learn says a packaged app's child
  processes run "inside the container" ([Windows app restore](https://learn.microsoft.com/en-us/windows/apps/develop/windows-app-restore),
  [MSIX AppContainer apps](https://learn.microsoft.com/en-us/windows/msix/msix-container)). No
  Microsoft source found states whether that container is a job, or what its breakaway limits
  are.
- **VS Code** runs node-pty in an Electron `utilityProcess` "ptyHost"
  (`C` `electronPtyHostStarter.ts:59`, `utilityProcess.ts:261`). Whether Electron/Chromium puts
  that process in a job is not stated in VS Code's source.
- **Probe for both:** from a shell in each real host, run a tiny binary that calls
  `IsProcessInJob(GetCurrentProcess(), NULL, &b)`. If `b`, it then calls
  `QueryInformationJobObject(NULL, JobObjectExtendedLimitInformation, …)`, which with NULL
  queries "the immediate job of the calling process"
  ([Learn](https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-queryinformationjobobject)).
  Record `LimitFlags`: `KILL_ON_JOB_CLOSE`, `BREAKAWAY_OK`, `SILENT_BREAKAWAY_OK`. Also record
  whether `CreateProcess(CREATE_BREAKAWAY_FROM_JOB)` succeeds and whether a detached child
  survives closing the tab, the window and the app.

**Breakaway rules that bound the design (V, Learn).**
- `CREATE_BREAKAWAY_FROM_JOB` "has no effect" if the caller is not in a job. If the caller is in
  a job, "the job must set the `JOB_OBJECT_LIMIT_BREAKAWAY_OK` limit"
  ([Process Creation Flags](https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags)).
- With `SILENT_BREAKAWAY_OK`, children escape automatically.
- With neither limit, no breakaway is possible and `AssignProcessToJobObject` to another job
  fails.
- Nested jobs exist since Windows 8. With `KILL_ON_JOB_CLOSE` on a nested job, closing its last
  handle "terminates all processes associated with the job and its child jobs"
  ([Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)).
- These rules underpin the map's contract (a detached daemon must break away, otherwise `exec`
  exits 125) and the per-session job: a session job nested under a host job is supported.

**GitHub runner (V for the runner agent).** `actions/runner` has no job-object code. It cleans up
orphans by marking each job with a `RUNNER_TRACKING_ID` environment variable and killing every
new process carrying it when the job ends (`R` [`JobExtension.cs:588-600,891-935`](https://github.com/actions/runner/blob/8cfa1049f86781d141056082d73802d56cd2b433/src/Runner.Worker/JobExtension.cs#L888-L935)).
Whether the hosted VM's launch chain places the runner in a job is **U**; probe it with the same
`IsProcessInJob` call as the first test step.

## 4. What the client sees when the host closes

**Windows Terminal tab or pane close (V).**
1. `ConptyConnection::Close()` resets `_hPC` "to signal ConPTY to send out a CTRL_CLOSE_EVENT to
   all attached clients", then waits for its output thread
   (`T` [`ConptyConnection.cpp:667-702`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/cascadia/TerminalConnection/ConptyConnection.cpp#L667-L702)).
   WT does **not** TerminateProcess the shell.
2. `ConptyClosePseudoConsole` only closes the signal, reference and process handles
   (`T` `winconpty.cpp:395-433`).
3. OpenConsole sees the broken signal pipe (or broken input pipe) and calls `SendCloseEvent` once,
   which calls `CloseConsoleProcessState`, which sets `CTRL_CLOSE_EVENT`
   (`T` [`PtySignalInputThread.cpp:322-326`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/host/PtySignalInputThread.cpp#L322-L326),
   [`VtIo.cpp:300-322`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/host/VtIo.cpp#L300-L322),
   [`output.cpp:459-474`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/host/output.cpp#L459-L474)).
4. `ProcessCtrlEvents` calls `EndTask(pid, CTRL_CLOSE_EVENT, …)` for every attached process. A
   comment there records that the old abort-on-failure loop "was removed around the Windows 11
   26100 time frame, because CSRSS (which handles EndTask) now waits 5s and then force-kills the
   process for us" (`T` [`input.cpp:335-364`](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/src/host/input.cpp#L335-L364)).
5. Learn ([HandlerRoutine](https://learn.microsoft.com/en-us/windows/console/handlerroutine)) says:
   - The timeout for `CTRL_CLOSE_EVENT` is `SPI_GETHUNGAPPTIMEOUT`, 5000 ms.
   - The handler runs on a new thread.
   - The handler may "Call the ExitProcess function". If it returns TRUE or FALSE, "the system
     terminates the process".
   - So a client can choose its own exit code, such as 129, from inside the handler, within
     ~5 s (**I**).
   - The exit code when a process is killed without a handler is **U** in public docs.
     `0xC000013A` is widely observed but not documented on that page.

**VS Code panel close (V).**
1. `shutdown()` queues a process exit and calls `_kill()` after `DataFlushTimeout = 250` ms, with
   a hard cap of `MaximumShutdownTime = 5000` ms. Windows never takes the `immediate` path
   (`C` [`terminalProcess.ts:27-44,472-492`](https://github.com/microsoft/vscode/blob/ff9acdc74d3fbc338a6e50da2c31695eb0176a33/src/vs/platform/terminal/node/terminalProcess.ts#L472-L492)).
2. `_kill()` calls node-pty `kill()` (`:400-420`).
3. With `useConptyDll` (the default), node-pty:
   - destroys the input socket;
   - calls `ConptyClosePseudoConsole`;
   - calls **`TerminateProcess(hShell, 1)`** at once
     (`N` [`windowsPtyAgent.ts:206-234`](https://github.com/microsoft/node-pty/blob/35c28c7d68a192699e70b1e6a142908bdccd9212/src/windowsPtyAgent.ts#L206-L234),
     [`conpty.cc:620-664`](https://github.com/microsoft/node-pty/blob/35c28c7d68a192699e70b1e6a142908bdccd9212/src/win/conpty.cc#L620-L664)).

   The shell dies with exit code 1. Other attached processes go through the same OpenConsole path
   as in Windows Terminal: `CTRL_CLOSE_EVENT` and the 5 s CSRSS window (**I** from shared
   source).
4. Without `useConptyDll`, node-pty first runs a helper that calls `AttachConsole` and
   `GetConsoleProcessList`, then calls `process.kill(pid)` on **every** attached process
   (`N` `windowsPtyAgent.ts:212-224`, `conpty_console_list.cc`). In libuv, `SIGTERM` is
   `TerminateProcess(handle, 1)`
   (`UV` [`process.c:1377-1385`](https://github.com/libuv/libuv/blob/fc7475a4dd8144e3eda0bf103892d774d1b675e1/src/win/process.c#L1377-L1385)).
   In that mode a climon2 client gets **no** close event and exits with 1.
5. Window reload and app quit with persistent sessions follow different paths (ptyHost
   lifetime). They are **U** and belong on the manual checklist.

**Pipes (V/I).** After `ClosePseudoConsole`, OpenConsole exits once the last client disconnects,
and the host stops draining. A client's console writes after that point are moot, because the
client is about to be killed. The close event is the hang-up signal; pipe closure is not
observable to a console client (**I**).

## 5. Can `climon2-test-term` model each host on `windows-2025`?

**Today (V, repo).**
- The harness opens hosts through portable-pty and drives `alacritty_terminal` as the host
  emulator (`crates/climon2-test-term/src/terminal.rs`).
- Process ownership is defined for macOS and Linux only (`ownership.rs`).
- The shell profile is bash-only (`shell.rs`).
- CI has a single Windows job, `neutral`, which runs `cargo test -p climon2-core` on
  `windows-latest` (`.github/workflows/check.yml`).

**What portable-pty 0.9.0 can and cannot do (V, `PP`).**
- **Can:**
  - Load `conpty.dll` by bare name, so the DLL search order picks up a copy beside the test
    executable, and fall back to `kernel32`. A harness can therefore select the bundled
    OpenConsole by deploying `conpty.dll` and `OpenConsole.exe` next to the test binary.
- **Cannot:**
  - Choose the creation flags: they are hard-coded to 0x7, which matches neither host.
  - Close the console gracefully apart from `Drop`. `kill()` is `TerminateProcess(…, 1)`.
  - Use any job-object API.

**Modelling options (facts about feasibility, no choice made).**

| Host property | Model in harness? | How |
| --- | --- | --- |
| Bundled ConPTY build | Yes | Pin `Microsoft.Windows.Console.ConPTY` (WT-matching 1.25.x, and VS Code's 1.25.260303002) and call `ConptyCreatePseudoConsole` directly, through `windows-sys`/`LoadLibrary`, instead of portable-pty. |
| Creation flags | Yes | WT: 0x08 (plus 0x1 for restore); VS Code: 0. |
| Inherit-cursor handshake | Yes | `alacritty_terminal` already answers DSR when `Options::replies` is set. |
| Win32-input-mode | Partly | The harness emulator must recognise `CSI ?9001h` and encode keys as `CSI Vk;Sc;Uc;Kd;Cs;Rc_` (spec: `T` `doc/specs/#4999 - Improved keyboard handling in Conpty.md`) to model WT. Ignoring it models VS Code's default. Fidelity to WT's real key encoding remains manual. |
| Close sequence | Yes | WT: `ClosePseudoConsole` only. VS Code: wait 250 ms, close input, `ConptyClosePseudoConsole`, `TerminateProcess(shell,1)`. VS Code inbox mode: kill every PID from `GetConsoleProcessList`. CSRSS `EndTask` behaviour comes from the real OS. |
| Host job | Yes, once the probes in §3 give the real limits | Create a job (none, `KILL_ON_JOB_CLOSE`, with or without `BREAKAWAY_OK`/`SILENT_BREAKAWAY_OK`) and assign the shell before resuming it (`CREATE_SUSPENDED`). Nested jobs make the per-session job testable underneath. "Breakaway refused, so exit 125" is directly testable. |
| Test-process ownership | Yes | A test-owned job (or job tree) replaces the procfs/proc_info ownership model. `TerminateJobObject` provides cleanup. |
| Real WT/xterm.js rendering, key encoding, UI close paths (tab, pane, window, app quit, VS Code reload, persistent sessions, defterm) | No | Manual real-app checklist. |
| MSIX and Electron job context | Not until probed | Manual probe (§3); feed the result into the harness's job shapes. |

**Runner constraints (V/U).**
- The runner image does not list Windows Terminal or VS Code (`RI` `Windows2025-VS2026-Readme.md`),
  so real-app automation would need installation first, plus UI automation (WinAppDriver
  1.2.2009.02003 *is* listed, `:87`).
- Whether hosted runners run in an interactive desktop session is **U**. ConPTY itself is
  headless (`--headless`, `T` `winconpty.cpp:196`).

## 6. Shells and full-screen apps on CI runners

**Runner labels (V, `RI` [README.md:36-38](https://github.com/actions/runner-images/blob/95e6f51a068507e8f43d061d2166a39d0ef36ce8/README.md)).**
- `windows-latest`, `windows-2025` and `windows-2025-vs2026` run Windows Server 2025, x64.
- `windows-2022` runs Server 2022, x64.
- `windows-11-arm` and `windows-11-vs2026-arm` run Windows 11 Arm64 only. There is no x64
  Windows 11 hosted image.
- Image builds:
  - Server 2025 is OS `10.0.26100`, Build 33822 (image 20261111.331.1) or 33674 for the VS2026
    image.
  - Server 2022 is `10.0.20348`.
  - Windows 11 Arm64 is `10.0.26200`.

  (`RI` `Windows2025-Readme.md:10-11`, `Windows2025-VS2026-Readme.md`,
  `Windows2022-Readme.md:7-8`, `Windows11-Arm64-Readme.md:7-8`)

| Program | On `windows-2025`? | Controlled startup | Source |
| --- | --- | --- | --- |
| `pwsh` | Yes, PowerShell 7.6.x | `-NoProfile` (ADR 0003); also `-NoLogo` | `RI` `Windows2025-VS2026-Readme.md:454`; [about_Pwsh](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_pwsh) |
| `powershell` 5.1 | Part of Windows (not separately listed) | `-NoProfile` | [about_PowerShell_exe](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_powershell_exe) |
| `cmd.exe` | Part of Windows | `/D` (skip AutoRun); set the prompt with `PROMPT` | [cmd](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/cmd) |
| vim for Windows | **No.** Only Git for Windows' MSYS2 `usr\bin\vim` (Git 2.56.0.windows.4, `:67`), which is not a native console vim | `--clean` ≈ `-u DEFAULTS -U NONE -i NONE`; or `-u NONE -i NONE -N` | [vim `starting.txt:535`](https://github.com/vim/vim/blob/188a790b05a282926b311503245f5119a5f6f3ed/runtime/doc/starting.txt#L535); install from [`vim/vim-win32-installer`](https://github.com/vim/vim-win32-installer) release zips (`gvim_9.2.1345_x64.zip` latest), or Chocolatey (2.7.5 on image, `:29`) |
| Microsoft Edit | **No** (not listed; `winget` not listed either) | No profile flag. Settings are read from `%APPDATA%\Microsoft\Edit\settings.json`, and a missing file means defaults, so set `APPDATA` to an empty directory. CLI: `-h`/`--help`/`/?`, `-v`/`--version`, `-g`/`--goto` | `E` [`settings.rs:48-58,94-107`](https://github.com/microsoft/edit/blob/b7f83220868373945945e2218e852c24cbb4dcf0/crates/edit/src/bin/edit/settings.rs#L94-L107), [`main.rs:253-270`](https://github.com/microsoft/edit/blob/b7f83220868373945945e2218e852c24cbb4dcf0/crates/edit/src/bin/edit/main.rs#L253-L270); release `v2.0.0` asset `edit-2.0.0-x86_64-windows.zip` |

- Edit reads input with `ReadConsoleInputExW` and enables `ENABLE_VIRTUAL_TERMINAL_INPUT`
  (`E` `crates/edit/src/sys/windows.rs:100-240`). It therefore exercises both win32-input-mode
  decoding and VT input (**I**).
- Whether the zip contains the console `vim.exe` as well as `gvim.exe` is **U**. The README says
  only "an archive of each architecture"; list the zip to check.

## 7. `windows-2025` (Server) as a stand-in for Windows 11

- **Same base build, different servicing.**
  - Learn release health lists Server 2025 as build 26100 and Windows 11 24H2 as 26100
    ([Server](https://learn.microsoft.com/en-us/windows/release-health/windows-server-release-info),
    [Windows 11](https://learn.microsoft.com/en-us/windows/release-health/windows11-release-information)).
  - 25H2 (26200) and 26H2 (26300) are enablement packages on the 24H2 branch (Microsoft
    shared-servicing whitepaper v1.3, Windows IT Pro blog).
  - Whether Server 2025 receives the same inbox conhost/`kernel32` ConPTY payload at the same
    revision is **U**.
  - Risk: low for the hosts, because both bundle their own ConPTY (§1) and a harness that pins
    the bundled build is OS-independent for that layer (**I**). Risk remains for anything that
    uses inbox ConPTY: VS Code with the setting off, a daemon whose session PTY uses `kernel32`,
    or classic conhost.
- **CSRSS close timing.**
  - The 5 s `EndTask` force-kill is an OS (CSRSS) behaviour that the OpenConsole comment ties to
    "Windows 11 26100".
  - Server 2025 shares build 26100, so the behaviour should match (**I**). Assert it in CI
    rather than assume it.
- **Inbox Edit.**
  - Windows 11 25H2, and 24H2 with the September 2025 optional update or later, include Edit
    ([Learn](https://learn.microsoft.com/en-us/windows/edit/)).
  - The inbox copy is 1.2.1 in `System32`, and it shadows newer installs on `PATH`
    ([microsoft/edit#951](https://github.com/microsoft/edit/issues/951)).
  - Server images list no Edit at all.
  - A CI pass against Edit 2.0.0 does not prove behaviour against the 1.2.1 that `edit`
    resolves to on many Windows 11 machines.
- **Default terminal handoff.**
  - "Default terminal application" exists on all Windows 11 versions
    ([Learn](https://learn.microsoft.com/en-us/windows/terminal/install)). A console process that
    gets a *new* console (for example `CREATE_NEW_CONSOLE`) may be handed off to Windows
    Terminal through `CTerminalHandoff` rather than conhost.
  - Its availability and default on Server 2025 are **U**.
  - Any daemon-spawn path that allocates a console can therefore behave differently on Windows
    11 (a visible WT window). `CREATE_NO_WINDOW` and `DETACHED_PROCESS` avoid allocating a
    console ([Process Creation Flags](https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags)).
- **Session and desktop context.**
  - Hosted runners are VMs whose session type is **U**. Real Windows 11 hosts run in the user's
    interactive session.
  - Job context differs: on the runner it is probe-able (§3), and on the real hosts it is
    probe-able but unknown (§3).
- **Hosts absent.** Windows Terminal and VS Code are not on the image, so only the harness model
  runs in CI. Real-app evidence comes from the manual checklist on a real Windows 11 machine.
- **Architecture.** Windows 11 hosted runners are Arm64 only. The map targets x86_64, so they do
  not help.

## Open questions (all **U**, each with its probe)

1. Is Windows Terminal (MSIX) or VS Code's pty host in a job, and with which limits? Probe:
   `IsProcessInJob` + `QueryInformationJobObject(NULL, ExtendedLimit)` in each real app, plus a
   breakaway attempt.
2. Is the GitHub runner process tree in a job? Probe: the same call as the first CI step.
3. How does inbox `kernel32` ConPTY on 26100 treat flags 0x2/0x4? Probe: create with each flag
   on `windows-2025` and inspect `OpenConsole`/`conhost` command lines with
   `Get-CimInstance Win32_Process`.
4. What exit code does a process get when it is killed by CSRSS after `CTRL_CLOSE_EVENT`, with
   and without a handler? Probe in the harness on `windows-2025`.
5. What do VS Code reload, quit and persistent-session restore do to a running client?
   Manual checklist.
6. Does Windows Terminal's shipped `compatibility.textMeasurement` default equal `graphemes`?
   Probe: read `defaults.json` in a real install.
