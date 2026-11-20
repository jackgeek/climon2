# Windows lifecycle probes: results

Measurements for
[Probe Windows lifecycle facts on a Windows 11 VM (#300)](https://github.com/jackgeek/climon2/issues/300),
feeding the decision ticket
[Define Windows platform support and lifecycle contract (#212)](https://github.com/jackgeek/climon2/issues/212).
The probes, their numbering (H1 … Q2) and the thirteen decisions they test
come from `docs/agents/windows-probe-handoff.md` on branch
`handoff/windows-probes`. This file records what was seen. It decides nothing.

Terms used throughout:

- **Job**: a Windows job object, a kernel group of processes that can be
  limited and killed together. **Breakaway** is a new process leaving its
  parent's job, which `CREATE_BREAKAWAY_FROM_JOB` asks for and the job's
  `BREAKAWAY_OK` limit permits.
- **Session job**: the job the daemon would create for one session, with
  `BREAKAWAY_OK | KILL_ON_JOB_CLOSE`.
- **ConPTY**: the Windows pseudoconsole. **Bundled ConPTY** is `conpty.dll` plus
  `OpenConsole.exe` from the `Microsoft.Windows.Console.ConPTY` NuGet package,
  loaded by full path.
- **Detached host**: a probe process started the way decision 2 starts the
  daemon: `DETACHED_PROCESS`, standard handles on `NUL`, no console. Every
  ConPTY probe ran in one (subcommand `detach`).
- **Close event**: `CTRL_CLOSE_EVENT`, which a console sends to its attached
  processes when the console goes away.

Raw output for every probe is in `probes/windows-lifecycle/results/`, one JSON
object per line. Exit codes are hex, times are milliseconds.

## Surprises

Each item contradicts an expectation or a decision in the handoff.

1. **Decision 5, empty handle list (S1).** `UpdateProcThreadAttribute` rejects
   `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` with zero entries: error 24
   (`ERROR_BAD_LENGTH`), 3 of 3 runs. The single call from decision 5 cannot be
   built. `bInheritHandles = FALSE` with no handle list does what the empty list
   was meant to do. The command had 85 handles both before and after the parent
   opened ten extra inheritable handles. With inherit TRUE and no list it had
   96; with a one-entry list it had 86.
2. **Decision 5, standard handles (S1, `pty-smoke`).** Without
   `STARTF_USESTDHANDLES` and null standard handles, the command does not use
   the pseudoconsole for stdin and stdout. It takes the host's own handles
   instead: `NUL` in a detached host, the parent console when attached. `cmd.exe`
   then reads end-of-file and exits 0 within 2 s. With
   `STARTF_USESTDHANDLES` and three null handles, the session works. Every later
   ConPTY probe uses that.
3. **Decision 6, Store pwsh escapes the session job (`pty-where`, S3).**
   PowerShell 7.7.0 from the Microsoft Store (MSIX, reached through the
   `pwsh.exe` app-execution alias) sits in the session job itself. But every
   process it starts, foreground or `Start-Process -NoNewWindow`, is in **no
   job at all** (3 of 3 runs, plus a probe started from pwsh that reported
   `in_job: false`). Windows PowerShell 5.1 children stay in the session job.
   The escaped children still died with `0xC000013A` on close because they
   share the console. A detached child, or one that ignores close events,
   would outlive `TerminateJobObject` and `KILL_ON_JOB_CLOSE`. The MSI build of
   pwsh was not tested: winget reported it installed, but no
   `C:\Program Files\PowerShell\7` appeared.
4. **Decision 3, `Start-Process -Wait` holds its child in a job (H1).** Both
   Windows PowerShell 5.1 and pwsh 7.7.0 put a `Start-Process -Wait` child in a
   job with `LimitFlags = 0x0`: no `BREAKAWAY_OK`, no `KILL_ON_JOB_CLOSE`. The
   breakaway spawn fails with 5, so decision 3 would exit 125. This is a
   launcher, not a terminal host, and the job would not kill a daemon that
   stayed in it.
5. **Decision 9, close-event order depends on the host (X1).** In the bundled
   ConPTY, close events arrived newest-first (C at 0.3 ms, B at 1.6 ms, A at
   3.4 ms), so each waiter saw its child's 129. In a Windows Terminal 1.18 tab,
   they arrived **oldest-first, one process at a time**. A got the event first
   and waited its 3 s bound while B had not yet been told, so A timed out with
   129. Then B did the same, then C at about 6.04 s. All three ended with
   129 in every run, but in Windows Terminal only by timeout, never by passing
   the child's code up. Nothing was force-killed at 5 s, even though the chain
   took 6 s.
6. **Decision 8, the grace period was never used (S3).** In every shell, the
   leader and its background child exited within 5–30 ms of
   `ConptyClosePseudoConsole`, long before the 5 s `TerminateJobObject`. The
   leader's status was `0xC000013A` for `cmd`, `powershell`, `pwsh` and
   `edit`, and `0x1` for `vim`.
7. **C3, `ping` is the wrong target.** `ping -t` answers Ctrl+Break by printing
   `Control-Break` and carrying on, so "`ping` ends" could never hold. The
   attach helper does deliver the event. A process with the default handler
   ended with `0xC000013A`, and `cmd` survived and answered a later command.
   This was the same with the helper inside or outside the session job.
8. **J4, kill-on-close exit code.** A process killed by closing a
   `KILL_ON_JOB_CLOSE` job exits with code `0x0`, indistinguishable from a
   clean exit.
9. **S7, long working directories.** A working directory longer than 260
   characters fails with 267 (`ERROR_DIRECTORY`) whatever the long-path policy
   and with or without a `longPathAware` manifest. A nonexistent directory and
   a file give the same 267.
10. **C5, more than `?9001h`.** The bundled ConPTY's first output is
    `ESC[1t ESC[c ESC[?1004h ESC[?9001h`: a window-state query and a primary
    device-attributes request as well as the two mode sets. The session works
    with nothing answering any of them.
11. **H3, the records for Ctrl+Break and Ctrl+C.** Ctrl+Break produced no
    key-down but did produce a key-**up** record with `vk = 0x03` and character
    `0x03`. Ctrl+C and Esc key-downs arrive with `vk = 0`. A client that reads
    characters from key-up records, or keys on the virtual-key code, would
    misread both.
12. **H3, VS Code drops Ctrl+Break.** In a VS Code task terminal, Ctrl+Break
    produced nothing at all: no `CTRL_BREAK_EVENT` and no input record (3 of 3
    runs). Ctrl+C arrived as a single key-down, `vk 0`, character `0x03`, with
    no key-up. Two Esc presses produced one record. A user in VS Code cannot
    send Ctrl+Break to a session.

## Environment

| Item | Value |
| --- | --- |
| Machine | QEMU VM, "Standard PC (Q35 + ICH9, 2009)", hypervisor present |
| OS | Microsoft Windows 11 Pro, version 25H2, build 26200.9974 (`OSVersion` 10.0.26200.0, HAL 10.0.26100.1). Client, not Server. |
| UAC | **Off** (`EnableLUA = 0`). Every process, including Explorer's children, runs elevated at High integrity (`0x3000`). |
| Session | Interactive console session 1, desktop present |
| Windows Terminal | 1.18.10301.0, Store/MSIX package `Microsoft.WindowsTerminal_1.18.10301.0_x64__8wekyb3d8bbwe`. Each elevated `wt -w new` started its own `WindowsTerminal.exe` process. |
| PowerShell 7 | 7.7.0, Store/MSIX package `Microsoft.PowerShell` (installed by the developer during the session) |
| Windows PowerShell | 5.1 |
| VS Code | 1.146.0 (commit 69047191, x64), installed by winget during the session. For H1–H3 the developer turned off workspace trust and allowed automatic tasks in VS Code's user settings, and each window was started from this session with `Code.exe -n <folder>`. |
| Node | LTS from winget, installed during the session; used by the H1 npm script |
| vim | `C:\Program Files\Vim\vim.exe`, installed by winget during the session |
| edit.exe | `C:\WINDOWS\system32\edit.exe`, 1.2.1 |
| Rust | rustc 1.96.0 (ac68faa20 2026-05-25), host x86_64-pc-windows-msvc, LLVM 22.1.2 |
| Bundled ConPTY | `Microsoft.Windows.Console.ConPTY` 1.25.261109001 (latest stable on nuget.org) |
| `conpty.dll` x64 SHA-256 | `e971856f89748dc3b458125e9afdd79b5e9574a7c937676116d3cbbd3b71f492` |
| `OpenConsole.exe` x64 SHA-256 | `b17e48b07ef153b420f0a6e178bacd31fb4c3390e21ae746586213fc0719a775` |
| Key presses | Synthesised with `SendInput` (no human at the keyboard) |
| Connection | Claude Code running in a Windows Terminal tab on the VM; not SSH |
| Long-path policy | `LongPathsEnabled = 0`; set to 1 only during S7 and restored |

## How the probes ran

The probe program is `probes/windows-lifecycle/`, a crate outside the cargo
workspace using `windows-sys` 0.61. It was built with `cargo build --release`
and run as `C:\probes\bin\probe.exe`, never through cargo except in J3.
`probe-lpa.exe` is the same program with a `longPathAware` manifest, and
`needs-dll.exe` imports from a DLL that does not exist.

Every probe ran at least three times unless stated.

## Host jobs

### H1: do the hosts hold shells in a job?

Each context ran `spawn-test`: a self-report, then a child started with
`DETACHED_PROCESS`, adding `CREATE_BREAKAWAY_FROM_JOB` only when in a job.

| Context | In a job | Child spawn | Child in a job |
| --- | --- | --- | --- |
| Windows Terminal, `cmd` | no | ok | no |
| Windows Terminal, `powershell` | no | ok | no |
| Windows Terminal, `pwsh` (Store) | no | ok | no |
| Classic conhost window (`Start-Process conhost.exe cmd …`) | no | ok | no |
| This Claude Code session (WT tab → powershell → claude → cmd → powershell) | no | ok | no |
| `Start-Process -Wait` from Windows PowerShell 5.1 | yes, `LimitFlags 0x0` | **error 5** | n/a |
| `Start-Process -Wait` from pwsh 7.7.0 | yes, `LimitFlags 0x0` | **error 5** | n/a |
| VS Code integrated terminal, default profile (Store `pwsh`), 2 runs | no | ok | no |
| VS Code task, `cmd` shell | no | ok | no |
| VS Code task, `powershell` shell | no | ok | no |
| VS Code task, `pwsh` shell | no | ok | no |
| VS Code task running `npm run probe` (`cmd` → `node` → `cmd` → probe) | no | ok | no |

The VS Code workspaces are in `probes/windows-lifecycle/vscode/`. Each task
runs when its folder opens (`runOptions.runOn: folderOpen`), and each opening
counted as one run. In every VS Code row, the shell's parent was `Code.exe`.
That `Code.exe` was started from this session, which is in no job.

The integrated-terminal rows came from typing into the terminal with
`SendInput`. A third typed run, meant to start `cmd`, ran in `pwsh` instead, so
it counts as a second default-profile run. Typing was unreliable, so the tasks
cover the individual shells.

Package name: no context was packaged (`GetCurrentPackageFullName` returned
15700 `APPMODEL_ERROR_NO_PACKAGE`), including probes run under Windows
Terminal and Store pwsh. Integrity was High everywhere because UAC is off.
`QueryInformationJobObject(NULL, …)` from a process in no job returns error 5,
not an empty result.

My first "conhost" run (`conhost.exe cmd.exe …` started from this session's
redirected shell) had no window of its own, so the windowed `Start-Process`
runs above are the ones that count. That first run also showed no job.

*Expectation (no job, or a job the child leaves, in every terminal host):*
**held** for every terminal host tested. `Start-Process -Wait` is a job that
forbids breakaway (surprise 4).

### H2: does the detached child survive the host closing?

The H1 spawn started a child writing one heartbeat line a second. The Windows
Terminal window holding the tab was then closed in two ways, three runs each:

- `WM_CLOSE` to the window. That window was the only one in its own
  `WindowsTerminal.exe` process, so closing it closed the tab and ended the
  application.
- `Stop-Process -Force` on that `WindowsTerminal.exe`.

| Mode | Lines at close | +3 s | +30 s | WT process alive at +3 s |
| --- | --- | --- | --- | --- |
| `WM_CLOSE` (4 runs) | 3–4 | 6–7 | 33–34 | no |
| kill WT process (3 runs) | 4 | 7 | 34 | no |

One `WM_CLOSE` run missed its window and was repeated (`h2-wmclose-2b.txt`).
A tab could not be closed on its own without also closing its window,
because closing a single tab needs UI input.

In VS Code, the "H2 heartbeat" task ran the same spawn when its folder
opened. The window was then closed with `WM_CLOSE`, or every `Code.exe` was
killed with `Stop-Process -Force`, three runs each. VS Code asked nothing on
`WM_CLOSE` and exited.

| Mode | Lines at close | +3 s | +30 s | `Code.exe` alive at +3 s |
| --- | --- | --- | --- | --- |
| VS Code `WM_CLOSE` (3 runs) | 4 | 7–8 | 34 | no |
| VS Code, kill every `Code.exe` (3 runs) | 4 | 7 | 34 | no |

The first run of each mode counted lines at +2 s (6) and +32 s (36), not at
close and +3 s.

*Expectation (continues):* **held** for Windows Terminal and VS Code.

### H3: what does each host deliver for Ctrl+Break and Ctrl+C?

The `client` subcommand set the decision-10 mode (`0x1F7` → `0x3B8`, accepted
in every run), logged every console control event and every input record.
Keys were synthesised with `SendInput`. Before each key, the probe checked that
the target window was in the foreground and would stop otherwise; it never
had to. An Alt tap used to take the foreground shows up as an extra key
record.

Windows Terminal and classic conhost behaved identically, 3 runs each:

| Key | Signal | Key records |
| --- | --- | --- |
| Ctrl+C | none | down `vk 0x00 ch 0x03 ctrl 0x0`; up `vk 0x43 ch 0x03 ctrl 0x28` |
| Ctrl+Break | `CTRL_BREAK_EVENT` | **no down**; up `vk 0x03 ch 0x03 ctrl 0x128` |
| Esc | none | down `vk 0x00 ch 0x1B`; up `vk 0x1B ch 0x1B` |
| Window closed | `CTRL_CLOSE_EVENT` | none |

In VS Code, the "H3 client" task ran `client` when its folder opened, 3 runs.
The same keys were sent to the VS Code window:

| Key | Signal | Key records |
| --- | --- | --- |
| Ctrl+C | none | down `vk 0x00 ch 0x03 ctrl 0x0`; no up |
| Ctrl+Break | **none** | **none** |
| Esc, pressed twice | none | one down `vk 0x00 ch 0x1B`; no up |
| Window killed | `CTRL_CLOSE_EVENT` in 1 of 3 runs | none |

In the first three VS Code runs, no key reached the client at all, because the
Welcome page held the keyboard focus. Those runs are kept in
`h3-vscode-nofocus.jsonl`. The runs above set `workbench.startupEditor` to
`none` in the workspace. A typed `x` (`SendInput` with `KEYEVENTF_UNICODE`)
also never reached the client, while the keys in the table did.

*Expectation (Ctrl+C is a `0x03` key record with no signal; Ctrl+Break is
`CTRL_BREAK_EVENT` with no key-down):* **held** for Windows Terminal and
classic conhost, with the key-up detail in surprise 11. **Did not hold** for
VS Code: Ctrl+C held, but Ctrl+Break never arrived (surprise 12).

## Jobs and breakaway

### J1: the error for a refused breakaway

The probe sat in a job with `KILL_ON_JOB_CLOSE` and no `BREAKAWAY_OK` (limit
flags `0x2000`). A spawn with flags `0x01000008` failed in `CreateProcessW`
with error 5, 3 of 3. *Expectation (`ERROR_ACCESS_DENIED`, 5):* **held**.

### J2: capture by an outer job

The outer job had `KILL_ON_JOB_CLOSE` and no breakaway; the inner job had
`BREAKAWAY_OK`. The spawn with the breakaway flag succeeded. The child was
still in a job: its own report showed the outer job's limit flags `0x2000`.
The outer job listed the child; the inner job was empty. In run 3 the child
was killed by the outer job closing before it could report.
*Expectation (spawn succeeds, child still in a job):* **held**.

### J3: cargo's job

| Run under | In a job | Limit flags | Breakaway spawn |
| --- | --- | --- | --- |
| `cargo test` (integration test running the probe) | yes | `0x2000` (`KILL_ON_JOB_CLOSE`, no `BREAKAWAY_OK`) | error 5 |
| `cargo run` | yes | `0x2000` | error 5 |

3 runs each. *Expectation:* **held**.

### J4: nested climon2

A probe in no job made a session job and started a child in it with
`PROC_THREAD_ATTRIBUTE_JOB_LIST`. The child started a grandchild with
`DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB`. The grandchild was in no job.
After the job handle closed, the child was dead with exit code `0x0`
(surprise 8) and the grandchild was alive (`0x103`), 3 of 3.
*Expectation:* **held**.

## Command spawn and hang-up (bundled ConPTY)

### S1: one-call spawn

Decision 5's shape (inherit TRUE plus an empty `HANDLE_LIST`) cannot be built
(surprise 1). With the working shape (`PSEUDOCONSOLE` and `JOB_LIST`, inherit
FALSE, `STARTF_USESTDHANDLES` with null handles), the `CreateProcessW` call
succeeded and `cmd.exe` was in the session job.

| Started | Handles in `cmd.exe` |
| --- | --- |
| Before the ten extra inheritable handles | 85 |
| After them, inherit FALSE | 85 |
| After them, inherit TRUE, no list | 96 |
| After them, inherit TRUE, one-entry list | 86 |

Identical in 3 of 3 runs. *Expectation (one call; no stray handles):*
**did not hold** as written; held with inherit FALSE.

### S2: release, then end of output

After `ConptyReleasePseudoConsole` (returned `S_OK`), `exit` in `cmd`:

| Variant | Leader exit | Output pipe ends | Error |
| --- | --- | --- | --- |
| plain | `0x0` | 1.0–1.1 ms after the leader exits | 109 `ERROR_BROKEN_PIPE` |
| `start /b ping -n 12 localhost`, then `exit` | `0x0` | 9.64–9.66 s after the leader exits, when `ping` finished | 109 |

The pipe stays open while a background child still holds the console.
*Expectation (broken pipe after release once the last client leaves):*
**held**.

### S3: close status per shell

Part 1: release, wait, start a background `ping -t` where the shell can, then
`ConptyClosePseudoConsole` without terminating the job. The close call
returned in under 1 ms every time, and the close event reached the processes
after release.

| Shell | Leader exit | Background child | Time from close to exit |
| --- | --- | --- | --- |
| `cmd /D` | `0xC000013A` | `PING.EXE` `0xC000013A` | 5–30 ms |
| `powershell -NoProfile` | `0xC000013A` | `PING.EXE` `0xC000013A` | 5–14 ms |
| `pwsh -NoProfile` (Store) | `0xC000013A` | not in the job (surprise 3) | 20–25 ms |
| `edit.exe` | `0xC000013A` | none | 5 ms |
| `vim.exe` | `0x1` | none | 11–12 ms |

Part 2 was the real sequence: close, sleep 5000 ms,
`TerminateJobObject(job, 0xC000013A)`. It ran 10 times per shell (3 for
pwsh with a 5 s warm-up). In all of them every process was already gone at 5 s
with the statuses above, so `TerminateJobObject` killed nothing.

*Expectation (leader reports `0xC000013A` or the shell's own code; processes
gone by about 5 s):* **held**, and much sooner than 5 s (surprise 6).

### S4: a process with no console in the session job

A `DETACHED_PROCESS` child in the session job logged any control event it
received. It received none, was alive at 4900 ms and 5000 ms after close, and
died at `TerminateJobObject` with `0xC000013A`. 3 of 3. *Expectation:*
**held**.

### S5: leader handle and job accounting

`cmd` started `start /b ping -n 6 localhost`, then exited. The leader's handle
signalled at once with exit `0x0`. `ActiveProcesses` went from 2 before the
exit, to 1 after it, to 0 about 4.06 s later when `ping` finished. 3 of 3.
Informational.

### S6: DLL load failure

`CreateProcessW` succeeded and the process exited `0xC0000135`, 3 of 3.
*Expectation:* **held**.

### S7: working directory length

| Working directory | Policy off, no manifest | Policy off, manifest | Policy on, no manifest | Policy on, manifest |
| --- | --- | --- | --- | --- |
| Short existing directory (control) | ok | ok | ok | ok |
| 308 characters | 267 | 267 | 267 | 267 |
| Same with `\\?\` prefix | 267 | 267 | 267 | 267 |
| Nonexistent | 267 | 267 | 267 | 267 |
| A file | 267 | 267 | 267 | 267 |

The long directory existed (created through `\\?\`). The policy was set back
to 0 afterwards. *Expectation:* none stated; see surprise 9.

## Ctrl+C and Ctrl+Break through ConPTY

### C1: Ctrl+C as a byte

Writing `0x03` to the input pipe stopped `ping -t` within 5 ms, exit
`0xC000013A`, 3 of 3. A raw-mode reader (processed input off) read the single
byte `03`, 3 of 3. *Expectation:* **held**.

### C2: inherited "ignore Ctrl+C"

`ping -t` in a session hosted by a detached host, then `0x03` written, 3 runs
per row:

| Host started | Host repairs first | `ping` stops |
| --- | --- | --- |
| plainly (baseline) | no | yes, `0xC000013A` |
| (a) with `CREATE_NEW_PROCESS_GROUP` | no | **no** |
| (b) after its parent called `SetConsoleCtrlHandler(NULL, TRUE)` | no | **no** |
| (c) both (a) and (b) | yes | yes, `0xC000013A` |
| (a) only | yes | yes |
| (b) only | yes | yes |

In the detached host, which has no console, `SetConsoleCtrlHandler(NULL,
FALSE)` returned 1 with `GetLastError` 0. *Expectation ((a) and (b) break
Ctrl+C; (c) repairs it):* **held**.

### C3: Ctrl+Break by attach helper

The helper ran `AttachConsole(leader)`, registered a TRUE handler, called
`GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0)` and `FreeConsole`. Every call
returned 1, the helper exited 0, inside and outside the session job, 3 runs
each.

| `cmd /D` running | Result | `cmd` answers a later `echo` |
| --- | --- | --- |
| `ping -t localhost` | `ping` prints `Control-Break` and keeps going | no (ping still running) |
| `probe.exe sleep` (default handler) | ends with `0xC000013A`; `cmd` alive | yes |

*Expectation (`ping` ends, shell survives):* the event was delivered and the
shell survived. `ping` did not end, by design (surprise 7).

### C4: Ctrl+Break by injected record, and the Esc side effect

The command was a raw-mode reader with a logging control handler. Times are
from the first write, and the pattern was identical in 3 of 3 runs:

| Step | After injection | Control (no injection) |
| --- | --- | --- |
| write `ESC[3;70;0;1;8;1_` | `CTRL_BREAK_EVENT` at 1 ms; no bytes | (not written) |
| write lone `0x1B` | **nothing for 800 ms** | `1b` at 1 ms |
| write `a` 800 ms later | `1b` then `61`, both at the moment of the `a` write | `61` at once |
| write `ESC[1;5R` | `1b 5b 31 3b 35 52` at once | same |

*Expectation (after injection the lone Esc is held until the next byte):*
**held**. Injection is therefore not the simpler alternative.

### C5: `?9001` at start-up

The first output from the bundled ConPTY, identical in 3 of 3 runs:

```
ESC[1t ESC[c ESC[?1004h ESC[?9001h ESC[?7l ESC[?7h Microsoft Windows [Version 10.0.26200.9974]…
```

Nothing answered any request and `echo c5-works` still round-tripped.
*Expectation:* `?9001h` is sent and the session works unanswered; **held**,
with the extra requests in surprise 10.

## Hand-offs and close events

### X1: close-event order

A, B and C each logged events. A and B waited up to 3 s for their child in the
handler, then exited with the child's code or 129. C exited 129 at once. Times
are from the close.

| Host | Event order | A's exit | Total |
| --- | --- | --- | --- |
| Bundled ConPTY, `ConptyClosePseudoConsole` (3 runs) | C 0.3 ms, B 1.6 ms, A 3.4–5.4 ms; each waiter's wait returned `0x0` (its child had exited) | `0x81` (129) passed up from C | 4–14 ms |
| Windows Terminal 1.18 tab, window closed (3 runs) | A 24–27 ms, B 3026–3038 ms, C 6035–6044 ms; A's and B's waits timed out (`0x102`) | `0x81` from A's own timeout | ~6.04 s |

See surprise 5. *Expectation:* unknown before.

### X2: full-width exit codes

| Child exit | Grandparent saw |
| --- | --- |
| `0xC000013A` | `0xC000013A` |
| `259` | `0x00000103` |
| `0xFFFFFFFF` | `0xFFFFFFFF` |

3 runs each. *Expectation (all round-trip):* **held**. 259 is also
`STILL_ACTIVE`, so `GetExitCodeProcess` alone cannot tell it from a live
process; only the signalled handle can.

### X3: Ctrl+C with handler routines

A fresh console window (`CREATE_NEW_CONSOLE`, so conhost) ran runner → A → B
→ C. The runner sent `GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0)`. A and B
logged event 0 and survived. C, which had the default handler, died with
`0xC000013A`, and B and then A reported that code. 3 of 3.
*Expectation:* **held**.

## Identity

### I1: creation time across a clock step

The time service was already stopped (`w32time`, start type Manual).
`SetSystemTime` to +1 h, then to −1 h (two hours back), then back to the
original plus the elapsed time all returned success. Creation time read
before, between and after was the same raw `FILETIME` every time, in 3 of 3
runs. Afterwards `w32tm /resync` had no time source ("no time data was
available"); the restore step had already put the clock back. The probe did
not read the clock back after each step, so it relies on `SetSystemTime`'s
success. *Expectation (unchanged):* **held**.

### I2: liveness errors

`OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE)`:

| Target | Elevated observer | Observer from `runas /trustlevel:0x20000` |
| --- | --- | --- |
| PID that never existed | error 87 | error 87 |
| PID 0 | error 87 | error 87 |
| PID 4 (System) | opens; wait `0x102` | error 5 |
| Exited and fully released | error 87 | error 87 |
| Exited, another handle held | opens; wait `0x0`; exit `0x0`; `GetProcessTimes` ok | same |
| Same user, elevated | opens | opens |
| SYSTEM (`winlogon`) | opens | error 5 |
| LOCAL SERVICE (`svchost`) | opens (error 5 when started through Explorer) | error 5 |

The `runas /trustlevel` observer still reported High integrity and elevated:
it only drops the Administrators group. With UAC off there is no real
non-elevated observer, so "an elevated process opened from a non-elevated
probe" was **not measured**. *Expectation ("no such process" is 87):*
**held**. Note that PID 0 also gives 87.

### I3: creation-time resolution

1000 processes were created suspended in a tight loop (about 0.9 s), and each
creation time was read. Across 6 batches: no two shared a value. The smallest
gap was 0.72–0.87 ms (7173–8700 × 100 ns), and the last digit took all values
0–9, so the clock is not quantised to the 15.6 ms tick. An earlier version
that waited on each process gave gaps of about 12.5 ms, which only reflected
the loop's own speed.

## Command line

### Q1: batch arguments

`std::process::Command` on a `.cmd` file (Rust 1.96.0). Every argument arrived
whole and quoted in `%1`…`%9` and `%*`:

| Sent | `%n` saw |
| --- | --- |
| `a%b`, `%PATH%`, `%%` | `"a%b"`, `"%PATH%"`, `"%%"`: not expanded |
| `a^b`, `^` | `"a^b"`, `"^"` |
| `a&b`, `&echo INJECTED` | `"a&b"`, `"&echo INJECTED"`: nothing ran |
| `a\|b`, `a<b`, `a>b`, `>C:\probes\q1\pwned.txt` | quoted as sent; the file was **not** created |
| `a!b`, `!PATH!` | as sent |
| `a"b`, `"` | `"a""b"`, `""""`: quotes doubled |
| `(a)`, `)` | as sent |
| `a\`, `a b\` | `"a\\"`, `"a b\\"`: trailing backslash doubled |
| `a b`, space, `  lead` | quoted, spaces kept |
| empty, `x`, empty | `""`, `x`, `""` |

CR and LF were refused ("batch file arguments are invalid", `InvalidInput`),
and NUL too ("nul byte found in provided data", `InvalidInput`).

### Q2: argv[0] as typed

`CreateProcessW` with `lpApplicationName = C:\probes\bin\probe.exe` and a
command line starting `probe`, `c2`, or `"some other name.exe"`: the child's
`argv[0]` was exactly that token. `GetModuleFileNameW` and
`std::env::current_exe` both gave the real path.

## Not run

| Probe part | Reason |
| --- | --- |
| H2 and H3 in VS Code's interactive terminal (not a task) | Typing into it with `SendInput` was unreliable. Tasks run in the same integrated terminal, and H1 covers the interactive default profile. |
| H1 over SSH | The session was not connected over SSH; the Claude Code session row stands in. |
| H2: closing a tab while its window stays open | Needs UI input. The tests closed the only window of a dedicated WT process, which ends both. |
| H3 with real key presses | Nobody at the keyboard; `SendInput` was used. |
| S3 with the MSI build of pwsh | Not present (see surprise 3). |
| I2: elevated target from a non-elevated observer | UAC is off, so no non-elevated observer exists. |
| Logoff and shutdown events | They end the session running the probes. They remain a manual check. |
