# Windows host and shell qualification (agreed, not implemented)

This document says what must be shown before a Windows release of climon2 can
be published: which terminals and shells are tested, which scenarios run, and
what a person checks by hand. It is for contributors. Nothing here is built
yet; it was agreed in
[Define Windows host and shell qualification](https://github.com/jackgeek/climon2/issues/213)
and is the specification the Windows port implements.

It does not cover the monitor or the client library on Windows, which
[Define Windows monitor library and specification integration](https://github.com/jackgeek/climon2/issues/216)
decides, nor how results are stored as evidence records, which
[Define Windows quality and CI qualification](https://github.com/jackgeek/climon2/issues/214)
decides.

## Terms

The [glossary](../GLOSSARY.md) defines the words used here. The ones that
matter most:

- A **host** is the terminal a user runs climon2 in. A **client** is the
  climon2 process in that terminal; it attaches to a **session daemon**, which
  runs the user's command and outlives the host.
- A **pseudoconsole** is the Windows object that lets one program act as the
  terminal of another. **ConPTY** is the Windows component that provides it. A
  host creates a pseudoconsole for the shell it shows, so the client always
  runs inside the host's pseudoconsole. Separately, the session daemon creates
  its own for the command, using the ConPTY files that ship with climon2
  ([ADR 0003](adr/0003-platform-and-shell-seams.md)).
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

Job shape is a separate harness setting, not part of a host model. Three
scenarios use it:

| Shape | Required result |
| --- | --- |
| The shell is in a job that allows breakaway | `exec` succeeds and the session survives the host |
| The shell is in a job that refuses breakaway | `exec` exits 125 |
| `c2` is run through `Start-Process -Wait`, from each PowerShell | `exec` exits 125 |

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
[smoke test](../GLOSSARY.md) is `pwsh` started from `pwsh`.

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
| A killed client falls back to the daemon's [leave policy](../GLOSSARY.md) | The same, with the client ended by `TerminateProcess` |
| A killed daemon: logs are reported and `list` reclaims the stale session | The same, and the command dies with the job object |

One scenario is an exception: "the actual previous release remains live
across an install" cannot run for the first Windows release, which has no
predecessor. It is recorded as not applicable until the second.

Two scenarios stay required but wait for decisions not yet made on the
[Windows support map](https://github.com/jackgeek/climon2/issues/205): the
refusal of a session attaching to itself, and log privacy.

### Windows-only scenarios

| Group | Required result |
| --- | --- |
| Close, per host model | The client exits 129, the leave policy applies, and a detached session survives and reattaches |
| Close status, per shell and app | The session reports the status measured for `pwsh`, `powershell`, `cmd.exe`, vim and Edit (`0xC000013A`; vim `0x1`) |
| Host jobs | The three job shapes under [Host models](#host-models) |
| Key input, per host model | One key table (arrows, Home and End, function keys, Alt and Ctrl combinations, Ctrl+C as byte `0x03`), sent in that model's encoding, arrives in the session as the expected bytes |
| Console preflight | `exec` and `attach` refuse a redirected stdin or stdout as they refuse a missing terminal on Unix |
| ConPTY files | With `conpty.dll` or `OpenConsole.exe` missing, `exec` exits 125 |
| Command resolution | Commands resolve in `PATH` × `PATHEXT` order; a `.cmd` or `.bat` argument that `cmd.exe` cannot represent exits 2 |
| Start-up negotiation | ConPTY's own start-up sequences (`ESC [ 1 t`, `ESC [ c`, `CSI ? 1004 h`) never reach the host, and `CSI ? 9001` is never forwarded |

Ctrl+Break has no scenario yet. Its design is still open in
[Define Windows platform support and lifecycle contract](https://github.com/jackgeek/climon2/issues/212),
and its scenarios follow that decision.

## Manual checklist

The real applications cannot run in CI, so a person runs this checklist once
per release candidate. It is a [release gate](../GLOSSARY.md): a missing or
failed result blocks publication unless the owner records a
[disposition](../GLOSSARY.md).

**Environment.** A Windows 11 client (not Server) with the current stable
Windows Terminal and VS Code, and the exact candidate archive installed with
`install.ps1`. Unlike the harness, it uses what a user has: Store `pwsh` as
the default profile, the user's PowerShell profile loaded, default host
settings.

Run every item in Windows Terminal, then in the VS Code integrated terminal:

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

Signing out or shutting down with a detached session is added as a further
item only if the lifecycle contract requires it.
