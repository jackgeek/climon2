# The monitor

How to use the monitor: switching between sessions, finding the ones that
need you, and what it does when things go wrong.

## Opening it

`c2 monitor`, or `c2 m`, opens the monitor. It finds every live session on its
own, and keeps finding new ones while it runs. It takes no arguments (any
argument is a usage error, status 2), and needs a terminal on stdin and
stdout. It always runs the monitor from the newest installed version.

The monitor watches each session through a **passive** connection: it gets
the session's output and status without attaching to it. It uses its own
full screen, with a status row along the top. While there are no sessions, it
shows:

```text
No session selected
                         No sessions to show.
                     Press Ctrl+] then x to exit.
```

Before it touches your terminal, it reads `config.jsonc` once: your
`client.escape_prefix` and `client.take_control_key` settings (or their
defaults) and the `monitor` section. See [configuration](configuration.md).
Every monitor action starts with the escape prefix, so the monitor refuses a
`null` prefix. A bad setting is reported, with status 125, before your
terminal changes. Editing the file while the monitor runs has no effect.

## Keys

Each of these is pressed after the escape prefix (Ctrl+] by default), in
lowercase:

| Key | Action |
| --- | --- |
| `s` | Open the session list (the **selector**) |
| `j` | Jump to the session that most recently started needing attention |
| `?` | Show the keys |
| `x` | Exit the monitor |

Every other key goes to the session you are viewing. An uppercase `J` is just
a `J`. Pasted text is never treated as a key, so pasting the prefix and `x`
does not exit, and a paste cancels a prefix you have started.

When you view a session, the monitor takes control of it: what you type goes to
that session, and the session takes the size of your terminal. A `null`
take-control key is fine here, because picking a session in the selector takes
control.

## The session list

The selector lists every session with its status on the left and its title
(or its command, if it has no title) on the right. Sessions that need
attention come first.

| Key | Action |
| --- | --- |
| Type or paste | Search by status and title |
| Left, Right, Home, End, Backspace | Edit the search |
| Up, Down, PageUp, PageDown | Move the highlight |
| Enter | Switch to the highlighted session |
| Esc | Cancel |
| Del | Delete the highlighted session |
| Click a row | Highlight it |
| Click outside | Cancel |
| Wheel | Scroll the list |

On macOS, Delete is shown as `Del (fn+⌫)`. Search is forgiving: the letters you
type must appear in order, but not next to each other. `bld` finds `Build`,
and `rgb` finds a `Running` session titled `Build`. The session you were
viewing keeps updating behind the list.

You can still view sessions that have disconnected or finished. Their history
is read-only.

**Deleting a session.** Del asks you to confirm, with Cancel as the default.
The confirmation is tied to that exact session, so it cannot hit a different
one. The session is ended, and it leaves the list once it has truly ended and
the monitor has cleaned up. It stays hidden only until the monitor exits.
The monitor does not switch you to another session in its place.

## Attention

A session **needs attention** when it is waiting for you. climon2 works this
out from the session's output. For example, a session that was busy printing
and has gone quiet for ten seconds needs attention: a build that finished, or
an agent waiting for an answer. Programs can also report their status to
climon2 directly. The
[session status specification](../behaviour/session-status.md) has the full
rules.

When a session starts needing attention, the monitor:

- shows a **toast**, a short notice in the bottom-right corner that names the
  session and the keys to jump to it (`Ctrl+] then j` by default);
- rings your terminal's bell, once per toast;
- tells your terminal that something needs attention, through its own
  [OSC 7501 reports](../protocol/README.md#monitor-to-host-status-and-bell).
  Your terminal can use this to flag the window or tab.

The escape prefix then `j` jumps to the session that most recently started
needing attention. That may not be the session the toast was about, if
another one came after it.

Attention is cleared when you go to the session on purpose: by picking it,
jumping to it, or closing the session list back to it. Highlighting it,
clicking a toast away, or just having the window in focus does not clear it.

Up to three toasts wait in line. If more arrive, one toast says
`More events occurred. Check sessions list` in their place. Click a toast to
dismiss it.

`monitor.bell` false turns off the bell, but not the toasts or the reports to
your terminal. `monitor.show_key_hints` false removes the key hints from
toasts and the rest of the screen.

## The key introduction

Until you turn it off, the monitor starts by showing its keys, using your
escape prefix:

```text
┌ Keys ───────────────────────────────────────────────────┐
│ Ctrl+] then x  Exit the monitor                         │
│ Ctrl+] then s  Open the session list                    │
│ Ctrl+] then j  Jump to the session that needs attention │
│ Ctrl+] then ?  Show this dialog                         │
│                                                         │
│ In the session list:                                    │
│ Type or paste  Search by status and title               │
│ Left, Right, Home, End, Backspace  Edit the search      │
│ Up, Down, PageUp, PageDown  Move the highlight          │
│ Enter  Select the highlighted session                   │
│ Esc  Cancel                                             │
│ Del  Delete the highlighted session                     │
│ Click a row  Highlight it                               │
│ Click outside  Cancel                                   │
│ Wheel  Scroll the list                                  │
│                                                         │
│ [ ] Don't show again (Space)                            │
│                                                         │
│ Continue (Enter)                                        │
└─────────────────────────────────────────────────────────┘
```

- Enter closes it.
- Space ticks **Don't show again**, or unticks it, and saves your choice at
  once. While it is ticked, the monitor starts without the dialog. It also
  starts without it while `monitor.show_key_hints` is false.
- The escape prefix then `?` always opens it again, as help. If you open help
  over the session list, closing it takes you back to your search.
- The dialog ignores every other key and any paste. The escape prefix then `x`
  still exits, and the escape prefix then `j` closes it and jumps.
- Pressing the escape prefix twice sends that key to the dialog. So with
  `Ctrl+M` (Enter) as the prefix, Enter Enter continues.
- A bare Escape closes it, unless `Ctrl+[` is your prefix. Then Escape is the
  prefix, so it cannot.

Your choice is saved in `<climon2 home>/monitor-preferences.json`, apart from
`config.jsonc`: `{"version":1,"suppress_key_introduction":true}`. Every
release can read it. The monitor writes it only when you change the box: it
writes a new file that only you can read, then renames it over the old one.
If the file cannot be read or is broken, the dialog opens and says why. If
saving fails, the dialog says so under the box. With key hints off, the empty
screen shows these messages instead.

## When things go wrong

If the monitor loses a session, that session's view shows `Reconnecting...`
while it tries again, waiting a little longer each time. It stops trying when
the session refuses for good or the config file is bad. It never resends
typing, requests or kills that may already have arrived. The monitor stays
open when there are no sessions, or all of them have finished.

When memory is short, a session's saved history shrinks, but its newest page
is always kept. The [operational limits](../behaviour/monitor/operational-limits.md)
set how much memory, time and work the monitor may use.

## Exiting

The escape prefix then `x` exits. Ctrl+C is just a key, not a way out.
SIGINT, SIGTERM and SIGHUP also exit cleanly. On the way out, the monitor
leaves every session it was controlling running, cleans up, and puts your
terminal back as it was, as [detaching](commands.md#sessions) does. It never
changes your terminal's title.

| Status | Meaning |
| --- | --- |
| 0 | Exited with the escape prefix then `x`, and everything was cleaned up |
| 128 + signal | Exited on a signal, and everything was cleaned up |
| 2 | You gave it an argument |
| 125 | A bad setting, a lost terminal, a fatal error, or cleanup that failed or stalled |

A command failing in one session never changes the monitor's exit status.
Errors are printed only after your terminal is restored. The monitor logs to
the general log in the climon2 home, including the settings it used. It never
logs what you type.
