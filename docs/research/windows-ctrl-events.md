# Ctrl+C and Ctrl+Break through the daemon's ConPTY

Research question: can the user's Ctrl+C and Ctrl+Break reach a session's command with native
Windows semantics? The setup is fixed by ADR 0003 ("Windows I/O and transport"). The client reads its
own console with `ReadConsoleInputW`, with `ENABLE_PROCESSED_INPUT` off and
`ENABLE_VIRTUAL_TERMINAL_INPUT` on, and forwards plain VT bytes. The daemon `WriteFile`s them into a
pseudoconsole hosted by a pinned, bundled `conpty.dll` + `OpenConsole.exe`. Checked 2026-11-19.
Nothing was run on Windows. This builds on `docs/research/windows-conpty.md` (branch
`research/windows-conpty`) and `docs/research/windows-lifecycle.md` (branch `research/windows-lifecycle`)
and does not repeat them.

**Tags.** **V** verified in a cited primary source. **I** inference from V facts. **U** not
established; each has a CI probe in [Probes](#probes). **S** secondary source, named.

**Sources.**

- `microsoft/terminal` tag `v1.25.3131.0`, commit `f88b209f1e57e1d8ca35977260c3cddacccaeb31`
  (2026-11-09), cited as `T:path:line`. Links: `https://github.com/microsoft/terminal/blob/f88b209f1e57e1d8ca35977260c3cddacccaeb31/<path>#L<line>`.
  **I:** this matches the latest stable NuGet `Microsoft.Windows.Console.ConPTY` `1.25.261109001`
  (published the same day; the nuspec names no commit). climon2 has not chosen a version yet.
- Microsoft Learn console pages from `MicrosoftDocs/Console-Docs` commit
  `8cf64f2ce92961421c4fb95f08ce3ee932893028`, cited as `L:file`
  (`https://github.com/MicrosoftDocs/Console-Docs/blob/8cf64f2ce92961421c4fb95f08ce3ee932893028/docs/<file>`).
- `microsoft/node-pty` commit `35c28c7d68a192699e70b1e6a142908bdccd9212`, cited as `N:path`.

## Material findings (read first)

1. **Ctrl+C works natively with no change.** Byte `0x03` written to ConPTY input becomes a
   synthesised Ctrl+`C` key that goes through the same routine as a physical keypress. If the
   command's console input mode has `ENABLE_PROCESSED_INPUT`, every process attached to the
   pseudoconsole gets `CTRL_C_EVENT` and pending reads are cancelled. If it is off (raw-mode apps),
   the key is placed in the input buffer, so a VT-input reader gets `0x03` back (V, §1).
2. **The only conditions are the command's own.** A command started with `CREATE_NEW_PROCESS_GROUP`,
   or one that inherited `SetConsoleCtrlHandler(NULL, TRUE)` from any ancestor (including the
   daemon), ignores Ctrl+C (V). The daemon must use neither (I).
3. **Ctrl+Break has an input encoding: a win32-input-mode record.** OpenConsole parses
   `CSI Vk;Sc;Uc;Kd;Cs;Rc _` on input **unconditionally**: no `?9001h` reply, acknowledgement or mode
   is needed. A key-down with `Vk=3` (`VK_CANCEL`) and a Ctrl flag, without Alt, raises
   `CTRL_BREAK_EVENT` for every attached process, whatever the input mode, and flushes the input
   buffer (V, §2). Example: `ESC [ 3 ; 70 ; 0 ; 1 ; 8 ; 1 _` (scan code is not consulted; 70 = 0x46
   is the Break scan code, I).
4. **That sequence has a lasting side effect.** The first win32-input-mode record sets a flag that is
   never cleared. From then on the input parser assumes a win32-input-mode terminal: a lone `ESC`
   (or `ESC`+one char) at the end of a read is no longer dispatched immediately but held as a
   possibly-split sequence, and `CSI 1;<mod>R` is treated as a cursor report rather than modified F3
   (V, §2). **I:** after one injected Ctrl+Break, a lone Esc keypress could sit unprocessed until the
   next input byte arrives. This makes plain injection risky unless the daemon also encodes lone Esc
   (and Alt+key) as records, or the effect is shown to be harmless (U, probe P3).
5. **The alternative, `AttachConsole` + `GenerateConsoleCtrlEvent`, works but is awkward in the
   daemon.** A detached process may attach to the command's console. node-pty does exactly this
   against a ConPTY-hosted shell, from a forked helper, because attachment is process-wide (V).
   With group 0 the caller receives the event too. Attaching resets the caller's handler table, so a
   handler must be registered after attaching or the default handler calls `ExitProcess` (V, §3).
   A short-lived helper process avoids all of it (I).
6. **In the client's console, Ctrl+Break is always a signal.** Conhost raises `CTRL_BREAK_EVENT` for
   `VK_CANCEL`+Ctrl regardless of `ENABLE_PROCESSED_INPUT`, does not store the key-down, and flushes
   the input buffer (V). The client therefore never reads Ctrl+Break; it can only observe it in a
   console control handler and must have one that returns TRUE, or it exits (V). The matching
   key-up record is stored, but with VT input on it is converted to no output (I, U P5).
7. **Parents sharing the client's console.** Keyboard Ctrl+Break goes to every process attached to
   the console, newest first, each on its own handler thread, regardless of what the others return
   (V). Launcher and CLI handlers that return TRUE survive it and do not block the client's handler.
   With processed input off (the mode is console-wide), keyboard Ctrl+C reaches no one as a signal (V).

**Proposed design consequence (I, no choice made).** Forward Ctrl+C as `0x03` (already the case).
For Ctrl+Break, the client's control handler sends a "break" request to the daemon, which either
injects a `VK_CANCEL` record (after P3 rules out or fixes the Esc side effect) or runs a helper that
attaches and calls `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0)`.

## 1. Ctrl+C: `0x03` into ConPTY input (V)

- The input engine maps a lone `0x03` to a key-down and key-up of VK `'C'`, `UnicodeChar 0x03`,
  `LEFT_CTRL_PRESSED`, sent through `WriteCtrlKey` "which is handled specially by the host"
  (`T:src/terminal/parser/InputStateMachineEngine.cpp:150-158`). `ESC 0x03` (Alt+Ctrl+C) is not
  this path (`:150`, `writeAlt`). This happens whether or not the command enabled VT input
  (`ActionExecute`, `:137-140`).
- `WriteCtrlKey` calls `HandleGenericKeyEvent`, the routine conhost uses for real keyboard input
  (`T:src/terminal/adapter/InteractDispatch.cpp:51-60`; `T:src/interactivity/win32/windowio.cpp:402`).
- `HandleGenericKeyEvent`: Ctrl down, Alt up, key down, VK `'C'` **and** processed input → set the
  Ctrl+C flag, terminate pending reads (`WaitTerminationReason::CtrlC`) and do not write the key
  (`T:src/host/input.cpp:114-131`). Otherwise the record is written to the input buffer (`:160-166`).
  `IsInProcessedInputMode` reads the single input buffer's mode, so it is console-wide (`:21-25`).
- The flag is dispatched in `ProcessCtrlEvents` on unlock (`T:src/host/handle.cpp:24`). A keyboard
  event has `LimitingProcessId` 0, so every attached process gets a termination record, newest to
  oldest, and `EndTask` is called for each (`T:src/host/input.cpp:265-363`,
  `T:src/server/ProcessList.cpp:204-226`). Learn: "these signals are passed to all console processes
  that are attached to the console … The system creates a new thread in each client process"
  (`L:ctrl-c-and-ctrl-break-signals.md`).
- Raw-mode command: the stored key record reaches a VT-input reader as `0x03` via
  `TerminalInput::HandleKey` (`T:src/host/inputBuffer.cpp:705-713`); a `ReadConsoleInput` reader sees
  the key record. Learn: with processed input "CTRL+C is processed by the system and is not placed in
  the input buffer" (`L:includes/console-mode-flags.md`).
- **Conditions on the command.**
  - `CREATE_NEW_PROCESS_GROUP`: "CTRL+C signals will be disabled for all processes within the new
    process group" ([Process Creation Flags](https://learn.microsoft.com/windows/win32/procthread/process-creation-flags),
    `git_commit_id 3aee48ac`).
  - `SetConsoleCtrlHandler(NULL, TRUE)`: "the calling process ignores CTRL+C signals … inherited by
    child processes" (`L:ctrl-c-and-ctrl-break-signals.md`, `L:setconsolectrlhandler.md`). node-pty
    calls `SetConsoleCtrlHandler(NULL, FALSE)` right after creating its pseudoconsole, "Restore
    default handling of ctrl+c" (`N:src/win/conpty.cc:295-296`). **U:** whether the flag survives a
    `DETACHED_PROCESS` hop and whether a console-less daemon can clear it (P4).
  - No pseudoconsole creation flag affects this path (I: none is read in `input.cpp` or the input
    engine).
- History: `0x03` → `WriteCtrlKey` existed in the initial source release (`d4d59fa3`, 2019-05-02)
  (`git log -S UNICODE_ETX`).

## 2. Ctrl+Break and win32-input-mode on ConPTY input (V)

- **No plain VT byte maps to Ctrl+Break.** Only `VK_CANCEL`+Ctrl in `HandleGenericKeyEvent` raises
  `CTRL_BREAK_EVENT` (`T:src/host/input.cpp:133-146`). The VT key paths synthesise only keys with VT
  encodings (prior research). Learn: "CTRL+BREAK is always treated as a signal"
  (`L:ctrl-c-and-ctrl-break-signals.md`).
- **The `VK_CANCEL` branch has no processed-input test.** It flushes the input buffer, sets the
  Ctrl+Break flag, terminates reads and drops the key-down (`input.cpp:133-146`). Delivery is the
  same all-attached-processes loop as §1.
- **win32-input-mode is parsed unconditionally.** `CsiActionCodes::Win32KeyboardInput` (`VTID("_")`,
  `T:src/terminal/parser/InputStateMachineEngine.hpp:99`) decodes the record and calls `WriteCtrlKey`
  "so … Ctrl+C, Ctrl+Break are handled correctly" (`InputStateMachineEngine.cpp:511-519`). No mode,
  reply or the command's VT-input setting gates it (`:378-385`, comment: "If the client was in VT
  input mode, but we received a win32-input-mode sequence, then _don't_ passthrough"). Field order
  and defaults: `Vk;Sc;Uc;Kd;Cs;Rc`, missing → 0, except `Rc` → 1 (`:1134-1155`).
- **ConPTY requests `?9001h` at start and `?9001l` at exit** (`T:src/host/VtIo.cpp:196-204`,
  `:257-260`), "If the terminal doesn't understand this sequence, it'll just ignore it". It waits
  only for a DA1 reply, only when inheriting the cursor, for at most 1000 ms (`:209-216`). Nothing
  depends on the terminal honouring `?9001` (I). The daemon stripping it is therefore harmless to
  ConPTY.
- **Lasting parser state.** `_encounteredWin32InputModeSequence` is set by the first record and never
  reset (`InputStateMachineEngine.cpp:518`; only other writer is the initialiser, `.hpp:193`). It
  changes two things:
  - `CSI …R` is no longer read as F3 (`:420-427`).
  - A trailing run of `ESC` or `ESC`+char is no longer flushed at the end of a read; it is cached as a
    split sequence (`T:src/terminal/parser/stateMachine.cpp:2048-2072`, added by `bc183489` #16466,
    2023-12-16). The input thread has no timer that flushes it (`T:src/host/VtInputThread.cpp:100-140`).
- **History.** Win32-input-mode parsing and the `?9001h` request arrived together in `f3276184`
  (#6309, 2020-06-08), first in Windows Terminal `v1.1.1671.0`, already unconditional and already
  routed through `WriteCtrlKey`. The `VK_CANCEL` branch is in the initial release `d4d59fa3`.

## 3. `AttachConsole` + `GenerateConsoleCtrlEvent` (V unless tagged)

- `AttachConsole(pid)` fails with `ERROR_ACCESS_DENIED` if the caller already has a console; "A
  process can be attached to at most one console" (`L:attachconsole.md`). A `DETACHED_PROCESS`
  daemon has none, so it may attach (I).
- **Prior art against ConPTY:** node-pty's `getConsoleProcessList` does `FreeConsole`,
  `AttachConsole(shellPid)`, `GetConsoleProcessList`, `FreeConsole` (`N:src/win/conpty_console_list.cc`),
  in a `child_process.fork` helper because "there can only be a single console attached to a
  process" (`N:src/conpty_console_list_agent.ts`). This shows attach works on a pseudoconsole (I,
  from shipped code).
- `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0)`: "the signal is generated in all processes that
  share the console of the calling process" (`L:generateconsolectrlevent.md`). The caller is one of
  them. Server side: the request sets `LimitingProcessId` and goes through the same
  `HandleCtrlEvent` as a keypress (`T:src/server/ApiDispatchersInternal.cpp:75-97`).
- With a group id, only that group gets it, and the id must be a `CREATE_NEW_PROCESS_GROUP` root
  (`L:generateconsolectrlevent.md`, `L:console-process-groups.md`). Making the command a group root
  disables its Ctrl+C (§1), so targeting the group costs native Ctrl+C (I). `CTRL_C_EVENT` "cannot be
  limited to a specific process group" (`L:generateconsolectrlevent.md`).
- Caveats:
  - "Calling AttachConsole, AllocConsole, or FreeConsole will reset the table of control handlers"
    (`L:setconsolectrlhandler.md`). Without a handler the default one calls `ExitProcess`
    (`L:handlerroutine.md`). Between attach and registration, any Ctrl+C/Break/Close in the session
    would kill the caller (I).
  - Console attachment and the handler table are process-wide; other daemon threads using console
    APIs or std handles would see the session's console while attached (I).
  - While attached the caller is a console client, so it also receives the session's keyboard
    Ctrl+C and `CTRL_CLOSE_EVENT` (I from §1).
  - **Helper variant (I):** the daemon spawns a tiny `DETACHED_PROCESS` helper that attaches,
    registers a TRUE-returning handler (or `SetConsoleCtrlHandler(NULL, TRUE)`, which does not mask
    Break), calls `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0)`, and exits. The daemon's own state
    is untouched; cost is a process spawn per Ctrl+Break.

## 4. Ctrl+Break in the client's own console (V)

- The client's console is the terminal's own conhost/OpenConsole. Windows Terminal sends it
  win32-input-mode records (`?9001h` honoured), so Ctrl+Break arrives as a `VK_CANCEL` record and
  takes the §2 branch: always a signal, input buffer flushed, key-down not stored
  (`T:src/host/input.cpp:133-146`). Learn: "always treated as a signal" (`L:setconsolectrlhandler.md`).
- The key-up record is stored (the branch requires `bKeyDown`, `:114-116`). With VT input on, stored
  key events pass through `TerminalInput::HandleKey` (`T:src/host/inputBuffer.cpp:705-713`), which
  emits nothing for a non-character key-up outside kitty/win32 modes (I, `T:src/terminal/input/terminalInput.cpp:216-280`).
  So `ReadConsoleInputW` should show no `VK_CANCEL` record at all (U, P5).
- What VS Code's terminal (xterm.js) sends for Ctrl+Break is not established (U, P6).

## 5. Launcher and CLI sharing the console (V)

- Every attached process gets its own handler thread, newest first; each process's handlers run "until
  one of the handlers returns TRUE", which stops only that process's chain (`L:handlerroutine.md`,
  `T:src/host/input.cpp:340-363`). The launcher/CLI returning TRUE neither suppresses nor delays the
  client's handler (I).
- Keyboard Ctrl+C with the client's processed input off is a key, not a signal, for everyone on that
  console (§1). Before raw mode is set, and after it is restored, Ctrl+C is a signal to all three; the
  parents' TRUE handlers keep them alive (I).
- Parents must use handler routines, not `SetConsoleCtrlHandler(NULL, TRUE)`, which children inherit
  (prior lifecycle research; `L:setconsolectrlhandler.md`). This matters for the session too if the
  client or CLI spawns the daemon (P4).

## Probes

Each runs on `windows-2025` (build 26100) with the pinned NuGet package.

- **P1 (confirms §1).** Daemon-style host: ConPTY from the bundled DLL, command = small program with a
  counting `CTRL_C_EVENT` handler and processed input on. Write `0x03`; assert one `CTRL_C_EVENT`.
  Repeat with processed input off and VT input on; assert `ReadFile` returns `0x03` and no event.
- **P2 (confirms §2).** Same host; write `ESC[3;70;0;1;8;1_`. Assert `CTRL_BREAK_EVENT` in the command
  for processed input on and off. Repeat with the command spawned with `CREATE_NEW_PROCESS_GROUP`;
  Ctrl+C is ignored, Ctrl+Break is not.
- **P3 (U, finding 4).** After P2's record, write a lone `0x1B` in its own `WriteFile`; record whether a
  VT-input command reads it within 500 ms, and whether a following `a` arrives as `ESC a` (Alt+a).
  Also write `ESC[1;5R` and record what the command reads.
- **P4 (U, §1).** Parent calls `SetConsoleCtrlHandler(NULL, TRUE)`, spawns a `DETACHED_PROCESS` child
  that spawns the ConPTY command; assert whether `0x03` still raises `CTRL_C_EVENT`. Repeat with the
  detached child calling `SetConsoleCtrlHandler(NULL, FALSE)` first; record its return value.
- **P5 (U, §4).** Model Windows Terminal: host a test client under the bundled ConPTY (WT itself is
  not available on runners; `WriteConsoleInputW` would bypass the key path). The client sets
  processed input off and VT input on and registers a handler. Write `ESC[3;70;0;1;8;1_` then
  `ESC[3;70;0;0;8;1_` (down, up); assert the handler fires and log every record `ReadConsoleInputW`
  returns.
- **P6 (U, §4).** Manual or Playwright check in VS Code's terminal: bytes and records received for
  Ctrl+Break. No runner can drive a real keyboard, so this one is likely manual.
- **P7 (§3).** Helper variant: detached helper attaches to the command's PID, registers a handler,
  calls `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0)`; assert the command and its child both get
  the event and the helper exits 0.
