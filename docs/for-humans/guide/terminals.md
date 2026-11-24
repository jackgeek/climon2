# Terminals, limitations and logs

Which terminals and shells climon2 supports, what it cannot do, and where it
logs problems.

## Terminal compatibility

The session daemon keeps its own copy of each session's terminal, built on
Alacritty's `alacritty_terminal` 0.26.0. It answers your command's questions
about the terminal itself, and never asks your real terminal anything.
`TERM=xterm-256color` and `COLORTERM=truecolor` describe the part of xterm
that climon2 is tested against, not all of xterm. They replace any values your
command would otherwise inherit.

On macOS and Linux, interactive shells are tested on GNU bash 5.x only. macOS `/bin/bash` 3.2 is
not supported for interactive sessions. `install.sh` still runs under
`/bin/bash`.

### Windows

On Windows, climon2 is tested in Windows Terminal and in the VS Code
integrated terminal with its default settings. The tested shells are
PowerShell 7 (`pwsh`) installed from its MSI package, Windows PowerShell 5.1
(`powershell`) and `cmd.exe`. Other terminals, such as the
classic console window or an SSH session, and other shells are not tested but
not blocked. `exec`, `attach` and `monitor` need a console on stdin and stdout:
redirected input or output is refused as a missing terminal is on Unix, with
125.

How the commands behave on Windows is in [Commands](commands.md#on-windows).

## Limitations

- **Only basic keyboard input.** Requests for the Kitty keyboard protocol or
  modifyOtherKeys are removed. Application keypad keys may not come through
  exactly.
- **Unmarked pastes look like typing.** Only bracketed pastes skip key
  bindings. A paste that is not bracketed and contains Ctrl+] or Ctrl+T can
  trigger them.
- **Bindings hide keys from apps.** The escape prefix is always caught (press
  it twice to send it). The take-control key is caught in the placeholder. You
  can change or turn off both in the config file.
- **Repeats on reattach.** Every attach sends all saved history again, so your
  terminal's scrollback can hold copies of it.
- **Not restored on attach:** scroll region, tab stops, character sets,
  colour palette changes, keyboard protocol modes and hyperlinks. Your
  terminal's own title is not restored when you leave. It keeps the session's
  title (by default `c2 exec <command> <args...>`).
- **Saved cursor on the alternate screen.** If a session is redrawn while an
  app is on the alternate screen, the normal screen's saved cursor is set to
  the current cursor. They usually match, but can differ after a resize while
  that screen was hidden.
- **Live output is filtered.** climon2 either answers terminal queries or
  removes them. It removes OSC 52 clipboard access, palette changes, every
  other OSC sequence (such as OSC 7 and shell-integration marks), and DCS,
  APC, PM and SOS strings. Hyperlinks (OSC 8) show as plain text. Control
  characters in titles are percent-encoded. Control strings and CSI sequences
  longer than 4096 bytes are dropped.
- **Session size is 2 to 500 columns by 1 to 200 rows.** On a bigger terminal
  the session sits in the top-left corner. It is drawn from the daemon's copy
  of the screen instead of being passed straight through. The scroll region is
  not kept when switching between the two. Terminals under 2 columns wide are
  not supported.
- **Only attaching to yourself directly is caught.** Other loops, such as two
  sessions each attached inside the other, are up to you to avoid.
- **Little is restored after a crash.** If the session daemon vanishes, the
  client can only reset text styles, cursor visibility, mouse, focus,
  bracketed-paste, cursor-key and keypad modes, and synchronized output, then
  restore termios. It cannot leave the alternate screen or restore anything
  else.
- **Combining marks have no limit.** Alacritty 0.26.0 stores a cell's
  combining marks without a cap, so a stream of them makes the daemon use more
  and more memory. Test results for other Alacritty versions do not apply to
  0.26.0.
- **Windows has limitations of its own.** They are listed under
  [Windows limitations](#windows-limitations).

### Windows limitations

- **The Microsoft Store build of PowerShell 7 cannot run in a session.**
  Windows refuses to start a Store app inside a session's job object. Install
  PowerShell 7 from its MSI package, or use `powershell` or `cmd.exe`, as the
  session's shell. Running `c2` from the Store build is not affected.
- **Working directories over 260 characters.** `c2 exec` cannot start a command
  in one, whatever the long-path setting: it exits 126 with "command cannot be
  executed".
- **Ctrl+Break does nothing in the VS Code integrated terminal.** That terminal
  delivers neither an event nor a key for it. In Windows Terminal, Ctrl+Break
  reaches the session's command as it would outside climon2.
- **Sign-out and shutdown end every session.** Windows ends every process you
  own, and no exit status is promised. The next `c2 list` cleans up what they
  leave behind.
- **The programs are not code-signed.** A zip downloaded with a browser and
  extracted in Explorer carries Windows' mark for files from the internet, so
  SmartScreen may warn when `install.exe` runs; run `Unblock-File` on the zip
  before extracting it. Files installed with `install.ps1` carry no such mark.
  Where Smart App Control is enforcing, it may block unsigned programs
  whatever their origin. Integrity rests on `SHA256SUMS` and the release's
  build-provenance attestations.
- **What the bundled ConPTY adds is dropped.** A session's pseudoconsole starts
  its output by negotiating with a terminal: `ESC [ 1 t`, `ESC [ c`,
  `CSI ? 1004 h` (focus reporting on) and `CSI ? 9001 h` (win32-input-mode
  on). climon2 drops these from the start of the output, so your terminal never
  receives them, they are not in the screen restored on attach, and
  win32-input-mode is never turned on. From the command's first output on,
  output passes on unchanged.
- **Keys plain VT input cannot express arrive as their plain form.** climon2
  reads your terminal's keys as VT sequences, so Shift+Enter, for example,
  arrives in the session as Enter.
- **Focus reports wait for the session to ask.** A session receives focus
  changes as `CSI I` and `CSI O` only after it turns focus reporting on, as on
  Unix. Your terminal's own pseudoconsole passes on climon2's "focus reporting
  off" and then turns it straight back on for itself.

## Diagnostics

When climon2 itself fails, it exits 125 and names a log file. Logs hold
climon2's own messages. They never hold what is on the terminal, command
arguments or environment values.

- Release builds log at INFO level. `CLIMON2_LOG` takes a tracing filter to
  change that.
- Each session log is capped at 8 MiB.
- Every client run writes its own session log.
- Logs of finished sessions older than seven days are deleted.
