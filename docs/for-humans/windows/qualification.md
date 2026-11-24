# Windows host and shell qualification

This document says what must be shown before a Windows release of climon2 can
be published: which terminals and shells are tested, which scenarios run, and
what a person checks by hand. It is for contributors. Built: both
[host models](#host-models) and the job shapes, the `pwsh`
[shell profile](#shell-profiles), the smoke test, nested sessions and
self-attach, the Windows-only scenarios except vim and Edit's part of
`WIN-02`, the monitor's Windows scenarios, and recording these scenarios and
the [manual checklist](#manual-checklist) in release qualification
([#330](https://github.com/jackgeek/climon2/issues/330)). Not built yet:
the `powershell` and `cmd.exe` profiles, the
[full-screen apps](#full-screen-apps) and the
[session suites](#existing-scenarios) on Windows
([#328](https://github.com/jackgeek/climon2/issues/328)); and the monitor
acceptance matrix's own suites on Windows. The sections below
say which parts are built. It was agreed in
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
Windows host. The VS Code model's creation flags, plain VT keys and close
were added for the monitor's scenarios
([#327](https://github.com/jackgeek/climon2/issues/327)), and its key table
and its scenarios with
[#325](https://github.com/jackgeek/climon2/issues/325) (see
[Windows-only scenarios](#windows-only-scenarios)). A real host's shell answers Ctrl+C, so
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

Built so far ([#324](https://github.com/jackgeek/climon2/issues/324)): the
three job-shape scenarios, `WIN-03`, in
`crates/climon2-cli/tests/packaged_windows.rs`. Each runs again outside every
job through the relay, then hosts a shell under the Windows Terminal model and
types `c2 exec --headless` into it:

- In a job that allows breakaway, `exec` exits 0. The scenario then drops the
  host, which ends every process in its job, and checks that the session's
  daemon still runs and is in no job.
- In a job that refuses breakaway, `exec` exits 125, says the daemon cannot
  leave the job, and leaves no session.
- `Start-Process -Wait -NoNewWindow` from `pwsh` and from Windows PowerShell
  5.1 gives the same 125. `Start-Process` without `-Wait` starts the session,
  so the job comes from `-Wait`.

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
runs. The `powershell` and `cmd.exe` profiles are
[#328](https://github.com/jackgeek/climon2/issues/328)'s.

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
`less` has on Unix. None of this is built yet
([#328](https://github.com/jackgeek/climon2/issues/328)).

Built so far ([#328](https://github.com/jackgeek/climon2/issues/328)): the
pins are `pins/vim.txt` (vim 9.2.1259) and `pins/edit.txt` (Edit 2.0.0). Each
names the zip's URL, the zip's SHA-256, the program in it and the SHA-256 of
each file the harness relies on: `vim.exe` and the `vim64.dll` it loads, or
`edit.exe`. `climon2-test-term`'s `App` downloads the zip, checks its hash,
unpacks it and checks each pinned file, and keeps it in `target/apps/<the
zip's hash>`, or under `CLIMON2_TEST_APP_CACHE`. It checks the pinned files
again before each use. A hash that differs fails.

The scenarios are in `crates/climon2-cli/tests/fullscreen_windows.rs`, under
both host models. From a session's `pwsh` each app opens a document, adds a
line, detaches, is attached again with its screen restored, sees the host's
new size after a resize, saves and quits, and leaves the shell's main screen
as it was. Both pass under both models.

vim leaves insert mode with a lone Escape. The client holds an Escape as the
possible start of a paste marker until the next byte arrives, and the
session's pseudoconsole reads an Escape and the byte after it, written
together, as Alt with that byte. So on Windows the client passes held bytes
on by themselves once 50 ms pass with no byte after them
([Windows platform and lifecycle contract](lifecycle.md#client-console)). Unix
keeps holding them until the next byte (owner decision on #328).

Edit could not start in a session until the daemon answered its
pseudoconsole's own device attributes request. The pseudoconsole takes the
first answer it reads as its host's. Edit asks for the device attributes
and waits for the answer, so it waited forever. The daemon now writes that
first answer itself, and the answer to the command's own request reaches the
command (`the_answer_to_a_commands_device_attributes_request_reaches_it` in
`crates/climon2-platform/tests/pty_windows.rs`).

## Automated scenarios

### Coverage

| Dimension | Full suite | Subset |
| --- | --- | --- |
| Host model | Windows Terminal model | VS Code model: key input, resize, close and hang-up, attach and restoration, full-screen apps |
| Shell | `pwsh` | `powershell` and `cmd.exe`: start and prompt, run a command and pass its exit status through, Ctrl+C, detach and reattach with restoration, close with its measured status |

In the shell subset, each shell is both the outer shell that runs `c2` and the
shell inside the session. The Windows
[smoke test](../../../GLOSSARY.md) is `pwsh` started from `pwsh`.

Built so far ([#328](https://github.com/jackgeek/climon2/issues/328)): the
shell subset, in `crates/climon2-cli/tests/shells_windows.rs`, under the
Windows Terminal model. From `powershell`, and from `cmd.exe`, `c2 exec`
starts the same shell, which shows its prompt and runs a command. A child's
exit status reaches it. Ctrl+C interrupts a running `ping` and the shell
takes input again. A detach gives the outer shell 0, and `c2 attach` restores
the session's screen. `exit 7` gives the outer shell 7. Then `c2 kill` of a
second session gives the outer shell `0xC000013A`, the measured close
status, which both shells show as `-1073741510`. The test waits for `ping`'s
English `Reply from` before Ctrl+C, as the CI runner is English. Keys typed
while an attaching client starts may not reach the session, so after
`c2 attach` it retypes a line until the session shows it.

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

The same file holds the Windows forms of two more scenarios
([#324](https://github.com/jackgeek/climon2/issues/324)):

- **Nested `pwsh` sessions.** Inside a session, `c2 exec pwsh` starts a second
  session at depth 2. Its daemon is in no job, so it has left the outer
  session's job. `exit 7` in the inner session reaches the outer session's
  shell, and `exit 9` in the outer one reaches the host's shell.
- **Self-attach.** `c2 attach $env:CLIMON2_SESSION_ID` inside a session exits
  125 with "cannot attach to the current session", and the session carries
  on.

The refusal of a session attaching to itself keeps its Unix intent, because
the daemon sets `CLIMON2_SESSION_ID` for the command on Windows too
([Windows platform and lifecycle contract](lifecycle.md#self-attach-and-nested-sessions)).
Log privacy keeps its Unix intent, driven through `pwsh`: the environment,
argument and typed-input secrets appear in no log. On Windows it also asserts
that every log file, metadata file and managed folder the run produced carries
the private descriptor and is not a reparse point, as
[ADR 0003](../../adr/0003-platform-and-shell-seams.md#windows-io-and-transport)
specifies under "Private files and folders". No exception is recorded. Log
privacy's Windows form is not built yet
([#328](https://github.com/jackgeek/climon2/issues/328)).

Built so far ([#328](https://github.com/jackgeek/climon2/issues/328)): the
three killed-process scenarios and log privacy, in
`crates/climon2-cli/tests/sessions_windows.rs`, under the Windows Terminal
model. In the killed scenarios the session's `pwsh` starts `ping` on a
console of its own with no window, so closing the session's console does not
reach it and only the session's job can end it:

- **Killed command.** `TerminateProcess(…, 1)` on the session's shell: the
  client, the CLI and the launcher exit 1, and the child ends.
- **Killed client.** `TerminateProcess` on the client: the CLI and the
  launcher exit with its status, and the leave policy hangs the ordinary
  session up. The child ends no sooner than the five-second grace after
  the kill, and `c2 list` shows no session.
- **Killed daemon.** The client says once that it lost the connection,
  naming the session's log folder, and exits 125. The shell and its child
  die with the job. The stale session stays until `c2 list` reclaims it.
- **Log privacy.** With `CLIMON2_LOG=trace`, an environment variable, an
  argument and a typed line each reach the session's `pwsh` and appear in
  no log. While the session runs and after it ends, every file and folder
  under `logs` and `sessions`, and the runtime directory with everything in
  it but its socket, carries the private descriptor and is no reparse point.

These run in the Windows Test job only. None is the only test covering a
registered Windows file, so they stay out of each mutant's test run
(owner decision on #328, 2026-11-22).

#### Scenarios with no Windows form

By the same decision, only the scenarios this document gives a Windows form
are ported. Each other Unix scenario is not applicable on Windows. Most
exercise shared code, which the Unix suites already test on every change;
the platform's own part is covered where the table says.

| Unix scenario | Why not applicable |
| --- | --- |
| The generic fixture keeps the exact arguments, working directory, environment and size | The fixture programs (`climon2-fixtures`) build only on Unix. Argument quoting is `WIN-07` and `command_windows.rs`; the terminal identity and size are in `exec_windows.rs`; log privacy is ported |
| The TUI fixture round-trips queries, mouse, paste and resize | The TUI fixture builds only on Unix. Keys and resize per host model are `WIN-04` and `WIN-08`; the rest is shared code |
| Debug-hook start-up panic and never-ready | Its faults come from hooked debug builds of the Unix package; start-up deadlines are shared code |
| Log headers, links and filters survive the `exec` hand-offs | Shared logging code; Windows log file creation is `private_windows.rs` and log privacy |
| Raw socket abuse and a pending hello | Shared connection admission; the Windows socket seam is `connection_windows.rs` |
| The previous release stays live across an install | No previous Windows release exists until the second one (owner decision) |
| A client missing after readiness | Already covered: an ordinary `exec` whose client is missing exits 125 and stops its daemon (`exec_windows.rs`) |
| Debug-hook crash in the middle of an install | It hooks the Unix installer; `install.ps1` has its own Archive job tests ([#329](https://github.com/jackgeek/climon2/issues/329)) |
| Controlled versions each route their own frozen client | Its synthetic versions are Unix scripts, and compatibility with an earlier Windows release is out of scope until the second one |
| Debug-hook client panic after `exec` | A hooked debug client; the leave policy after a lost client is the killed-client scenario |
| The outer shell's input stays below output restored after vim | Shared restoration code |
| Detach and reattach on an occupied host | Shared restoration code; detach and reattach run in the smoke test and `WIN-01` |
| An alternate-screen program is reattached and quitting it shows the normal screen | Shared restoration code; the full-screen apps' scenarios detach and reattach on Windows |
| Two clients share a session with exactly one controller | Shared control code; two attached clients run in `WIN-02` |
| `kill` ends detached and attached sessions | Already covered: `WIN-02` and `exec_windows.rs`'s `kill` of a headless session |
| An invalid config is refused before a session exists | Shared configuration code |
| The config rebinds the client's keys | Shared configuration and key code |
| `scrollback.max_rows` bounds restored history | Shared daemon code |
| The session answers queries without the host | Shared terminal code; ConPTY's own start-up queries are `WIN-08` |
| Titles are encoded and links are plain text | Shared terminal code |
| A detached session absorbs a large write | Shared daemon code; the Windows pseudoconsole's output is `pty_windows.rs` |
| The client releases the host on every ending | The Unix form checks the terminal's termios; Windows restores console modes and code pages, which `terminal_windows.rs` checks |
| An interrupted paste is closed once without a stale payload | Shared control and paste code |
| A large restoration is complete and ordered | Shared restoration code |
| A wide host presents the session's rectangle | Shared presentation code |
| A tall host scrolls the session's rectangle | Shared presentation code |

### Windows-only scenarios

| ID | Group | Required result | Layer |
| --- | --- | --- | --- |
| `WIN-01` | Close, per host model | The client exits 129, the leave policy applies, and a detached session survives and reattaches | Contract |
| `WIN-02` | Close status, per shell and app | The session reports the status measured for `pwsh`, `powershell`, `cmd.exe`, vim and Edit (`0xC000013A`; vim `0x1`) | Contract |
| `WIN-03` | Host jobs | The three job shapes under [Host models](#host-models) | Contract |
| `WIN-04` | Key input, per host model | One key table (arrows, Home and End, function keys, Alt and Ctrl combinations, Ctrl+C as byte `0x03`), sent in that model's encoding, arrives in the session as the expected bytes | Contract |
| `WIN-05` | Console preflight | `exec` and `attach` refuse a redirected stdin or stdout as they refuse a missing terminal on Unix | Contract |
| `WIN-06` | ConPTY files | With `conpty.dll` or `OpenConsole.exe` missing, `exec` exits 125 | Contract |
| `WIN-07` | Command resolution | Commands resolve in `PATH` × `PATHEXT` order; a `.cmd` or `.bat` argument that `cmd.exe` cannot represent exits 2 | Contract |
| `WIN-08` | Start-up negotiation | ConPTY's own start-up sequences (`ESC [ 1 t`, `ESC [ c`, `CSI ? 1004 h`) never reach the host, and `CSI ? 9001` is never forwarded | Contract |
| `WIN-09` | Ctrl+Break, Windows Terminal model | A program using the default handler ends with `0xC000013A` and the shell survives; the key-up record that accompanies Ctrl+Break never reaches the session as `0x03` | Contract |
| `WIN-10` | Full-width statuses | `0xC000013A`, `259` and `0xFFFFFFFF` pass unchanged through the launcher, the CLI and the client | Contract |
| `WIN-11` | Long working directory | `exec` from a working directory over 260 characters exits 126 | Contract |

`WIN-07`, `WIN-10` and `WIN-11` are built
([#322](https://github.com/jackgeek/climon2/issues/322)). `WIN-07` and
`WIN-11` run headless `exec` through the relay; `WIN-10` runs ordinary `exec`
through the launcher in the Windows Terminal model. `WIN-11` gives the CLI its
working directory through a test hook, because Windows cannot start a
program in a directory that long either.

Built so far ([#323](https://github.com/jackgeek/climon2/issues/323)):
`WIN-01` under the Windows Terminal model and `WIN-02` for the three shells,
in `crates/climon2-cli/tests/packaged_windows.rs` beside the smoke test.
`WIN-01` closes the host of an attached ordinary session (the client exits 129
and the session ends), of a client attached to a headless session (the session
keeps running and reattaches) and of a shell that a session was detached from
(the session survives and reattaches). `WIN-02` attaches two clients to a
session of each shell and runs `c2 kill`: both exit with `0xC000013A`. The
`powershell` and `cmd.exe` start-ups are the test file's own until the harness
has their profiles. vim and Edit join `WIN-02` with their pinned downloads
([#328](https://github.com/jackgeek/climon2/issues/328)), in
`fullscreen_windows.rs`: `c2 kill` of a session whose command is the app
ends its client with vim's `0x1` or Edit's `0xC000013A`. vim's close handler
writes to the console as it ends. The daemon reads no output during the
grace, so until #328 ConPTY blocked on the full output pipe and vim with it,
and the session job ended vim with `0xC000013A`. The daemon's output thread
now drops what it reads once the pseudoconsole is closed
([Session job and hang-up](lifecycle.md#session-job-and-hang-up)). Where only the Store
build of `pwsh` is installed, which a session cannot start, the local runs of
`WIN-01` use `powershell` in its place; CI requires the MSI build. In
`crates/climon2-cli/tests/attach_deadline_windows.rs`, an ordinary session
that nobody attaches to within five seconds ends, with `exec` held before it
starts the client. It is a test binary of its own because cargo runs test
binaries one at a time: running beside the kill tests, it made them miss
their ten-second deadline under the coverage build.

`WIN-09` is built ([#326](https://github.com/jackgeek/climon2/issues/326)),
in `crates/climon2-cli/tests/packaged_windows.rs`. It runs ordinary `exec` of
`cmd.exe` through the launcher in the Windows Terminal model and presses
Ctrl+Break twice. `ping -t` in the session reports Ctrl+Break and keeps
running, which it would not after Ctrl+C, so the key-up record never arrived
as `0x03`. `timeout`, which keeps the default handler, ends with
`0xC000013A`, and `cmd.exe` goes on to its next command.

`WIN-09` to `WIN-11` follow the
[Windows platform and lifecycle contract](lifecycle.md). Ctrl+Break
has no VS Code model scenario: that host delivers nothing for it, a documented
limitation.

Built so far ([#325](https://github.com/jackgeek/climon2/issues/325)):
`WIN-04`, `WIN-05`, `WIN-08` and the VS Code model's subset, in
`crates/climon2-cli/tests/host_models_windows.rs`. Its session program reads
its console as a full-screen program does, with VT input on and nothing
processed, and records every byte it reads.

- `WIN-04` presses the arrows, Home, End, F1, F4, F5, F12, Alt+x, Ctrl+Right,
  Ctrl+A and Ctrl+C under each model. Each arrives as the bytes xterm sends,
  Ctrl+C as `0x03`.
- Under each model, the host's focus reports reach a session once it turns
  focus reporting on, as `CSI O` and `CSI I`, and not before. The console
  never makes them focus records in VT input mode, so the client passes them
  on as typed bytes (see
  [Client console](lifecycle.md#client-console)).
- `WIN-08` runs a session under each model and finds none of ConPTY's
  start-up sequences in what the host received since `exec`. The host's own
  pseudoconsole passes on the client's "focus reporting off" and turns it
  straight back on for itself; the scenario leaves that pair out. The same
  run resizes the host and sees the new size in the session.
- `WIN-05` pipes into `exec` and `attach`, and sends their output to a file:
  each exits 125 with the error a missing terminal gives.
- The VS Code subset closes the host of an ordinary session's client, which
  ends the session; closes the host of a headless session's client, after
  which the session runs on and a new host's `attach` restores its screen;
  and detaches, reattaches and runs `c2 kill`, which ends the client.

`WIN-06` is `exec_exits_125_when_a_bundled_conpty_file_is_missing` in
`crates/climon2-cli/tests/exec_windows.rs`. The key table's encodings are
`climon2-test-term`'s, and its own tests check them against the
win32-input-mode specification and xterm.

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

Built so far ([#327](https://github.com/jackgeek/climon2/issues/327)): the
CLI's `tests/monitor_windows.rs` runs the monitor's Windows scenarios against
the packaged product: the console refusals, listing, switching and input
routing, resize, toasts with the bell and the host attention reports,
prefix+j, exit and console close leaving sessions running, and a stalled
library load. Each terminal-facing one runs under both host models. The
Unix monitor, client-library and headless `exec` suites themselves do not
build on Windows yet.

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

Run every item in Windows Terminal, then in the VS Code integrated terminal,
except `WINM-12`, which is for VS Code alone. Each item is a scenario in the
release's evidence records, in the Host manual layer, and passes only with a
passed record for every host its row names:

| ID | Check | Required result | Layer | Hosts |
| --- | --- | --- | --- | --- |
| `WINM-01` | Record the Windows build, the host's version and the `pwsh` build. Run the job probe, a PowerShell snippet that calls `IsProcessInJob` | Its result is recorded | Host manual | Windows Terminal, VS Code |
| `WINM-02` | `c2 exec pwsh`, run a command, detach, `c2 list`, reattach | The screen is restored | Host manual | Windows Terminal, VS Code |
| `WINM-03` | `exit 7` inside the session | The outer shell sees 7 | Host manual | Windows Terminal, VS Code |
| `WINM-04` | Close the tab or panel while attached | From a new one, the session is listed and reattaches | Host manual | Windows Terminal, VS Code |
| `WINM-05` | Close the whole application while attached | From a new one, the session is listed and reattaches | Host manual | Windows Terminal, VS Code |
| `WINM-06` | Keys: arrows, Home and End, a function key and an Alt combination; Ctrl+C during a running command | Each key arrives; Ctrl+C interrupts the command | Host manual | Windows Terminal, VS Code |
| `WINM-07` | Paste several lines | They arrive once, in order | Host manual | Windows Terminal, VS Code |
| `WINM-08` | Resize the window with vim open | vim redraws to the new size | Host manual | Windows Terminal, VS Code |
| `WINM-09` | vim, then Edit: open, edit, detach, reattach, quit | The main screen returns | Host manual | Windows Terminal, VS Code |
| `WINM-10` | Show CJK text, emoji and box-drawing characters | They line up | Host manual | Windows Terminal, VS Code |
| `WINM-11` | A short pass in `cmd.exe` and in `powershell` 5.1: start, run a command | The exit status is the command's | Host manual | Windows Terminal, VS Code |
| `WINM-12` | Run `c2 exec` from a task terminal, and run "Reload Window" with a session attached | The session survives | Host manual | VS Code |
| `WINM-13` | With two sessions running, `c2 monitor` | It opens and lists both | Host manual | Windows Terminal, VS Code |
| `WINM-14` | Switch between them with the selector, and type | Typed input reaches only the selected session | Host manual | Windows Terminal, VS Code |
| `WINM-15` | Make a session need attention, then press the prefix followed by `j` | A toast is raised, and the prefix and `j` jump to the session | Host manual | Windows Terminal, VS Code |
| `WINM-16` | Resize the window, then exit the monitor | The monitor re-renders, and both sessions keep running | Host manual | Windows Terminal, VS Code |

Signing out and shutting down are not checklist items: sessions end with
them and no status is promised.

**Observation, not an item.** In Windows Terminal with the Store build of
`pwsh` as the session's shell, start a hidden program
(`Start-Process -WindowStyle Hidden`), kill the session, and record whether the
hidden program survived. The contract accepts that it can; the record is
evidence, not a gate.
