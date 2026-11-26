# Commands

Every `c2` command: starting, attaching to, detaching from, ending and listing
sessions, and showing or releasing the climon2 home's owner.

```sh
c2 exec bash --noprofile --norc
c2 exec vim
c2 exec --headless make test
c2 attach 3f2a9c1e
c2 kill 3f2a9c1e
c2 list
c2 list --json
c2 monitor
c2 home
c2 home release
c2 home release --force
```

Every command has a one-letter shortcut that works exactly like the full name.
`-h` is short for `--headless`:

| Command | Shortcut |
| --- | --- |
| `exec` | `x` |
| `attach` | `a` |
| `kill` | `k` |
| `list` | `l` |
| `monitor` | `m` |
| `home` | `h` |

```sh
c2 x vim
c2 x -h make test
c2 a 3f2a9c1e
c2 l --json
c2 m
```

For help, run `c2 --help`. `-h` is taken by `exec`.

Your command keeps its arguments, working directory and environment. The one
change is that it always sees `TERM=xterm-256color` and
`COLORTERM=truecolor`. You can run sessions inside sessions. Session data
lives in `~/.climon2` by default. Set `CLIMON2_HOME` to use another folder;
this folder is the **climon2 home**.

## Sessions

`exec` starts your command and attaches you to it.

**Headless sessions.** `c2 exec --headless <command> [args...]` (or
`c2 x -h`) starts the command in a [headless session](../../../GLOSSARY.md#sessions):
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
- A [monitor](monitor.md) watching a session keeps it alive by taking over.
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
[what happens when clients leave](#sessions) above). From then on it counts as an
attached client.

`c2 list --json` prints an array of objects, one per session. Each has a
`state` key holding `{"lifecycle": …, "attachments": …, "controlled": …}`, or
`null` when the session does not report a valid state.

Each session is attached and killed through the client of its own climon2
version, so that version must still be installed. Older sessions that do not
support an action fail with 125 and are left alone. `list` needs no client.

## One climon2 home, one computer

A climon2 home belongs to one computer. The first `c2` command that uses a
home records its owner in the home's `owner` file: the computer's machine ID
and, on Linux, its PID namespace (its set of process IDs). On Linux, macOS and
Windows the machine ID alone decides who the owner is. The PID namespace
decides only on a Linux computer or container whose `/etc/machine-id` is
missing, or new at every boot (as on live systems). A container usually has
its own PID namespace, unless it is started with `--pid=host`. Containers
like that, without a machine ID of their own, can look like one computer to
climon2.

Sharing a home with another machine, for example through a network home
directory, or with a container that has another machine ID, is refused.
`exec`, `attach`, `list`, `kill` and `monitor` then exit with 125 before they
do anything else:

```text
climon2: error: this climon2 home is in use by another machine or container (machine 8f2c4d1e9a7b4c3d8e6f5a4b3c2d1e0f, PID namespace 4026531836). climon2 does not work across machines yet (`c2 relay` is planned). To move this home here, run `c2 home release`, or `c2 home release --force` if the host that owns it is gone. To use it from here anyway, unclaimed, give the command `--force`.
```

To use such a home anyway, give the command `--force` before anything else,
for example `c2 list --force` or `c2 exec --force --headless make`. `exec`,
`attach`, `kill`, `list` and `monitor` take it. The command then runs
without claiming the home and prints a warning naming its owner. Sessions the
owner runs may then be judged ended here, so use it with care.

climon2 does not work across machines yet; `c2 relay` is planned. Until then,
give each machine and container a climon2 home of its own, with
`CLIMON2_HOME` if their home directories are shared.

`c2 home` shows the home, its owner and whether this computer is the owner. It
works whoever owns the home, and changes nothing:

```text
climon2 home: /home/me/.climon2
owner: machine 8f2c4d1e9a7b4c3d8e6f5a4b3c2d1e0f, PID namespace 4026531836
this host: machine 0b7e5a1c2d3f4e5a6b7c8d9e0f1a2b3c, PID namespace 4026531836 (not the owner)
```

`c2 home release` removes the `owner` file, so the next command claims the
home for the computer it runs on. It also clears an `owner` file that cannot
be read, which commands refuse until it is released or deleted. An empty
folder named `owner` is removed too; any other folder you must delete
yourself.

- On the computer that owns the home, it refuses, with 125, while any of that
  computer's sessions in the home may still be running. `c2 home release
  --force` releases it anyway and warns how many may be running.
- On any other computer, it refuses, with 125, while the home holds any
  sessions at all, because the owning computer may still be running them. If
  that computer is gone, run `c2 home release --force`.

After a forced release, the new owner never deletes the sessions the old
owner started: `list` does not show them, and they are kept until you delete
their files from the `sessions` folder, or the old owner takes the home back
with `c2 home release --force` and ends them.
## On Windows

On Windows, `c2` runs the same commands as on macOS and Linux, with the same
shortcuts, escape prefix, leave policy and monitor:

```powershell
c2 exec pwsh
$id = c2 exec --headless pwsh -NoProfile -File .\long-build.ps1
c2 attach $id
c2 list
c2 kill $id
c2 monitor
c2 home
c2 home release
c2 home release --force
```

The terminals and shells climon2 is tested with on Windows are listed in
[Terminals](terminals.md#windows).

**How a session runs.** A session's daemon runs the command in a
pseudoconsole, the Windows counterpart of a pseudo-terminal. climon2 ships its
own copy of ConPTY, the Windows component that provides one, as `conpty.dll`
and `OpenConsole.exe` in each version folder; if either is missing, `exec`
exits 125. The session's processes share a job object, a Windows group of
processes that can be ended together. A hang-up or `c2 kill` closes the
pseudoconsole, which tells every program on it to close, waits the full five
seconds, then ends whatever is left in the job. A `c2 exec` inside a session
starts its own session outside that job.

**Finding the command.** climon2 looks the command up itself: in each `PATH`
directory in order, trying each `PATHEXT` extension in order, so `c2 exec npm`
runs `npm.cmd`. The current directory is not searched. A command containing
`\`, `/` or `:` is found from the working directory instead. Batch files run
through the system's `cmd.exe`, and an argument a batch file cannot receive
unchanged (one holding a carriage return, line feed or NUL) is a usage error
(2).

**Exit statuses** are Windows' full 32-bit statuses, passed through `c2`
unchanged, and none is derived from a signal. A command ended by `c2 kill` or a
hang-up normally reports `0xC000013A`, which PowerShell's `$LASTEXITCODE` shows
as `-1073741510`; a program that handles the close itself reports its own
status. Closing the terminal while a client is attached still ends the client
with 129, and climon2's own 2, 125, 126 and 127 keep their meanings. Ctrl+C
reaches the command as on Unix.

**Jobs around your shell.** The session daemon must leave any job object your
shell runs in, or it would end with your terminal. `c2 exec` exits 125 when it
cannot: inside a job that refuses this, and under PowerShell's
`Start-Process -Wait`. Windows Terminal, VS Code terminals and VS Code tasks
put no job around their shells.
