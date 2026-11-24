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

Interactive shells are tested on GNU bash 5.x only. macOS `/bin/bash` 3.2 is
not supported for interactive sessions. `install.sh` still runs under
`/bin/bash`.

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

## Diagnostics

When climon2 itself fails, it exits 125 and names a log file. Logs hold
climon2's own messages. They never hold what is on the terminal, command
arguments or environment values.

- Release builds log at INFO level. `CLIMON2_LOG` takes a tracing filter to
  change that.
- Each session log is capped at 8 MiB.
- Every client run writes its own session log.
- Logs of finished sessions older than seven days are deleted.
