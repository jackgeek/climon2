# Windows host and shell qualification (agreed, partly implemented)

This document says what must be shown before a Windows release of climon2 can
be published: which terminals and shells are tested, which scenarios run, and
what a person checks by hand. It is for contributors. Only parts of the test
harness are built so far, and the sections that describe them say which. It
was agreed in
[Define Windows host and shell qualification](https://github.com/jackgeek/climon2/issues/213)
and is the specification the Windows port implements.

The monitor and client-library scenarios, and checklist items 13 to 16, were
added by
[Define Windows monitor library and specification integration](https://github.com/jackgeek/climon2/issues/216).
[Windows quality and CI qualification](quality.md) says which CI jobs
run these scenarios and how their results are stored.

## Terms

The [glossary](../../../GLOSSARY.md) defines the words used here. The ones that
matter most:

- A **host** is the terminal a user runs climon2 in. A **client** is the
  climon2 process in that terminal; it attaches to a **session daemon**, which
  runs the user's command and outlives the host.
- A **pseudoconsole** is the Windows object that lets one program act as the
  terminal of another. **ConPTY** is the Windows component that provides it. A
  host creates a pseudoconsole for the shell it shows, so the client always
  runs inside the host's pseudoconsole. Separately, the session daemon creates
  its own for the command, using the ConPTY files that ship with climon2
  ([ADR 0003](../../adr/0003-platform-and-shell-seams.md)).
- A **job object** is a Windows group of processes that can be ended together.
  A process **breaks away** when it is started outside the job its parent is
  in. The session daemon must break away from any job the host placed the
  shell in, or it would die with the host.
- The **terminal oracle** is the test harness's emulated host: what it shows
  stands for what a user would see.
- A **shell profile** is the harness's description of how to start one shell
  in a controlled way.
- A **host model** is the harness's description of how one qualified host
  creates, feeds and closes the pseudoconsole it gives a client.

## What is qualified

| Kind | Qualified | Unqualified, not blocked |
| --- | --- | --- |
| Host | Windows Terminal; the VS Code integrated terminal, default settings | Classic conhost; VS Code with `terminal.integrated.windowsUseConptyDll` off; SSH sessions |
| Shell | PowerShell 7 (`pwsh`), Windows PowerShell 5.1 (`powershell`), `cmd.exe` | Any other shell |
| Full-screen app | vim for Windows, Microsoft Edit | |

A qualified host has two kinds of evidence: automated scenarios run under its
host model, and a [manual checklist](#manual-checklist) run in the real
application before each release.

## Host models

The harness creates the pseudoconsole itself, by calling ConPTY directly, and
configures it as the real host does. It cannot use the `portable-pty` crate
for this, because that crate fixes the creation flags and can only end a
process abruptly.

| | Windows Terminal model | VS Code model |
| --- | --- | --- |
| Creation flags | `0x08` | `0` |
| Key input | win32-input-mode: the model honours ConPTY's `CSI ? 9001 h` and sends keys in that encoding | plain VT sequences |
| Close | closes the pseudoconsole | waits 250 ms, closes the pseudoconsole, then ends the shell with `TerminateProcess(shell, 1)` |
| Job around the shell | none | none |

Both models load the same pinned ConPTY package that climon2 ships: one
version and one hash for the product and the harness. The real hosts ship
their own ConPTY versions, which deliver close events in different orders, so
scenarios about closing assert outcomes only (exit status, the session
surviving, nothing left behind) and never the order of events. Differences
between real ConPTY versions are the manual checklist's job.

The pin is `pins/conpty.txt`: the version of Microsoft's
`Microsoft.Windows.Console.ConPTY` package, a stable release, and the SHA-256
of the package and of its x64 `conpty.dll` and `OpenConsole.exe`. The harness
downloads that version from nuget.org, checks all three hashes, and fails if
any differs. It keeps the two files in `target/conpty/<version>` and checks
them again before each use.

Job shape is a separate harness setting, not part of a host model. Three
scenarios use it:

| Shape | Required result |
| --- | --- |
| The shell is in a job that allows breakaway | `exec` succeeds and the session survives the host |
| The shell is in a job that refuses breakaway | `exec` exits 125 |
| `c2` is run through `Start-Process -Wait`, from each PowerShell | `exec` exits 125 |

Built so far ([#320](https://github.com/jackgeek/climon2/issues/320)): the
Windows Terminal model and the job-shape setting, in `climon2-test-term`'s
Windows host. The VS Code model is not. A real host's shell answers Ctrl+C, so
the harness clears any instruction to ignore Ctrl+C that the test process
inherited before it starts the shell; otherwise the shell would inherit it
too.

cargo runs every test inside a job of its own that refuses breakaway, so a
`c2 exec` started from a test would always exit 125. On the `windows-2025`
runner every process from `Runner.Listener` down, the step's shell included,
is also in a job of the runner's that refuses breakaway, so no process a test
can reach is outside every job. The harness therefore runs a program through
WMI's `Win32_Process.Create`, the one documented way out of the caller's job,
with `CREATE_BREAKAWAY_FROM_JOB` to leave WMI's own: it starts the test
executable again as a relay, which starts the program with captured output,
optionally inside jobs of the shapes above, outer job first
([#317](https://github.com/jackgeek/climon2/issues/317)). Headless `exec`'s
tests use it for every job shape.

## Shell profiles

| Shell | Start-up | Prompt |
| --- | --- | --- |
| `pwsh` 7.x, the runner's MSI build | `-NoLogo -NoProfile` | a fixed `prompt` function |
| `powershell` 5.1 | `-NoLogo -NoProfile` | a fixed `prompt` function |
| `cmd.exe` | `/D` | the `PROMPT` variable |

PSReadLine, PowerShell's line editor, stays on, because users run with it.
Each shell is started by absolute path. A missing shell or a wrong major
version fails CI, and the exact version is recorded with the results.

The Store build of `pwsh` behaves differently around job objects and is not on
the CI runner. Only the manual checklist covers it.

Built so far ([#320](https://github.com/jackgeek/climon2/issues/320)): the
`pwsh` profile. `CLIMON2_TEST_PWSH` may name another absolute path for local
runs.

## Full-screen apps

Neither app is on the CI runner, and Windows 11 25H2 ships an older `edit.exe`
that takes precedence on `PATH`. Both are therefore pinned downloads, verified
against a SHA-256 kept in the repository, and run by absolute path:

| App | Source | Controlled start-up |
| --- | --- | --- |
| vim for Windows | the `vim-win32-installer` release zip | `--clean` |
| Microsoft Edit | the Microsoft Edit 2.0.0 release zip | `APPDATA` set to an empty directory |

Each app's scenarios: enter it, edit, detach, reattach and see the screen
restored, resize, quit and see the main screen restored. Edit takes the place
`less` has on Unix.

## Automated scenarios

### Coverage

| Dimension | Full suite | Subset |
| --- | --- | --- |
| Host model | Windows Terminal model | VS Code model: key input, resize, close and hang-up, attach and restoration, full-screen apps |
| Shell | `pwsh` | `powershell` and `cmd.exe`: start and prompt, run a command and pass its exit status through, Ctrl+C, detach and reattach with restoration, close with its measured status |

In the shell subset, each shell is both the outer shell that runs `c2` and the
shell inside the session. The Windows
[smoke test](../../../GLOSSARY.md) is `pwsh` started from `pwsh`.

### Existing scenarios

The session suites, `packaged.rs` and `continuity.rs` in
`crates/climon2-e2e/tests`, and the harness's own tests must pass on Windows.
Of their 34 scenarios, 27 keep their intent and are driven through `pwsh`
instead of bash. These change:

| Unix scenario | Windows form |
| --- | --- |
| bash smoke test, exit status 7 | `pwsh` from `pwsh`, exit status 7 |
| Nested bash sessions | Nested `pwsh` sessions, which rely on the session's job object allowing breakaway |
| Real vim edits and saves | vim for Windows |
| Real `less` navigates and resizes | Microsoft Edit navigates and resizes |
| A killed command returns 137 and its descendants are cleaned up | A command ended with `TerminateProcess(…, 1)` reports status 1, and the session's job object leaves no descendant |
| A killed client falls back to the daemon's [leave policy](../../../GLOSSARY.md) | The same, with the client ended by `TerminateProcess` |
| A killed daemon: logs are reported and `list` reclaims the stale session | The same, and the command dies with the job object |

One scenario is an exception: "the actual previous release remains live
across an install" cannot run for the first Windows release, which has no
predecessor. It is recorded as not applicable until the second.

Built so far ([#321](https://github.com/jackgeek/climon2/issues/321)): the
smoke test, in `crates/climon2-cli/tests/packaged_windows.rs` until
`climon2-e2e` builds on Windows. From `pwsh` it runs `c2 exec pwsh`, runs a
command, detaches, lists the session with `c2 list`, reattaches with
`c2 attach` and sees the session's screen restored, then ends the session with
`exit 7`, which reaches the outer shell. The Windows Test job runs it against
the zip the Dist job built
([Windows quality and CI qualification](quality.md#pull-request-jobs)).
The test unpacks the zip and writes the `current` file itself; the Archive
job tests installing with `install.ps1`
([#329](https://github.com/jackgeek/climon2/issues/329)). Like headless `exec`'s tests, it
runs again outside every job, through the harness's relay.

The refusal of a session attaching to itself keeps its Unix intent, because
the daemon sets `CLIMON2_SESSION_ID` for the command on Windows too
([Windows platform and lifecycle contract](lifecycle.md#self-attach-and-nested-sessions)).
Log privacy keeps its Unix intent, driven through `pwsh`: the environment,
argument and typed-input secrets appear in no log. On Windows it also asserts
that every log file, metadata file and managed folder the run produced carries
the private descriptor and is not a reparse point, as
[ADR 0003](../../adr/0003-platform-and-shell-seams.md#windows-io-and-transport-agreed-not-implemented)
specifies under "Private files and folders". No exception is recorded.

### Windows-only scenarios

| ID | Group | Required result |
| --- | --- | --- |
| `WIN-01` | Close, per host model | The client exits 129, the leave policy applies, and a detached session survives and reattaches |
| `WIN-02` | Close status, per shell and app | The session reports the status measured for `pwsh`, `powershell`, `cmd.exe`, vim and Edit (`0xC000013A`; vim `0x1`) |
| `WIN-03` | Host jobs | The three job shapes under [Host models](#host-models) |
| `WIN-04` | Key input, per host model | One key table (arrows, Home and End, function keys, Alt and Ctrl combinations, Ctrl+C as byte `0x03`), sent in that model's encoding, arrives in the session as the expected bytes |
| `WIN-05` | Console preflight | `exec` and `attach` refuse a redirected stdin or stdout as they refuse a missing terminal on Unix |
| `WIN-06` | ConPTY files | With `conpty.dll` or `OpenConsole.exe` missing, `exec` exits 125 |
| `WIN-07` | Command resolution | Commands resolve in `PATH` × `PATHEXT` order; a `.cmd` or `.bat` argument that `cmd.exe` cannot represent exits 2 |
| `WIN-08` | Start-up negotiation | ConPTY's own start-up sequences (`ESC [ 1 t`, `ESC [ c`, `CSI ? 1004 h`) never reach the host, and `CSI ? 9001` is never forwarded |
| `WIN-09` | Ctrl+Break, Windows Terminal model | A program using the default handler ends with `0xC000013A` and the shell survives; the key-up record that accompanies Ctrl+Break never reaches the session as `0x03` |
| `WIN-10` | Full-width statuses | `0xC000013A`, `259` and `0xFFFFFFFF` pass unchanged through the launcher, the CLI and the client |
| `WIN-11` | Long working directory | `exec` from a working directory over 260 characters exits 126 |

`WIN-07`, `WIN-10` and `WIN-11` are built
([#322](https://github.com/jackgeek/climon2/issues/322)). `WIN-07` and
`WIN-11` run headless `exec` through the relay; `WIN-10` runs ordinary `exec`
through the launcher in the Windows Terminal model. `WIN-11` gives the CLI its
working directory through a test hook, because Windows cannot start a
program in a directory that long either.

`WIN-09` to `WIN-11` follow the
[Windows platform and lifecycle contract](lifecycle.md). Ctrl+Break
has no VS Code model scenario: that host delivers nothing for it, a documented
limitation.

### Monitor and client-library scenarios

Every scenario in the [monitor acceptance matrix](../behaviour/monitor/acceptance.md),
which covers the client library, the monitor and headless `exec` (about 210
scenarios in fifteen test files), runs in the Windows Test job. The programs
inside its sessions run under `pwsh`; the shell subset above does not apply.

| Host model | Scenarios |
| --- | --- |
| Windows Terminal model | All of them |
| VS Code model | The terminal-facing monitor scenarios: rendering, input routing, resize, the selector, toasts, the bell and attention reports |

A scenario that cannot pass on Windows needs an explicit, documented decision,
as the exceptions above have.

## Manual checklist

The real applications cannot run in CI, so a person runs this checklist once
per release candidate. It is a [release gate](../../../GLOSSARY.md): a missing or
failed result blocks publication unless the owner records a
[disposition](../../../GLOSSARY.md).

**Environment.** A Windows 11 client (not Server) with the current stable
Windows Terminal and VS Code, and the exact candidate archive installed with
`install.ps1`. Unlike the harness, it uses what a user has: Store `pwsh` as
the default profile, the user's PowerShell profile loaded, default host
settings.

Run every item in Windows Terminal, then in the VS Code integrated terminal.
Item *n* is scenario `WINM-`*n* (`WINM-01` to `WINM-16`) in the release's
evidence records:

1. Record the Windows build, the host's version and the `pwsh` build. Run the
   job probe, a PowerShell snippet that calls `IsProcessInJob`, and record its
   result.
2. `c2 exec pwsh`, run a command, detach, `c2 list`, reattach: the screen is
   restored.
3. `exit 7` inside the session: the outer shell sees 7.
4. Close the tab or panel while attached: from a new one, the session is
   listed and reattaches.
5. Close the whole application while attached: the same.
6. Keys: arrows, Home and End, a function key and an Alt combination arrive;
   Ctrl+C interrupts a running command.
7. Paste several lines: they arrive once, in order.
8. Resize the window with vim open: it redraws to the new size.
9. vim, then Edit: open, edit, detach, reattach, quit; the main screen
   returns.
10. CJK text, emoji and box-drawing characters line up.
11. A short pass in `cmd.exe` and in `powershell` 5.1: start, run a command,
    check the exit status.
12. VS Code only: run `c2 exec` from a task terminal, and run "Reload Window"
    with a session attached; the session survives.
13. With two sessions running, `c2 monitor` opens and lists both.
14. The selector switches between them, and typed input reaches only the
    selected session.
15. A session that needs attention raises a toast, and the prefix followed by
    `j` jumps to it.
16. Resizing the window re-renders the monitor, and exiting the monitor leaves
    both sessions running.

Signing out and shutting down are not checklist items: sessions end with
them and no status is promised.

**Observation, not an item.** In Windows Terminal with the Store build of
`pwsh` as the session's shell, start a hidden program
(`Start-Process -WindowStyle Hidden`), kill the session, and record whether the
hidden program survived. The contract accepts that it can; the record is
evidence, not a gate.
