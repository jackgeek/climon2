# climon2

climon2 runs a terminal command inside a **session** that lives on its own.
You can leave a session (detach) and come back to it later (attach). You can
also end a session on purpose, or list the sessions that are still running:

```sh
c2 exec bash --noprofile --norc
c2 exec vim
c2 exec --headless make test
c2 attach 3f2a9c1e
c2 kill 3f2a9c1e
c2 list
c2 list --json
c2 monitor
```

Each session runs in a background process called the **session daemon**. The
terminal you work in connects to it as a **client**. Several clients can
connect to one session.

Every command has a one-letter shortcut that works exactly like the full name.
`-h` is short for `--headless`:

| Command | Shortcut |
| --- | --- |
| `exec` | `x` |
| `attach` | `a` |
| `kill` | `k` |
| `list` | `l` |
| `monitor` | `m` |

```sh
c2 x vim
c2 x -h make test
c2 a 3f2a9c1e
c2 l --json
c2 m
```

For help, run `c2 --help`. `-h` is taken by `exec`.

The command is called `c2`. There is no `climon2` command. The product is
still called climon2, so that name appears in the version output, in error
messages (`climon2: error:`), and in config paths, data paths and environment
variables.

Your command keeps its arguments, working directory and environment. The one
change is that it always sees `TERM=xterm-256color` and
`COLORTERM=truecolor`. When the command ends, every client still attached
exits with the command's exit status. You can run sessions inside sessions.
Session data lives in `~/.climon2` by default. Set `CLIMON2_HOME` to use
another folder; this folder is the **climon2 home**.

More documents:

- The [documentation index](docs/for-humans/README.md) lists every document
  written for people to read.
- The [protocol reference](docs/for-humans/protocol/README.md) is for people
  who write clients and, later, plugins. It covers how a client greets the
  daemon and the messages they exchange. It also covers the monitor's
  in-process client interface and the reports the monitor sends to the
  terminal it runs in. It marks what is built, what is agreed but not built,
  and what is still open.
- [Session status, attention, and activity](docs/for-humans/behaviour/session-status.md)
  explains how the daemon tracks each session's state, and how the monitor
  orders events and recovers from failures. Both the daemon and the monitor
  now do all of this.
- The [monitor acceptance criteria](docs/for-humans/behaviour/monitor/acceptance.md)
  say what proof a release needs: tests against a real monitor, tests of the
  packaged library, and measurements. Each crate's README maps these
  scenarios to its tests. A test listed there is not, on its own, proof that a
  release is ready.

## Sessions

`exec` starts your command and attaches you to it.

**Headless sessions.** `c2 exec --headless <command> [args...]` (or
`c2 x -h`) starts the command in a [headless session](GLOSSARY.md#sessions):
one you are not attached to. It needs no terminal, so scripts and CI can start
work in the background and come back to it later:

```sh
id=$(c2 exec --headless ./long-build.sh)
c2 attach "$id"
```

Like `exec`, it checks the whole config file first. It returns only once the
session is running. Then it prints the full session ID, in lowercase, plus a
newline, and exits 0.

If the session cannot start, you get the same statuses and messages as `exec`:

- 127: the command was not found.
- 126: the command could not be run.
- 125: climon2 itself failed, for example because of a bad config file.

If the ID cannot be written to stdout, the session still runs and the status
is still 0. `c2 list` will show it. The command's own exit status later
belongs to the session, not to this `c2` call.

The session starts at the size of the terminal on stdin, or 80x24 if there is
none. After that, the client in control sets its size, as in any session.
`--` still separates climon2's options from your command:
`c2 exec -- --headless` runs a command named `--headless`.

A headless session keeps running with no clients at all. That is true from
the start, after you detach, and after a client is lost by accident. It ends
only when its command exits or you run `c2 kill`. `c2 list` shows it as
`detached` whenever no client is attached.

**Detach** by pressing the **escape prefix**, Ctrl+], then `d`. The session
keeps running, with no time limit. The client prints this to stderr and
exits 0:

`climon2: detached from session <id>; reattach with: c2 attach <id>`

Other keys after the prefix:

- Ctrl+] Ctrl+] sends one Ctrl+] to your command.
- Ctrl+] then any other key sends both keys.
- A lone Ctrl+] waits for the next key, for as long as it takes.

Every other key goes to your command, including Ctrl+C and Ctrl+D. Inside a
bracketed paste (text your terminal marks as pasted), no key binding works.

**Attach** with `c2 attach <session>`. It needs a terminal on stdin and
stdout, and takes control at once. `<session>` is a full session ID, or the
start of one that matches just one live session. The start can be a single hex
digit. Case and hyphens do not matter.

- A badly formed `<session>` is a usage error (status 2).
- No match, or more than one, fails with 125. If the start of an ID matches
  more than one session, every match is listed.
- You cannot attach to the session you are running inside.

Each time you attach or take control, climon2 first pushes what is on your
screen up into your terminal's own scrollback. Then it redraws the session: its
saved history, screen, cursor, modes and title. If a full-screen app is
running, the normal screen behind it comes back too. A
`[climon2: older session scrollback was discarded]` line marks where old
history was dropped to stay under `scrollback.max_rows`.

**Several clients** can attach to one session. One of them, the
**controller**, sends input and sets the session's size. The others show this
placeholder and get no session output:

```text
This session is being viewed elsewhere.
Press Ctrl+T to take control
```

Press the take-control key (Ctrl+T) in a placeholder to take control. If
`take_control_key` is `null`, the placeholder shows only its first line. Other
keys typed in a placeholder are thrown away, and the escape prefix still
detaches. When you are in control, Ctrl+T goes to your command as usual.

The latest attach or takeover always wins, with no prompt. When the controller
leaves, control goes back to the client that had it most recently and is still
attached.

**What happens when clients leave:**

- If another client is still attached, the session goes on.
- If the last client detaches, the session runs until its command exits or it
  is killed.
- If the last client leaves any other way (terminal closed, client crashed,
  connection lost), an ordinary session ends at once. Its terminal is hung up,
  and any jobs still running are killed five seconds later. There is no grace
  period to reconnect. A headless session keeps running instead.
- A client also counts as lost if any of these happen:
  - its terminal takes no output for five seconds;
  - it falls more than 16 MiB behind;
  - it does not finish redrawing the session within 30 seconds, plus one
    second for each 4 MiB redrawn.
- An ordinary session ends if no client attaches within five seconds of its
  start. If its command exits before then, the exit is held for that first
  client until the five seconds are up. Headless sessions have no such limit.
- When the command exits, every attached client exits with its status.
- A [monitor](#monitor) watching a session keeps it alive by taking over.
  If the controller leaves, in any way, and no other client is attached,
  control passes to the monitor that started watching most recently. That
  monitor becomes an attached client in the background. It does not resize
  the session or type into it. The rules above apply only once no monitor is
  watching. Watching is not the same as attaching. So it does not stop a new
  session from ending under the five-second rule. And a session that is being
  killed passes control to no one. (The monitor watches every live session
  this way.)

**Kill** a session with `c2 kill <session>`. It picks the session the same way
`attach` does, and works whether or not clients are attached. It needs no
terminal and prints nothing.

The session hangs up its terminal, then always waits the full five seconds
before it kills any jobs left. So a kill usually takes about five seconds.
`c2 kill` exits 0 once the command has ended and the session is cleaned up. It
exits 125 if that is not confirmed within 10 seconds. Attached clients exit
with the command's status, which usually comes from the signal. On Linux, as
when you close a terminal window, a command that reads the terminal may see it
close before the hang-up signal arrives. It then ends with its own status
instead.

When a client stops showing a session (detach, exit, kill, or another client
taking over), climon2 puts your terminal back in a normal shell state. It:

- ends any synchronized update;
- leaves the alternate screen, but only if it entered it;
- resets margins, the origin, and the insert and new-line modes;
- turns autowrap on;
- turns off mouse, focus, bracketed-paste, application cursor-key and keypad
  modes;
- shows the cursor in its default style and resets text colours and styles.

Terminal settings (termios) are restored exactly.

**Exit statuses:**

| Status | Meaning |
| --- | --- |
| 0 | Detach or kill confirmed, or headless session created |
| The command's own | The command finished while the client was attached |
| 2 | Usage error |
| 125 | climon2 failed |
| 126, 127 | `exec` could not start the command |
| 128 + signal | The client was ended by a signal |
| 129 | The client's terminal went away |

## Configuration

climon2 reads an optional `<climon2 home>/config.jsonc`. These are the
defaults:

```jsonc
{
  "client": {
    // Ctrl+[ is Escape: it delays bare Escape, and in the monitor legacy Alt+s/x/j/? may trigger the shortcuts.
    "escape_prefix": "Ctrl+]",     // null disables escape interception in ordinary attach; the monitor refuses null
    "take_control_key": "Ctrl+T"   // null disables keyboard takeover
  },
  "scrollback": {
    "max_rows": 10000
  },
  "monitor": {
    "show_key_hints": true,        // false hides the monitor's implicit key hints, never a binding
    "bell": true,                  // false: no host bell when a toast's title first shows
    "limits": {
      "memory": {
        "total_bytes": 603979776,
        "client_bytes": 67108864,
        "history_bytes_per_session": 16777216,
        "history_bytes_total": 268435456,
        "queue_bytes_total": 67108864
      },
      "history": { "max_rows": 10000 },
      "resources": {
        "stack_bytes": 2097152,
        "max_workers_per_client": 2,
        "descriptors_per_client": 8,
        "descriptors_per_load": 4,
        "descriptor_headroom": 64,
        "release_bytes": 67108864
      },
      "queues": {
        "request_bytes": 262144,
        "request_records": 64,
        "event_bytes": 1048576,
        "event_records": 1024,
        "progress_bytes": 65536,
        "progress_records": 32,
        "deliveries": 8,
        "operations": 16
      },
      "transfer": { "chunk_bytes": 65536, "drain_records": 64, "drain_payload_bytes": 65536 },
      "payload": {
        "wire_frame_bytes": 1048576,
        "metadata_record_bytes": 16384,
        "title_bytes": 4096,
        "notification_title_bytes": 4096,
        "notification_body_bytes": 8192,
        "assembly_records": 32,
        "assembly_bytes": 65536
      },
      "scheduling": { "probes": 8, "loads": 2, "starts": 8, "newer_start_burst": 8 },
      "discovery": {
        "pass_ms": 30000,
        "stall_ms": 5000,
        "batch_records": 128,
        "batch_bytes": 1048576,
        "metadata_file_bytes": 262144,
        "metadata_depth": 32
      },
      "deadlines": {
        "load_ms": 10000,
        "start_ms": 10000,
        "operation_ms": 5000,
        "kill_ms": 10000,
        "restoration_stall_ms": 5000,
        "restoration_total_ms": 60000,
        "delivery_stall_ms": 5000,
        "leave_ms": 5000,
        "cleanup_ms": 10000,
        "shutdown_ms": 10000,
        "final_host_ms": 1000,
        "assembly_ms": 5000
      },
      "retry": { "initial_ms": 1000, "cap_ms": 30000, "jitter_percent": 20, "admission_ms": 1000 },
      "ui": {
        "fps": 30,
        "client_quantum_ms": 2,
        "background_quantum_ms": 8,
        "search_bytes": 4096
      }
    }
  },
}
```

**The file format.** It is JSON that also allows `//` and `/* */` comments
and trailing commas. A comment counts as a space: it can split two tokens but
never joins them. So `1/* c */0000` is an error, not `10000`.

**Missing and unknown settings.** A missing file, object or field means the
default. Fields climon2 does not know are ignored.

**Keys.** A key setting can be `Ctrl+A` to `Ctrl+Z`, `Ctrl+[`, `Ctrl+\`,
`Ctrl+]`, `Ctrl+^`, `Ctrl+_` or `Ctrl+Space`. `null` turns the binding off.
`take_control_key` cannot be `Ctrl+[` (that is Escape), and the two bindings
cannot be the same key. If `escape_prefix` is `null`, there is no detach key,
so closing the last client ends the session.

**Scrollback.** `scrollback.max_rows` (0 to 100000, default 10000) is how many
rows of normal-screen history a session keeps for redrawing. It is set when the
session starts; attaching never changes it. 0 keeps no history, but the screen
itself is still redrawn.

**Key hints.** When `monitor.show_key_hints` (default true) is false, the
[monitor](#monitor) hides the key hints it adds on its own. It never hides a
binding or text from an app. Hints appear in the empty screen, the top row,
the placeholder, the session list footer and toasts. False also stops the key
introduction from opening at start-up. The escape prefix then `?` still opens
it. Your **Don't show again** choice is stored apart from this setting and
never changes it.

**Bell.** When `monitor.bell` (default true) is false, toasts do not ring
your terminal's bell. Toasts and the monitor's attention reports to your
terminal still appear.

**Monitor limits.** `monitor.limits` holds the
[operational limits](docs/for-humans/behaviour/monitor/operational-limits.md):
sizes in bytes, times in milliseconds (`_ms`), and counts. The monitor checks
and reports all of them, and enforces most. That document says which ones it
does not enforce yet. `history.max_rows` here is the monitor's own history; it
never changes `scrollback.max_rows`.

Each limit is a whole number in this range:

- Times: 1 to 3600000. But `ui.client_quantum_ms` and
  `ui.background_quantum_ms` are 1 to 50.
- `history.max_rows`: 200 to 100000. `ui.fps`: 1 to 120.
  `retry.jitter_percent`: 0 to 50. `resources.max_workers_per_client`: 0 or
  more.
- Everything else: at least 1. The upper bound is 4294967295 for counts, for
  `transfer.chunk_bytes`, and for the `payload` byte limits other than
  `assembly_bytes`. Other byte limits go up to 18446744073709551615.

The limits must also fit together:

- These must each fit inside `memory.total_bytes`: `memory.client_bytes`,
  `history_bytes_total`, `queue_bytes_total`, the `resources.stack_bytes` and
  `release_bytes` reserves, and every monitor byte limit not already inside a
  smaller total (`transfer.drain_payload_bytes`, `discovery.batch_bytes`,
  `discovery.metadata_file_bytes` and `ui.search_bytes`).
- `history_bytes_per_session` must fit inside both `history_bytes_total` and
  `client_bytes`.
- `queues.request_bytes` and `event_bytes` must each fit inside both
  `memory.queue_bytes_total` and `memory.client_bytes`.
- `payload.assembly_bytes` and `assembly_records` limit how the daemon builds
  notifications. They are not monitor memory, so no monitor total includes
  them.
- `queues.progress_bytes` must be less than `event_bytes`, and
  `progress_records` less than `event_records`. This leaves room for normal
  work.
- `transfer.chunk_bytes` must fit inside `queues.request_bytes`, inside the
  normal event space (`event_bytes` minus `progress_bytes`), and inside
  `transfer.drain_payload_bytes`. `payload.metadata_record_bytes` must fit
  inside `progress_bytes`, that normal event space, and
  `drain_payload_bytes`.
- `ui.client_quantum_ms` must not be more than `ui.background_quantum_ms`, and
  `retry.initial_ms` not more than `retry.cap_ms`.

`memory.client_bytes` is shared, per session, between the client library and
the monitor's copy of the session's terminal (its **terminal model**). The
library's share is its request and event queues, one wire frame, and its
1048576-byte instance memory. That is 3407872 bytes by default, and it must fit
inside `client_bytes`. The terminal model gets the rest.

The monitor also refuses limits too small for one session's terminal model at
the largest session size, 500x200. (`exec` does not check this.) Such a model
needs 16966087 bytes for its current screen plus the history that holds its
latest page. So:

- `memory.client_bytes` must hold that plus the library's share (20373959
  bytes by default).
- `memory.total_bytes` must hold all that plus the monitor's one shared parser
  (22471111 bytes by default).
- `memory.history_bytes_total` and `history_bytes_per_session` must hold that
  history: 12115200 bytes.

The error names the first limit to raise. Limits that only just pass will let
such a session in. But to redraw it next to its current model, the model's
share of a client's memory needs at least 33932174 bytes (36029326 in the total
with the parser, plus the library's share). The monitor redraws whenever the
session changes size.

**Who reads what.** `exec` checks the whole file before it starts the session.
`attach` checks only `client`. The monitor checks `client` and `monitor`, and
refuses a `null` `escape_prefix`. `list` and `kill` do not read the file.

Any error exits 125 before anything starts. Errors include: bad JSONC,
repeated keys, wrong types, `null` for a setting that cannot be turned off,
bad key names, two bindings on one key, values out of range, limits that do
not fit together, and, for the monitor, limits too small for its terminal
model.

Each command reads the file once. There is no environment variable to
override it, it is never created for you, and changes are not picked up while
a command runs.

### Monitor behavior

The [monitor](#monitor) follows the
[agreed terminal integration](docs/for-humans/protocol/terminal-integration.md).
That covers keyboard input and drawing, its own history of each session's
normal screen, one status row at the top, and clear rules for when it takes
control and when it restores your terminal. The monitor does all of this.

It reads the config file once, before it touches your terminal. It allows a
`null` take-control key, because choosing a session in the session list can
take control instead.

Keys, each pressed after the escape prefix, in lowercase:

- `s` opens the session list (the **selector**).
- `x` exits.
- `j` jumps to the most recent connected session that needs attention. This
  may not be the session a toast was about.
- `?` opens the
  [key introduction and help dialog](docs/for-humans/behaviour/monitor/selector.md#key-introduction-and-help).

An uppercase `J` is not a key binding; it is passed on as plain input. Pasted
shortcuts are always treated as text.

A **toast** is a short pop-up notice. When a session needs attention, its
toast shows your actual prefix plus `j`, and says it jumps to the most recent
such session. It shows no hint when `monitor.show_key_hints` is false.

If you open help over the selector, the selector keeps its search and
highlight. Closing help takes you back to them. Help never takes control or
marks anything as seen.

`monitor.bell` false stops the monitor ringing your terminal's bell. Toasts
and OSC 7501 reports still happen. Whether you hear a bell is up to your
terminal's own settings.

The [selector specification](docs/for-humans/behaviour/monitor/selector.md)
covers the layout, read-only viewing of history from disconnected or finished
sessions, and animated toasts.

The later [lifecycle specification](docs/for-humans/behaviour/monitor/lifecycle.md)
covers deleting sessions and recovering from failures. The monitor does both.

**Deleting a session.** Delete asks you to confirm, with Cancel as the
default. The confirmation is tied to that exact session, so it cannot hit a
different one. Removal waits until the session has truly ended and the
monitor has cleaned up its own copy. A removed session stays hidden for the
rest of this run only. The monitor never switches you to another session in
its place, and never brings back the removed session's ID in the same run.
There is no way to look at a connected session's cache after it is revoked,
and there are no prefix+`h` or prefix+`r` keys.

**Recovering from failures.** While the monitor tries to reconnect on its
own, a disconnected session's read-only view shows exactly
`Reconnecting...`. Failures it can recover from are retried, waiting longer
each time (exponential backoff). These include connection and start-up
failures, such as a missing library, and worker crashes it can safely survive.
A permanent refusal or a bad config file stops the retries. A new connection
waits for the old one to be fully cleaned up. The monitor never resends input,
requests or kills when it does not know whether they arrived. It stays open
when there are no sessions or all of them have finished. It keeps the same
session selected, and never takes control of a replacement on its own.

**Shutting down.** Prefix+`x` and catchable signals shut down every session
connection the monitor owns, all at once. The monitor checks each step on its
own: the daemon accepted the leave, the connection closed, the final result
arrived, cleanup finished, and the worker thread ended. It restores your
terminal and clears its attention report before it prints any error. If
cleanup stalls, the whole process fails within a time limit. It never falsely
claims a detach or a finished thread, and never kills a thread.

The agreed
[operational limits](docs/for-humans/behaviour/monitor/operational-limits.md)
set the `monitor.limits` budgets and deadlines, and say how to prove them.
The monitor enforces most of them, and that document lists the ones it does
not. When memory is short, a session's local history shrinks to its newest
page but never vanishes. Sessions held back by resource limits stay visible.
The monitor does not try to keep sessions alive in general, and does not
isolate crashes in native code.

**Exit status.** Exiting on purpose succeeds (0) only once all required leave
steps, cleanup and terminal restore have succeeded. The monitor never exits
with the selected command's status. It exits 125 for a fatal monitor error, a
leave that failed or whose result is unknown, unfinished cleanup, or a failed
restore. A catchable signal gives a signal-style status, but only after
cleanup succeeds. A command or client failing in one session does not spoil a
later clean exit.

The monitor also sends your terminal its own
[OSC 7501 attention reports](docs/for-humans/protocol/README.md#monitor-to-host-status-and-bell),
which sum up all sessions. It never passes on a session's raw escape
sequences.

## Listing

`c2 list` shows live sessions, newest first, in these columns: SESSION,
CREATED, VERSION, STATE, CLIENTS, COMMAND and ARGS.

- STATE is `starting`, `attached`, `detached` or `terminating`. It is
  `unknown` for a session that does not report it. A headless session is never
  `starting`: it is `detached` whenever no client is attached.
- CLIENTS counts attached clients, or shows `-` when unknown.

A monitor can watch a session without attaching to it (a **passive**
connection). That does not count as attaching. It never adds to CLIENTS or
changes STATE until the monitor takes control or inherits it (see
[what happens when clients leave](#sessions)). From then on it counts as an
attached client.

`c2 list --json` prints an array of objects, one per session. Each has a
`state` key holding `{"lifecycle": …, "attachments": …, "controlled": …}`, or
`null` when the session does not report a valid state.

Each session is attached and killed through the client of its own climon2
version, so that version must still be installed. Older sessions that do not
support an action fail with 125 and are left alone. `list` needs no client.

## Monitor

`c2 monitor`, or `c2 m`, opens the **monitor**: one terminal that keeps track
of every live session. It takes no arguments (any argument is a usage error,
status 2), and needs a terminal on stdin and stdout. It always runs the
monitor from the current version, on macOS and x86-64 GNU/Linux alike.

Before it touches your terminal, it reads `config.jsonc` once. It uses your
`client.escape_prefix` and `client.take_control_key` settings (or their
defaults) and the `monitor` section (see [Configuration](#configuration)). It
refuses a `null` escape prefix, because that prefix is the only way to reach
its actions. Start-up errors, including bad settings, are reported with status
125 before your terminal changes. Editing the file while the monitor runs has
no effect.

The monitor keeps finding live sessions and gives each one a passive
connection. It uses its own alternate screen, with one top row that is always
there. While there are no sessions, it shows an empty screen with a key hint
that names your escape prefix. With `monitor.show_key_hints` false, only
`No sessions to show.` remains:

```text
No session selected
                         No sessions to show.
                     Press Ctrl+] then x to exit.
```

**Exiting.** The escape prefix then `x` exits with status 0. Any other key is
ignored, and Ctrl+C is just a key, not a way out. Pasted text is never a key:
pasting the prefix and `x` does not exit, and a paste cancels a prefix you have
started. SIGINT, SIGTERM and SIGHUP exit the same way, with status
128 + signal. If your terminal hangs up or can no longer be read, the monitor
exits with 125.

Resizing redraws the screen at the new size without blanking it.

Every time it exits, the monitor puts your terminal back in the normal shell
state described for clients above. It restores terminal settings (termios)
exactly and never changes your terminal's title. Only after that does it print
any error, with status 125.

It writes to the general log in the climon2 home, including its settings in
effect as JSON. It never logs what you type.

**The key introduction.** Until you turn it off, the monitor starts by showing
its keys over the empty screen, using your escape prefix. It also lists the
session list's own keys. (On macOS, Delete is shown as `Del (fn+⌫)`.)

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

- Enter goes on to the empty screen.
- Space ticks **Don't show again**, or unticks it, and saves your choice at
  once. While it is ticked, later starts with the same climon2 home go straight
  to the empty screen. The introduction also stays closed while
  `monitor.show_key_hints` is false.
- Whatever those say, the escape prefix then `?` opens the same dialog as
  help, showing your saved choice. Doing this while it is already open does
  nothing.
- The dialog takes every other key and any paste, and none of them does
  anything else. The escape prefix then `x` still exits.
- Pressing the escape prefix twice sends that key to the dialog. So with
  `Ctrl+M` (Enter) as the prefix, Enter Enter continues.
- While the dialog is open, the escape prefix then `s` does nothing. Once it
  is closed, it opens the session list.
- The escape prefix then `j` closes the dialog and jumps to the session that
  needs attention.
- A bare Escape closes the dialog when the next key arrives. The exception is
  when `Ctrl+[` is the prefix: then Escape is the prefix, so it cannot close
  the dialog.

**Where the choice is saved.** It lives in
`<climon2 home>/monitor-preferences.json`, apart from `config.jsonc`:
`{"version":1,"suppress_key_introduction":true}`. Every release reads it,
whatever its version, and ignores fields it does not know.

The monitor writes it only when you change the box. It writes a new file that
only you can read (`monitor-preferences.json.<pid>.tmp`), then renames it over
the old one. Before that, it deletes any such temporary file left by a monitor
that stopped before its rename and is no longer running.

If the file cannot be read or is broken, the introduction opens and shows why
under its unticked box. (With key hints off, the empty screen shows the reason
instead.) The file is left as it is until you change the box. If saving fails,
the dialog says why under the box, and the box keeps showing the saved choice.
With key hints off, the empty screen says so too. The general log records
these problems and every saved choice.

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

## Install

climon2 runs on:

- **macOS 15 or newer**, on Apple Silicon and Intel. The macOS programs are
  universal and ad-hoc signed. They are not signed with a Developer ID or
  notarised.
- **x86_64 GNU/Linux** with glibc 2.39 or newer, tested on Ubuntu 24.04.

Arch Linux is supported for development. musl and Linux arm64 are not tested.
See [distribution](docs/for-humans/distribution.md) for the archives for each
platform.

Latest release:

```sh
curl -fsSL https://github.com/jackgeek/climon2/releases/latest/download/install.sh | /bin/bash
```

A specific release (replace `0.1.0` with the version you want):

```sh
curl -fsSL https://github.com/jackgeek/climon2/releases/download/v0.1.0/install.sh | /bin/bash
```

Each script downloads only its own version and checks its SHA-256 checksum
before it runs the installer.

The programs go in the **executable home**, `~/.local/share/climon2` by
default. To pick another folder:

```sh
curl -fsSL https://github.com/jackgeek/climon2/releases/latest/download/install.sh |
  /bin/bash -s -- --dir "$HOME/tools/climon2"
```

Add the executable home to your PATH:

```sh
export PATH="$HOME/.local/share/climon2:$PATH"
```

For a custom folder, the installer prints the matching line. It never edits
your shell start-up files. Upgrades keep the existing version folders and
running sessions. Every install replaces `c2`, the launcher, in the executable
home. You do not need `sudo` or Homebrew.

If you download on macOS with a browser, check `SHA256SUMS`, unpack the
archive, then remove the quarantine flag from the unpacked folder before you
run its installer:

```sh
shasum -a 256 -c SHA256SUMS
tar -xzf climon2-0.1.0-macos-universal.tar.gz
xattr -dr com.apple.quarantine climon2-0.1.0-macos-universal
./climon2-0.1.0-macos-universal/install
```

To install by hand on Linux, use the `linux-x86_64-gnu` archive and
`sha256sum`. There is no quarantine step. Linux needs procfs, and a file
system at the install path that can swap two folders with `renameat2`.
Processes that share a climon2 home must share the same PID namespace. Sharing
session data across containers or PID namespaces is not supported.

## Diagnostics

When climon2 itself fails, it exits 125 and names a log file. Logs hold
climon2's own messages. They never hold what is on the terminal, command
arguments or environment values.

- Release builds log at INFO level. `CLIMON2_LOG` takes a tracing filter to
  change that.
- Each session log is capped at 8 MiB.
- Every client run writes its own session log.
- Logs of finished sessions older than seven days are deleted.

## Development and releases

Two diagrams show how the code fits together:

- [Executables and dynamic libraries](https://github.com/jackgeek/climon2/blob/main/docs/for-humans/architecture/runtime-artifacts.md)
  shows the programs that run, the client library, the test tools, and how the
  monitor loads client libraries.
- The [workspace crate diagram](https://github.com/jackgeek/climon2/blob/main/docs/for-humans/architecture/crate-relationships.md)
  shows which climon2 crates use which, including test tools.

```sh
cargo test --workspace --locked
cargo xtask setup-quality
cargo xtask check
cargo xtask check --print-mutation-plan
cargo xtask check --full
cargo xtask check --full --mutate-all
cargo xtask dist
cargo xtask install
cargo xtask install --dir "$HOME/tools/climon2"
```

### What you need

- The pinned Rust toolchain, a C compiler and libclang.
- ShellCheck, Python 3.11 or newer, GNU bash 5.x, vim and less.

Interactive tests use `CLIMON2_TEST_BASH` if set. Otherwise they look for
Homebrew bash on macOS, or `/bin/bash` and `/usr/bin/bash` on Linux. macOS
`/bin/bash` 3.2 is not used for these tests. If bash 5 is missing, CI fails;
locally the tests are skipped and the skip is reported. On Arch Linux, use
`CLIMON2_TEST_BASH=/usr/bin/bash`. `install.sh` still works with macOS
`/bin/bash` 3.2.

On arm64 macOS, add both Rust targets before you run workspace tests, `dist`
or `install`:
`rustup target add aarch64-apple-darwin x86_64-apple-darwin`.

### What CI runs

On Linux, CI runs the full runtime tests, real-terminal tests and tests of the
packaged build. It runs them on both macOS architectures too, but only while
the repository variable `CLIMON2_MACOS_TESTS` is `on`. Otherwise macOS only
builds its release archive. Windows runs only the tests that do not depend on
a platform.

Three agreed documents describe Windows support to come:

- [Windows host and shell qualification](docs/for-humans/windows/qualification.md)
  says what a Windows release will have to pass.
- [Windows quality and CI qualification](docs/for-humans/windows/quality.md)
  says how CI will check it.
- The [Windows platform and lifecycle contract](docs/for-humans/windows/lifecycle.md)
  says how its processes will start, run and end.

Two quality checks run on a dedicated Linux development machine, not in CI:

- **CRAP**, a score that combines a function's complexity with how much of it
  the tests cover.
- **Mutation testing**, which makes small changes (**mutants**) to the code
  and checks that some test fails for each one. A mutant no test catches is a
  **survivor**.

That machine runs them for shared and Linux-only code, with 8 mutation
workers.

The macOS quality job in CI also needs `CLIMON2_MACOS_TESTS` on. It runs
`cargo xtask mutate --native-only`. CRAP scores only the source files
registered as macOS-only, and mutation picks the changed files among them.
Changes to shared code alone skip this job. Release tags use
`--native-only --mutate-all` to check every macOS-only file. Coverage still
runs the whole workspace, so real callers are exercised. Mutation still needs
the unchanged whole workspace to pass first in a worker. Native CI uses as
many CPUs as it has, up to 16.

### The quality gate

`xtask check` runs: formatting, strict clippy, unit, integration and
packaged tests (built by the installer), ShellCheck, append-only metadata
fixtures, proofs that instrumentation and worker isolation work, a CRAP score
of 5 or less on every shipped function (using coverage), and mutation testing
with no survivors.

It comes in two levels:

- The **ticket gate**, `cargo xtask check`, measures load once (see below) and
  mutates only the functions your change adds or edits.
  - A function, `const` or `static` counts as changed if it is new, or if the
    base has no item with the same path and the same tokens. A path looks like
    `<Pty as Drop>::drop` or `outer::inner`.
  - Whitespace, comments and doc comments do not count. So functions that only
    moved, or only had their docs edited, are not mutated.
  - Nested items are compared on their own, including those inside `const` and
    `static` values. An item owns every line its own code is on.
  - Editing an impl or trait header (generics, bounds, where clause, self type,
    trait path or attributes), or the attributes of the module around it,
    changes every item inside.
  - Renamed files are compared with their old path. Added, copied and
    untracked files are mutated whole. Test-only code is skipped.
  - One mutation run covers every file with a chosen site, so the whole
    workspace baseline runs only once. Its `--lines` option applies to every
    file, so it gets all the lines that hold chosen sites in any file. These are
    found with the mutator's `--scan`, which runs no tests. A file may then
    also mutate a site on a line number chosen for another file. That is safe:
    it only ever mutates more, never less.
  - `cargo xtask check --print-mutation-plan` prints this choice and exits.
- The **full gate**, `cargo xtask check --full`, measures load five times and
  mutates whole changed files. Run it once before a feature branch merges into
  `main`, with `CLIMON2_MUTATION_BASE=origin/main`.

`--mutate-all` mutates every shipped file whole, at either level.
`cargo xtask mutate` also mutates whole files, and does not measure load.

**Build check.** Like `cargo xtask test`, the gate's tests first run the
monitor's build test alone. This proves the plain monitor binary has no test
hooks. Only then do the workspace tests build it with hooks at the same path.

**How a mutant is tested.** The unchanged workspace must pass inside a real
mutation copy before mutation starts. A worker that breaks for its own reasons
can never count as a caught mutant. Each mutant runs in stages:

1. the workspace library tests;
2. its own crate's integration tests;
3. the whole workspace.

A failing stage stops the run at once. A mutant that passes the first two
stages still runs the whole workspace before it can count as a survivor. The
unchanged baseline always runs the whole workspace.

**Stage time limits.** After coverage, with nothing else running, the gate
times each of the first two stages for every target once, in the warm
`target/mutation-tests` build. It writes the **stage time limits** (10 times
each run, at least 2 s) to `target/mutation-stage-timeouts.json`. A mutant
builds each of those stages, then runs its tests under that stage's limit. If
a stage runs past it, the whole process group is killed and the mutant counts
as caught, the same as with the mutator's own time limit. So a test that hangs
a mutant's library stage frees its worker in seconds, not after 10 whole
workspace runs. The whole-workspace stage has no stage limit. The mutator's
limit for the whole run (10 times the unchanged whole-workspace run, at least
2 s) still applies.

**Load measurement.** Both levels measure how tests behave under load before
mutating, even when no changed shipped code needs mutating. The gate first
times one whole-workspace run with nothing else running, in the warm build.
Then it runs its rounds (one for the ticket gate, five for the full gate). Each
round runs 8 suites at once, each in its own copy of the warm build
(copy-on-write where the file system supports it). Each suite acts out a
mutant's run on the unchanged workspace, in its own session: the first two
stages in order, each under its stage limit, then the whole workspace, all
inside the mutator's limit.

A suite fails if any stage has a failing test or runs past its limit. Either
result would count a mutant as caught and hide a survivor. A failure under
load is a bug in the tests, so nothing is retried. The measurement stops after
the failing round. It names each failing test and test target, how many suites
each failed in, and the limit each killed suite hit. Then it lists their logs.
Logs from every run stay in their own folder under `target/flake-logs/`.
Worker builds are deleted afterwards. The next measurement deletes any left by
one that was killed.

Workers get their own copies of the warm build (copy-on-write where
supported). They do not rebuild dependencies from scratch for each changed
file.

**Choosing the base.** `CLIMON2_MUTATION_BASE` sets what the changes are
compared with (default `HEAD`). Both levels compare with the merge base of
that revision and `HEAD`. So a base like `origin/main` that has moved on does
not pull in its own changes. For committed work, set it to the target branch
or the merge base, for example
`CLIMON2_MUTATION_BASE=origin/main cargo xtask check --full`.

Code for one platform must live in source files registered for that platform.
Shared files must not hide macOS-only code that Linux cannot check.

### Building release archives

`xtask dist` needs either an **arm64 macOS host** with both Rust macOS
targets, or an **x86_64 GNU/Linux host**.

- macOS builds keep deployment target 15.0, universal binaries and signature
  checks.
- Linux builds check that the programs are native ELF files. Published Linux
  builds are made on Ubuntu 24.04, not Arch, to set the lowest glibc version
  they need.

Both builds keep the packages without test hooks apart. They write the
platform's archive, a stamped `install.sh` and `SHA256SUMS` to `target/dist/`.
Each version folder also holds the client library: `libclimon2_client.dylib`
on macOS or `libclimon2_client.so` on Linux. It must export only its start-up
function and report exactly the archive's version. The Linux library must also
have no soname, RPATH or RUNPATH, and need only glibc's libraries.
`release-check` runs these library checks on the local per-package release
build too. Set `CLIMON2_E2E_ARCHIVE` to the archive to test it as it is.

On an **x86_64 Windows host**, `xtask dist` builds the Windows zip, the
download script `install.ps1`, and their `SHA256SUMS` instead, with the pinned
ConPTY files. It fails if any shipped program or library needs the Visual C++
runtime. The zip's `install.exe` installs it, but it is not published yet. See
[Windows packaging and upgrades](docs/for-humans/windows/distribution.md).

`xtask install` builds and checks the same release files as `dist`, then runs
the installer without making an archive. It needs the same host and tools as
`dist`. It installs to `~/.local/share/climon2` by default; `--dir <directory>`
picks another folder. It keeps other versions and prints a PATH hint when
needed. It does not touch shell start-up files.

### Releasing

Before a release, the maintainer must run
`cargo xtask check --full --mutate-all` locally on the exact release commit and
see it pass. CI does not repeat it.

A `v<workspace version>` tag runs:

1. full macOS-only mutation, and the native runtime and archive checks (on
   macOS only while `CLIMON2_MACOS_TESTS` is on);
2. [release qualification](docs/for-humans/distribution.md#release-qualification)
   of the exact archives on all three platforms, whatever that variable says.

The same final macOS archive must pass on both architectures. The Linux
archive must pass on Ubuntu 24.04, under bash 5, with vim and less. Both
platforms must pass before one GitHub Release and its provenance attestation
are published.

`-rc.N` tags are prereleases. From the second published release on, it must
work with the release before it. The first release has nothing to be
compatible with.

**No Intel runner, no release.** If GitHub retires the `macos-15-intel`
runner, a new native Intel macOS 15+ runner must pass the same archive tests.
A missing runner, or passing on arm64 alone, does not prove Intel support.

## License

climon2 is open source software under the [MIT License](LICENSE).
