# ConPTY and Windows console I/O

Research for [#206](https://github.com/jackgeek/climon2/issues/206), under
[map #205](https://github.com/jackgeek/climon2/issues/205). Checked **2026-11-14** against
climon2 `main` `fd25a71`. The ticket was rescoped while this was written: **Windows 11 only**
(Windows 10 is unsupported). This file therefore compares Windows 11 builds and GitHub's Windows
Server runners instead of a 1809 floor. It records facts and credible options only. It chooses no
policy, approves no exception to the contract, and implements nothing.

**Tags.** **V** = verified in a cited primary source: Microsoft Learn, or `microsoft/terminal`
source, commit messages or maintainer comments at a pinned commit. **I** = inference from V facts.
**U** = not established. Nothing was run on Windows.

**Pinned sources.**

- `microsoft/terminal` `main` at `f6de9e0618015def1c9dda5b82f9d3abf03eb799` (2026-11-11), cited as
  `T:path:line`.
- The last pre-rewrite `main`, `a8582978afa50ece88edc7eda9e182ced64876e2` (the parent of #17510),
  cited as `T-old:path:line`.
- Learn pages at the `git_commit_id` shown in each page's metadata on 2026-11-14.
- Short commit hashes below are `microsoft/terminal` commits (PR numbers in parentheses).

## Material findings (read first)

1. **There are two ConPTY implementations. Which one you get depends on who calls
   `CreatePseudoConsole`, not on the Windows 11 version alone.**
   - **Pre-rewrite "VtEngine" conhost.** It re-renders conhost's buffer and is not a passthrough.
   - **Post-rewrite conhost**, from `450eec4` (#17510, 2024-08-01, first shipped in Windows
     Terminal 1.22). It forwards the application's VT "unmodified" and translates Console API calls
     to VT synchronously.

   The inbox `conhost.exe` is synced with OpenConsole only "for every major OS swap"
   ([lhecker, 2025-10-20](https://github.com/microsoft/terminal/discussions/17608#discussioncomment-14728100)).
   Microsoft's stated path for ConPTY updates is a redistributable `conpty.dll` plus
   `OpenConsole.exe` (NuGet `Microsoft.Windows.Console.ConPTY`) that callers pin
   ([DHowett, 2024-06-26/27](https://github.com/microsoft/terminal/discussions/17608#discussioncomment-10142151)).
   **V** for both statements. **I:** no released inbox Windows 11 build is shown to contain #17510;
   24H2 (build 26100) was finalised before 2024-08 (**U** precisely).
2. **The client's host and the daemon's ConPTY can be different implementations.**
   - Windows Terminal ships OpenConsole. 1.22 and later contain #17510; `release-1.21` does not
     (**V**, ancestry check).
   - VS Code `main` defaults `terminal.integrated.windowsUseConptyDll` to `true`, using its bundled
     `conpty.dll` v1.25.260303002 (**V**, `microsoft/vscode`
     `ff9acdc7…:src/vs/platform/terminal/common/terminalPlatformConfiguration.ts:496-501`).
   - A daemon calling `kernel32!CreatePseudoConsole` gets the inbox (pre-rewrite, **I**) engine.
3. **On the pre-rewrite engine, ADR 0007's "daemon is the sole query authority" cannot hold.**
   - Conhost answers DA1, DSR/CPR, DECRQM and similar queries itself, from its own buffer
     (`T-old:src/host/outputStream.cpp:34-41`, `T-old:src/terminal/adapter/adaptDispatch.cpp:1554-1723`).
   - Titles are re-emitted as `OSC 0;…BEL`.
   - Output is a frame-based re-rendering of the console buffer.

   On the post-rewrite engine, conhost never answers queries under ConPTY: "ConPTY should not
   respond to requests. That's the job of the terminal." (`T:src/host/outputStream.cpp:54-58`).
   The daemon's virtual terminal would then receive and answer them.
4. **Every post-rewrite ConPTY sends `ESC[c ESC[?1004h ESC[?9001h` to its terminal at start, and
   `ESC[?1004l ESC[?9001l` at shutdown** (`T:src/host/VtIo.cpp:200-204`, `:257-260`). The
   pre-rewrite engine sends `ESC[?9001h ESC[?1004h` (`T-old:src/renderer/vt/state.cpp:537`).
   ADR 0007's live policy currently forwards "everything else", so these mode sets would reach the
   client's host unless the daemon filters them.
5. **The end-of-session API differs by build.**
   - `ReleasePseudoConsole` exists only from Windows 11 24H2 / Windows Server 2025 (build 26100).
   - Before 26100, `ClosePseudoConsole` "will wait indefinitely for the pseudoconsole to exit"
     unless the output pipe is drained or closed. From 24H2 it returns immediately.
   - **`windows-2022` runners (build 20348) have the blocking `ClosePseudoConsole` and no
     `ReleasePseudoConsole`.** `windows-2025` runners (26100) list `ReleasePseudoConsole` support.

   **V** ([Learn: ClosePseudoConsole](https://learn.microsoft.com/en-us/windows/console/closepseudoconsole),
   [Learn: ReleasePseudoConsole](https://learn.microsoft.com/en-us/windows/console/releasepseudoconsole),
   [runner-images](https://github.com/actions/runner-images/blob/95e6f51a068507e8f43d061d2166a39d0ef36ce8/images/windows/Windows2022-Readme.md)).
6. **`PSEUDOCONSOLE_PASSTHROUGH_MODE` is gone.**
   - It was the experimental flag `8u`, added by `2c2f4f9` (#11264, 2022-03-30, behind a hidden
     Windows Terminal setting) and removed by `2050416` (#16954, 2024-03-28).
   - It was never documented on Learn, which lists only `0` and `PSEUDOCONSOLE_INHERIT_CURSOR`.
   - The value `0x08` is now reused as `PSEUDOCONSOLE_GLYPH_WIDTH_GRAPHEMES` (`T:src/inc/conpty-static.h:27-38`).
   - #17510 is the replacement: recognised VT is forwarded unmodified without any flag.

## 1. Which implementation runs where

| Context | Build | ConPTY engine | `ReleasePseudoConsole` | `ClosePseudoConsole` |
| --- | --- | --- | --- | --- |
| Windows 11 21H2 / 22H2 / 23H2 | 22000 / 22621 / 22631 | inbox, pre-rewrite (**I**) | absent (**V**: min 26100) | blocks until conhost exits (**V**) |
| Windows 11 24H2 / 25H2 | 26100 / 26200 | inbox, pre-rewrite (**I**, **U** exact) | present (**V**) | returns immediately (**V** 24H2; **I** 25H2) |
| Windows 11 26H1 (new devices only) | 28000 | **U** | **I** present | **I** immediate |
| GitHub `windows-2022` | 10.0.20348 | inbox, pre-rewrite (**I**) | absent (**V**) | blocks (**I**: pre-26100) |
| GitHub `windows-2025` / `windows-latest` | 10.0.26100 | inbox, as 24H2 (**I**) | present (**V**: "Windows Server 2025 (build 26100)") | **I** immediate (Learn names only "Windows 11 24H2 (build 26100)") |
| Bundled `conpty.dll` + `OpenConsole.exe` (any build) | — | the pinned package version | `ConptyReleasePseudoConsole` (`T:src/winconpty/winconpty.cpp:558-578`) | closes handles and returns; no wait (`T:…winconpty.cpp:426-433`) |

Build numbers come from [Windows 11 release information](https://learn.microsoft.com/en-us/windows/release-health/windows11-release-information)
(`d3bdb40`). That page says 26H1 "is scoped to support new devices … not offered as an in-place
update from 24H2 or 25H2". The runner OS versions come from `actions/runner-images` `95e6f51`.

**`conpty.dll` mechanics (V).** It launches `OpenConsole.exe` from beside the DLL, or from an
architecture subfolder (`x64`). If neither exists, it falls back to inbox `conhost.exe`
(`T:src/winconpty/winconpty.cpp:52-97`). That fallback would silently reintroduce the inbox
engine. The package was introduced by `9611433` (#12980).

## 2. Output: what the daemon receives

### Post-rewrite engine (WT ≥ 1.22, VS Code bundled conpty.dll, any pinned package)

- `450eec4` (#17510) says: "any VT output that an application generates will now be given to the
  terminal unmodified". Console API calls are translated synchronously, for example
  `SetConsoleCursorPosition` → `CUP` (**V**, commit message).
- `WriteCharsVT` runs the app's string through conhost's own parser for its buffer, then writes
  the same chunk to the pipe (`T:src/host/_stream.cpp:391-463`). The exceptions are:
  - **LF→CRLF.** If the app has not set `DISABLE_NEWLINE_AUTO_RETURN`, every LF is rewritten to
    CRLF (`:411-421`; the #17510 message calls this a known quirk).
  - **Injections.** After `RIS`, conhost appends `ESC[?1004h ESC[?9001h`. After an app's
    `ESC[?1004l` it appends `ESC[?1004h`. After `ESC[?9001l` it appends `ESC[?9001h`. After LNM it
    stops translating newlines (`:429-455`; `T:src/terminal/adapter/adaptDispatch.cpp:1867-1904`).
    The app's own `ESC[?9001h/l` "should not bubble up" (comment at `:1895-1900`).
  - The writer is used only when the writing buffer is the active screen buffer or its alternate
    (`T:src/host/consoleInformation.cpp:134-141`). Writes to inactive buffers do not reach the pipe.
- **Queries (DA1/DA2, DSR/CPR, DECRQM, XTWINOPS, …)** pass through to the terminal. Conhost
  suppresses its own replies under ConPTY (`T:src/host/outputStream.cpp:50-58`). Kitty-keyboard
  set/query is ignored under ConPTY (`T:…adaptDispatch.cpp:2078-2125`).
- **Conhost-generated VT** is interleaved for Console API callers:
  - titles `OSC 0;…ST` (`T:src/host/VtIo.cpp:759-764`);
  - alternate screen `ESC[?1049h/l` for buffer switches (`:732-737`);
  - mouse `ESC[?1003;1006h/l` when a client toggles `ENABLE_MOUSE_INPUT` (`:716-721`;
    `T:src/host/getset.cpp:362-381`);
  - `DECAWM` for `ENABLE_WRAP_AT_EOL_OUTPUT`;
  - window visibility `CSI 1/2 t`;
  - SGR translated from legacy attributes, always prefixed by `SGR 0` (`:340-376`);
  - C0/C1 characters written through `WriteConsoleOutput` replaced by CP437 glyphs or `?`
    (`:399-431`).
- **Startup and shutdown** sequences are as in Material finding 4. With `PSEUDOCONSOLE_INHERIT_CURSOR`, a
  `DSR CPR` precedes DA1 and conhost waits up to 1000 ms for the DA1 reply (`T:src/host/VtIo.cpp:191-217`).
  Without that flag, it does not wait ("The PTY host may not be a terminal", `:209-210`).
  `1ac221a` (#17574) added the earlier 3 s timeout.
- Known unresolved quirks are listed in #17510: focus events are injected without checking the
  ground state, `ScrollConsoleScreenBuffer` loses extended attributes, and a delayed EOL wrap
  breaks reflow (**V**, commit message).
- **Synchronized output** `ESC[?2026h/l` is tracked by conhost (`T:…adaptDispatch.cpp:1884-1890`)
  and forwarded with the rest of the string (**I** from `_stream.cpp` above).
- **Win32 console applications** such as `cmd.exe`'s cooked prompt and PowerShell 5.1's
  `WriteConsoleOutput` paths produce conhost-generated VT, not their own. That VT is still a
  deterministic, ordered translation (**I**).

### Pre-rewrite engine (inbox, both runner images)

- Output is a renderer: "VtEngine worked like a renderer after all" (`450eec4` message).
  `2050416` (#16954) lists "out-of-sync issues after resize".
- Recognised sequences update conhost's buffer and are re-painted. The painter hides the cursor
  around frames (`T-old:src/renderer/vt/XtermEngine.cpp:96-115`) and emits `ESC[2J`/`ESC[3J`,
  `OSC 0;…BEL` titles (`T-old:src/renderer/vt/VtSequences.cpp:109-114,347-350`) and
  `ESC[?1049h/l` on buffer switches (`:485-488`).
- Sequences whose dispatch returns false are flushed raw to the terminal ("If we were unable to
  process the string … flush the string to the terminal",
  `T-old:src/terminal/parser/OutputStateMachineEngine.cpp:337-343, 695-701, 931-937`). In pty mode
  these include:
  - OSC 52 ("forward the operation to the hosting terminal", `T-old:…adaptDispatch.cpp:3518-3528`);
  - bracketed paste `?2004` (`:2041-2042`);
  - cursor-blink `?12` and `DECSCNM`;
  - input modes (`DECCKM`, mouse modes, focus) when the client is in VT-input mode (`:1908-1911, 1934-2036`);
  - `?2026`, which this revision does not know.
- BEL is passed through as well (`T-old:…OutputStateMachineEngine.cpp:57-64`).
- **Ordering (I):** flushed raw sequences are written immediately, while printable text reaches
  the pipe only when the renderer paints. Their order relative to the text is therefore not
  guaranteed.
- **Queries are answered by conhost** (`T-old:src/host/outputStream.cpp:34-41`): DA1 returns
  `?61;6;7;14;21;22;23;24;28;32;42c` under pty (`T-old:…adaptDispatch.cpp:1554-1556`), and DSR/CPR
  is answered from conhost's cursor.
- `PSEUDOCONSOLE_INHERIT_CURSOR` blocks input until the CPR reply on Learn's description ("the
  conpty will not process any input until it does", `T:src/winconpty/winconpty.cpp:446-460`).
- **Exact inbox behaviour per build** is **U**. `T-old` is the last open-source state before the
  rewrite. Inbox builds are earlier snapshots plus servicing fixes.

### Consequences for ADR 0007 (I)

- **Post-rewrite engine.** The daemon's `climon2-vt` sees the command's VT nearly byte-for-byte.
  It must answer DA1 (at ConPTY start and for the command) and handle the `?1004`/`?9001` mode sets
  that ConPTY itself requests. Those mode sets are requests to the daemon, not output for the host.
- **Pre-rewrite engine.** The daemon receives a re-rendering:
  - its virtual terminal is a mirror of conhost's buffer;
  - query authority is split, with conhost answering first;
  - titles, alt-screen and clears are synthesised;
  - restoration (ADR 0007 phase A–C) is built on conhost's paint, not the command's bytes.

## 3. Input

### What arrives at ConPTY's input pipe and how the command sees it (V unless tagged)

Conhost parses the input pipe with `InputStateMachineEngine` (`T:src/host/VtInputThread.cpp:51-162`).

- **Plain VT keys** (`ESC[A`, C0 controls, text) become synthesised `KEY_EVENT` records. This
  applies when the command is not in VT-input mode (`T:src/terminal/parser/InputStateMachineEngine.cpp:148-226, 446-466`).
  If the command set `ENABLE_VIRTUAL_TERMINAL_INPUT`, cursor/function/generic CSI keys and SGR
  mouse "return false" and are written raw to its input buffer (`:383-466`; `ActionPassThroughString`
  → `WriteStringRaw`, `:296-303`).
- **`0x03` becomes Ctrl+C.** It is synthesised as VK `'C'` with `LEFT_CTRL_PRESSED` and sent
  through `WriteCtrlKey` (`:150-157`). If the command's input mode has `ENABLE_PROCESSED_INPUT`, this
  raises `CTRL_C_EVENT` (`T:src/host/input.cpp:114-124`).
- **Ctrl+Break** has no VT encoding. It is only reachable through a win32-input-mode record with
  `VK_CANCEL` (`T:src/host/input.cpp:133-137`; spec #4999 lists "ctrl+break is not ctrl+c").
- **win32-input-mode** `CSI Vk;Sc;Uc;Kd;Cs;Rc _` is decoded into a full `KEY_EVENT_RECORD` (`:511-519`).
  The format is defined in [spec #4999](https://github.com/microsoft/terminal/blob/f6de9e0618015def1c9dda5b82f9d3abf03eb799/doc/specs/%234999%20-%20Improved%20keyboard%20handling%20in%20Conpty.md).
  It is the only way to send Shift+Enter, Ctrl+Space, lone modifiers and similar keys (spec
  "Considerations").
- **Bracketed paste.** `ESC[200~`/`ESC[201~` are passed raw to VT-input commands. For non-VT
  commands the unknown generic key is dropped (`_GetGenericVkey` fails → nothing written,
  `:452-460, 1000-1012`), and the pasted text arrives as key events (**I**).
- **Replies to the command's queries** (for example the daemon's DA1/CPR answers) that are not
  ConPTY's own startup DA1 or captured CPR go through the unhandled path. They are written raw into
  the input buffer (**I** from `:416-430, 482-509`).

**Options for what the daemon writes.**

- **(a)** Forward the client's bytes as plain VT, which is lossy for keys without VT encodings.
- **(b)** Encode win32-input-mode sequences. ConPTY requests this with `?9001h`; doing it needs
  full key records from the client.

### What the client receives from its own console (it is a console app under WT, VS Code or conhost)

- `ENABLE_VIRTUAL_TERMINAL_INPUT` makes conhost encode user input as VT for
  `ReadFile`/`ReadConsole` ([Learn: SetConsoleMode](https://learn.microsoft.com/en-us/windows/console/setconsolemode), `425fa47`).
  Under ConPTY, WT sends win32-input-mode records to conhost. Conhost re-encodes each key with
  `TerminalInput::HandleKey` for VT-input clients (`T:src/host/inputBuffer.cpp:700-713`).
- **Ctrl+] → 0x1D, Ctrl+T → 0x14.** These come from `_makeCtrlChar` (`ch & 0x1F` for `@`..`~`,
  `T:src/terminal/input/terminalInput.cpp:497-520`). The base character is computed from the
  keyboard layout when `UnicodeChar` is 0 (`:1286-1316`), so on layouts where `]` is AltGr or
  elsewhere the result is layout-dependent (**I**).
- **Ctrl+C reaches the client as 0x03 only if its input mode has `ENABLE_PROCESSED_INPUT` off.**
  Otherwise "CTRL+C is processed by the system and is not placed in the input buffer" (Learn:
  SetConsoleMode).
- **Option.** A client could write `ESC[?9001h` to its own console. Conhost then sets win32 mode
  in its `TerminalInput` and does not bubble the request up (`T:…adaptDispatch.cpp:1893-1904`).
  The client would receive full-fidelity `CSI …_` key records to forward unchanged to the daemon's
  ConPTY. **I**: this works only on post-rewrite or late-pre-rewrite conhost. Inbox behaviour is **U**.
- **Alternative.** Read `INPUT_RECORD`s directly with `ReadConsoleInputW` (no VT-input mode) and
  encode them. `ENABLE_WINDOW_INPUT` and `ENABLE_MOUSE_INPUT` events are only visible through
  `ReadConsoleInput`; `ReadFile`/`ReadConsole` filter them (Learn: SetConsoleMode, Remarks).

## 4. Client raw mode and exact restoration (V)

- Input-mode bits: `ENABLE_PROCESSED_INPUT 0x1`, `ENABLE_LINE_INPUT 0x2`, `ENABLE_ECHO_INPUT 0x4`,
  `ENABLE_WINDOW_INPUT 0x8`, `ENABLE_MOUSE_INPUT 0x10`, `ENABLE_INSERT_MODE 0x20`,
  `ENABLE_QUICK_EDIT_MODE 0x40` (requires `ENABLE_EXTENDED_FLAGS`),
  `ENABLE_VIRTUAL_TERMINAL_INPUT 0x200`.
- Output-mode bits: `ENABLE_PROCESSED_OUTPUT 0x1`, `ENABLE_WRAP_AT_EOL_OUTPUT 0x2`,
  `ENABLE_VIRTUAL_TERMINAL_PROCESSING 0x4`, `DISABLE_NEWLINE_AUTO_RETURN 0x8` (Learn: SetConsoleMode).
- **Raw-mode recipe (I):**
  - **Input:** clear `PROCESSED|LINE|ECHO`, set `VIRTUAL_TERMINAL_INPUT`. Add `WINDOW_INPUT` if
    resize records are wanted, which forces `ReadConsoleInputW` instead of `ReadFile`.
  - **Output:** set `PROCESSED_OUTPUT|VIRTUAL_TERMINAL_PROCESSING|DISABLE_NEWLINE_AUTO_RETURN`.
    Without `DISABLE_NEWLINE_AUTO_RETURN`, post-rewrite ConPTY rewrites LF→CRLF in the forwarded
    bytes (§2).
- **Modes live in the console, not the process.**
  - `InputMode` is a field of the shared `InputBuffer`, and `OutputMode` belongs to each screen
    buffer (`T:src/host/inputBuffer.hpp:23`, `T:src/host/getset.cpp:386, 425-431`).
  - Learn: "Setting the output modes of one screen buffer does not affect the output modes of
    other screen buffers."
  - **I:** a crashed or killed client leaves the console in raw mode for the shell that follows.
    The restore is the client's job.
- **Exact restore.**
  - `GetConsoleMode` on input adds `ENABLE_EXTENDED_FLAGS` plus the insert, quick-edit and
    auto-position bits when private flags are in use (`T:src/host/getset.cpp:37-56`).
  - Writing that value back with `SetConsoleMode` restores them (`:340-361`).
  - Conhost stores the mode **before** validating it, then returns `E_INVALIDARG` for ECHO without
    LINE: "we need to set the modes and then return the error codes" (`:386-400`).
  - **I:** a restore that reports failure may still have applied, so failure is not proof that
    nothing changed.
- Output mode is set on the *active* buffer (`context.GetActiveBuffer()`, `:424-431`).
- **Classic conhost window (unqualified host).** The client's console is a real conhost window, not
  ConPTY. VT output is rendered by conhost itself and there is no forwarding (**I**).

## 5. Resize

- **Daemon side.** `ResizePseudoConsole` writes a `PTY_SIGNAL_RESIZE_WINDOW` packet to the signal
  pipe (`T:src/winconpty/winconpty.cpp:282-299`). If no client has connected yet, conhost stashes
  the size (`T:src/host/PtySignalInputThread.cpp:171-187`).
  [Learn: ResizePseudoConsole](https://learn.microsoft.com/en-us/windows/console/resizepseudoconsole)
  says it adjusts the internal buffers so Console API callers see the right dimensions.
  - On the pre-rewrite engine, a resize triggers a repaint ("out-of-sync issues after resize",
    #16954).
  - On the post-rewrite engine, reflow uses the app's original bytes. The #17510 quirk "`\r\n` …
    breaks text reflow on window resize" applies (**V**, commit message).
- **Client side (SIGWINCH replacement).** With `ENABLE_WINDOW_INPUT`, conhost queues a
  `WINDOW_BUFFER_SIZE_EVENT` (`T:src/host/output.cpp:262-279`;
  [Learn: WINDOW_BUFFER_SIZE_RECORD](https://learn.microsoft.com/en-us/windows/console/window-buffer-size-record-str)).
  - When the viewport changes for a VT-input-mode client on the active buffer, conhost raises it
    "as the closest alternative" to SIGWINCH. "With ASB or in ConPTY, viewport will match buffer
    size" (`T:src/host/screenInfo.cpp:1180-1191`).
  - Consecutive size events are coalesced (`T:src/host/inputBuffer.cpp:762-766`).
  - The record carries the **buffer** size (Learn). Under ConPTY that equals the window (**V**,
    comment above). In a classic conhost window with scrollback, the buffer height is not the
    window height. Use `GetConsoleScreenBufferInfo.srWindow` there (**I**).

## 6. End of session

- **`ClosePseudoConsole`** "will send `CTRL_CLOSE_EVENT` to each client application that is still
  connected. Until the applications have disconnected they may continue writing more output." It
  is non-blocking from 24H2 (build 26100); earlier versions "wait indefinitely". "Don't call
  `ClosePseudoConsole` on the same thread that you're reading the output pipe from" (**V**, Learn).
- [Learn: Creating a Pseudoconsole session](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session)
  (`c303556`, Learn source commit) adds: "Any attached client character-mode applications … will be terminated", and
  closing "may emit a final frame update to `hOutput`".
- **CTRL_CLOSE_EVENT.**
  - The timeout is `SPI_GETHUNGAPPTIMEOUT`, 5000 ms. CTRL_C and CTRL_BREAK have no timeout.
  - Whatever a handler returns (TRUE or FALSE) for CTRL_CLOSE_EVENT, the process is then
    terminated.

  ([Learn: HandlerRoutine](https://learn.microsoft.com/en-us/windows/console/handlerroutine), `0b1b2c4`;
  also `T:doc/ConsoleCtrlEvent.md`.)
  - In OSS conhost, events go to clients newest-to-oldest, and to clients that are already dead, as
    restored by `09d8ac4` (#18233, 2024-11-25). Inbox ordering per build is **U**.
  - `b6b1ff8` (#14282) stopped ConPTY conhost from exiting early, which had left clients running.
    This matters for ADR 0003's "ClosePseudoConsole, then 5 s, then TerminateProcess".
- **Daemon-side EOF (the `EIO` replacement).**
  - The `HPCON` holds a `\Reference` handle that "keeps conhost running".
  - With `ReleasePseudoConsole` (26100+, or `ConptyReleasePseudoConsole` in the package), conhost
    exits when the last client disconnects. `ReadFile` on the output pipe then fails with
    `ERROR_BROKEN_PIPE`. Learn says to "read from or write to your output and input pipe handles
    until they return a failure" (`T:src/winconpty/winconpty.cpp:558-561`; Learn:
    ReleasePseudoConsole). Windows Terminal calls it right after launch (`T:src/cascadia/TerminalConnection/ConptyConnection.cpp:451`).
  - Without it (21H2–23H2, `windows-2022`), Learn states the problem directly. Testing the first
    process for exit "is insufficient, as it may have spawned additional processes". `ClosePseudoConsole`
    "would also wait until all clients had disconnected".
  - **I:** the pre-26100 pattern is to wait on the root process handle, call `ClosePseudoConsole`
    on a non-reader thread, and keep draining until `ERROR_BROKEN_PIPE`.
- **Pipes.**
  - Learn says both pipe handles are "restricted to synchronous I/O", and asks for one thread per
    channel to avoid deadlocks.
  - Post-rewrite conhost accepts overlapped handles (`T:src/host/VtIo.cpp:97-105`;
    `T:src/host/VtInputThread.cpp:66-76`), and Windows Terminal passes one overlapped duplex pipe
    as both input and output (`T:…ConptyConnection.cpp:411-413`).
  - Conhost also treats a 0-byte read as closure and names winsock handles as compatible
    (`T:src/host/VtInputThread.cpp:142-149`). **I:** this is incidental, not a contract.
- **Startup failure.** Closing the session while the child is still connecting can show a
  `0xc0000142` dialog (Learn: Creating a Pseudoconsole session). `733a5e7` (#18588) fixed shutdown
  during the startup DA1 wait in OSS.

## 7. Options surfaced (no choice made)

**O1. Engine source for the daemon.**

- *(a)* Inbox `kernel32` ConPTY: a per-build matrix, pre-rewrite rendering on every target
  including both runners, and split query authority.
- *(b)* Ship a pinned `conpty.dll` + `OpenConsole.exe` (MIT, NuGet package) next to the
  executables: one passthrough engine on every Windows 11 build and both runners, with
  `ConptyReleasePseudoConsole` everywhere. This adds artifacts to the release and installer, and to
  ADR 0002/0004 and `docs/runtime-artifacts.md`.
- *(c)* Choose per build.

The test harness's "host models" would mirror whichever is chosen.

**O2. Daemon handling of ConPTY's own negotiation** (`DA1`, `?1004h`, `?9001h`, and the
`CSI I/O` focus reports it expects):

- answer it and strip it in the front filter; or
- forward it.

**O3. Input encoding into ConPTY:** plain VT from the client, or win32-input-mode. If
win32-input-mode, the client reads records itself, either via `ReadConsoleInputW` or by enabling
`?9001h` on its own console.

**O4. End-of-output detection:**

- `ReleasePseudoConsole` + broken pipe (needs 26100+ or the package); or
- root-process wait + `ClosePseudoConsole` on a separate thread.

For the `windows-2022` runner, choose between keeping it and its blocking-close path in CI, or
qualifying on `windows-2025` only.

**O5. Client resize source:** `WINDOW_BUFFER_SIZE_EVENT` via `ReadConsoleInputW` (requires a
record-reading input loop), or polling `GetConsoleScreenBufferInfo`.

## 8. Not established (U)

- The exact inbox conhost revision in each Windows 11 build (22000–28000) and on Server 2022/2025.
  Whether any serviced inbox build contains #17510, #18233 or the win32-input-mode fixes from
  2023-12.
- Whether inbox `kernel32!CreatePseudoConsole` on any build honours the historical flags `2u`
  (resize quirk), `4u` (`PSEUDOCONSOLE_WIN32_INPUT_MODE`, `f327618`/#6309) or `8u`
  (passthrough). The current header defines none of them.
- Whether `ClosePseudoConsole` on Server 2025 is non-blocking. Learn names only Windows 11 24H2,
  though `ReleasePseudoConsole` names Server 2025 build 26100.
- ConPTY behaviour inside GitHub's hosted-runner session: whether a console or desktop exists for
  `CreatePseudoConsole` ("desktop apps only"). Runner steps' stdio are not consoles, so client
  raw-mode tests need a harness-created ConPTY or console.
- The pre-rewrite renderer's frame cadence, and the exact ordering of flushed raw sequences
  relative to painted text.
