# climon2

Run any terminal command in a managed session, detach from it and reattach
later, end it deliberately, or list live sessions:

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

`c2` is the short command for climon2. The installer creates `c2 -> climon2`;
the original `climon2` command remains supported without warnings. Both names
run the same current version. Product names, version output, diagnostic prefixes,
configuration/data paths and environment variables remain unchanged.

The command keeps its arguments, working directory and environment, except that
it always sees `TERM=xterm-256color` and `COLORTERM=truecolor`. Its exit status
becomes the exit status of every client attached when it finishes. Nested
sessions are supported. Session data defaults to `~/.climon2`; set
`CLIMON2_HOME` to override it.

The [protocol reference](docs/protocol/README.md) explains the current daemon
handshake and the planned in-process monitor-client interface for future plugin authors.
It distinguishes implemented behavior from agreed plans and unresolved details.
The planned [session status, attention, and activity mechanics](docs/session-status.md)
include both daemon state machines and the monitor's ordering/recovery behavior.
The agreed [monitor acceptance criteria](docs/monitor-acceptance.md) define
the required real-monitor, packaged-library and measured evidence; only the
monitor's entry, startup refusals and exits, and headless sessions (HDL-01),
have tests so far.

## Sessions

`exec` starts the command and attaches to it.

**Headless sessions.** `c2 exec --headless <command> [args...]` starts the
command in a [headless session](GLOSSARY.md#sessions) without attaching to it.
It needs no terminal, so scripts and CI can start background work and target it
later:

```sh
id=$(c2 exec --headless ./long-build.sh)
c2 attach "$id"
```

Like `exec`, it validates the whole configuration first. It returns only once
the session is running: it then prints the full lowercase session ID and a
newline on stdout and exits 0. Creation failures keep `exec`'s statuses and
diagnostics: 127 when the command is not found, 126 when it cannot be executed,
and 125 for climon2 failures such as invalid configuration. If the ID cannot be
written to stdout, the session keeps running and the status is still 0;
`c2 list` shows it. The command's later exit status belongs to the session, not
to this CLI. The session starts at the size of the terminal on stdin, or 80x24
without one; after that, the controlling client sets its size as for any
session. `--` still separates climon2's options from the command:
`c2 exec -- --headless` runs a command named `--headless`.

A headless session survives with zero clients: from the start, after a
deliberate detach and after a client is lost unexpectedly. It ends only when its
command exits or it is killed with `c2 kill`. `c2 list` shows it as
`detached` whenever no client is attached.

**Detach** with the escape prefix, Ctrl+], then `d`. The session keeps running
with no idle timeout, and the client exits 0 after printing
`climon2: detached from session <id>; reattach with: climon2 attach <id>` to
stderr. Ctrl+] Ctrl+] sends one Ctrl+] to the command; Ctrl+] followed by any
other key sends both. A lone Ctrl+] waits for the next key, without a timeout.
Every other key, including Ctrl+C and Ctrl+D, goes to the command. No binding
is interpreted inside a bracketed paste.

**Attach** with `c2 attach <session>`. It needs a terminal on stdin and
stdout, and takes control immediately. `<session>` is a full session ID or a
unique prefix of at least 1 hex digit, case-insensitive, with or without
hyphens, matching a live session. A malformed `<session>` is a usage error (2).
No match, or more than one, fails with 125; an ambiguous prefix lists every
matching ID. Attaching to the session you are running inside is refused.

On every attach or takeover, climon2 first scrolls the visible terminal into its
native scrollback. It then restores the session's retained history, screen,
cursor, modes and title, including the normal screen behind a full-screen
application. A `[climon2: older session scrollback was discarded]` line marks
history that was dropped to stay within `scrollback.max_rows`.

**Several clients** can attach to one session. One of them, the controller,
drives the session's input and size. The others show a placeholder and receive
no session output:

```text
This session is being viewed elsewhere.
Press Ctrl+T to take control
```

Press the take-control key (Ctrl+T) in a placeholder to take control; with
`take_control_key` set to `null` the placeholder shows only its first line. Other
keys typed in a placeholder are discarded, and the escape prefix still detaches.
Ctrl+T reaches the command normally while you are in control. The most recent
attach or takeover always wins, without a prompt. When the controller leaves,
control returns to the most recently controlling client that is still attached.

**Leave policy**:

- While another client remains attached, the session continues.
- When the last client detaches, the session runs until its command exits or it
  is killed.
- When the last client leaves any other way (terminal closed, client crashed,
  connection lost), an ordinary session ends at once: its terminal is hung up
  and remaining jobs are killed after five seconds. There is no reconnection
  grace. A headless session keeps running instead. A client also counts as lost
  if its terminal accepts no output for five seconds, falls more than 16 MiB
  behind, or does not finish restoring within 30 seconds plus one second per
  4 MiB restored.
- An ordinary session that no client attaches to within five seconds of starting
  ends. If its command exits sooner, the exit is kept for that first client
  until then. A headless session has no such window.
- When the command exits, every attached client exits with its status.

**Kill** one session with `c2 kill <session>` (same selector rules), whether
or not clients are attached. It needs no terminal and prints nothing. The
session hangs up its terminal and always waits the full five-second grace
before killing any remaining jobs, so a kill normally takes about five seconds.
It exits 0 once the command has ended and the session is cleaned up, or 125 if
that is not confirmed within 10 seconds. Attached clients exit with the
command's status, normally signal-derived. On Linux, as when a terminal window
closes, a command reading the terminal can see it close before the hang-up
signal arrives and end with its own status instead.

When a client's view of a session ends (detach, exit, kill or takeover by
another client), climon2 returns its terminal to a normal shell state: it ends
any synchronized update, leaves the alternate screen only if it entered it,
resets margins and the origin, insert and new-line modes, turns autowrap on,
turns off mouse, focus, bracketed-paste, application cursor-key and keypad
modes, shows the cursor in its default style, and resets the rendition. Terminal
settings (termios) are restored exactly.

Exit statuses: 0 for a confirmed detach or kill, or a created headless session;
the command's own status when it finishes while the client is attached; 2 for
usage errors; 125 for climon2 failures; 126 and 127 when `exec` cannot start the
command; 128 + signal when the client is terminated by a signal; 129 when the
client's terminal goes away.

## Configuration

climon2 reads an optional `<climon2 home>/config.jsonc`. These are the defaults:

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
    "bell": true,                  // false: no host bell as a toast first draws
    "limits": {
      "memory": {
        "total_bytes": 536870912,
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
        "toast_records": 128,
        "toast_bytes": 524288,
        "search_bytes": 4096
      }
    }
  },
}
```

- The format is JSON with `//` and `/* */` comments and trailing commas.
  Comments act as whitespace: they separate tokens but never join them, so
  `1/* c */0000` is malformed, not `10000`.
- A missing file, object or field means the default. Unknown fields are ignored.
- Keys are `Ctrl+A` to `Ctrl+Z`, `Ctrl+[`, `Ctrl+\`, `Ctrl+]`, `Ctrl+^`,
  `Ctrl+_` and `Ctrl+Space`; `null` disables a binding. `take_control_key`
  cannot be `Ctrl+[` (Escape), and the two bindings cannot be the same key.
- With `escape_prefix` set to `null` there is no detach key; closing the last
  client then ends the session.
- `scrollback.max_rows` (0 to 100000, default 10000) is the number of
  normal-screen history rows a session retains for restoration. It is fixed
  when the session starts; attaching never changes it. 0 disables retained
  history, but the screen is still restored.
- `monitor.show_key_hints` (default true) hides the [monitor](#monitor)'s
  implicit key hints when false, never a binding or application text. Today
  the empty state's hint is the only one; the planned hints below follow it.
  False also keeps the key introduction from opening at startup; the escape
  prefix followed by `?` still opens it, and your **Don't show again** choice
  is kept apart and never changes this setting.
- `monitor.bell` (default true) is validated; the toasts whose bell it
  silences are planned.
- `monitor.limits` holds the [operational limits](docs/monitor-operational-limits.md),
  in bytes, milliseconds (`_ms`) or counts. The monitor validates and reports
  them; nothing enforces them yet. Its `history.max_rows` is the monitor's own
  local history and never changes `scrollback.max_rows`. Each is an integer:
  - 1 to 3600000 for durations, but 1 to 50 for `ui.client_quantum_ms` and
    `ui.background_quantum_ms`;
  - 200 to 100000 for `history.max_rows`, 1 to 120 for `ui.fps`, 0 to 50 for
    `retry.jitter_percent`, and 0 or more for `resources.max_workers_per_client`;
  - at least 1 for every other budget, ceiling or count, up to 4294967295 for
    counts and for `transfer.chunk_bytes` and the `payload` `_bytes` ceilings
    other than `assembly_bytes`, or 18446744073709551615 for the other byte
    budgets.
- The limits must also agree with each other:
  - `memory.client_bytes`, `history_bytes_total` and `queue_bytes_total`, the
    `resources.stack_bytes` and `release_bytes` reservations, and each monitor
    byte budget that no smaller total contains (`transfer.drain_payload_bytes`,
    `discovery.batch_bytes`, `discovery.metadata_file_bytes` and
    `ui.search_bytes`) fit `memory.total_bytes`; `history_bytes_per_session`
    fits both `history_bytes_total` and `client_bytes`;
  - `queues.request_bytes` and `event_bytes` each fit both
    `memory.queue_bytes_total` and `memory.client_bytes`, and
    `ui.toast_bytes` fits `memory.queue_bytes_total`, which includes the UI
    queues;
  - `payload.assembly_bytes` and `assembly_records` limit the daemon's
    notification assembly, not the monitor's memory, so no monitor total
    contains them;
  - `queues.progress_bytes` is less than `event_bytes`, and `progress_records`
    less than `event_records`, leaving room for ordinary work;
  - `transfer.chunk_bytes` fits `queues.request_bytes`, the ordinary event
    capacity (`event_bytes` minus `progress_bytes`) and
    `transfer.drain_payload_bytes`; `payload.metadata_record_bytes` fits
    `progress_bytes`, that ordinary capacity and `drain_payload_bytes`;
  - `ui.client_quantum_ms` fits `ui.background_quantum_ms`, and
    `retry.initial_ms` fits `retry.cap_ms`.

`exec` validates the whole file before starting the session. `attach`
validates only `client`; the monitor validates `client` and `monitor`, and
refuses a null `escape_prefix`. `list` and `kill` do not read the file. Any
error, including malformed JSONC, duplicate keys, wrong types, `null` for a
setting that cannot be disabled, invalid keys, conflicting bindings,
out-of-range values and limits that disagree, exits 125 before anything starts.
The file is read once per command; there is no environment override,
auto-creation or live reload.

### Planned monitor behavior (not implemented)

The [agreed terminal integration](docs/protocol/terminal-integration.md) provides
interactive terminal input/rendering, internal normal-screen history, one top
status row, and explicit control/restoration gates. Of these, the
[monitor](#monitor) implements only its top row, empty state, exits and key
introduction so far, and already treats pasted shortcuts as data. It reads the
configuration above once, before changing terminal state. Null takeover is
allowed because selector confirmation can acquire control. Lowercase prefix+s
opens the selector, prefix+x exits, and **prefix+j** jumps to the most recent
currently connected session needing attention, not necessarily a toast's session.
Prefix+? opens the [key introduction/help dialog](docs/monitor-selector.md#key-introduction-and-help).
Uppercase J is unknown literal input; bracketed pasted shortcuts remain data.

Needs-attention toasts will hint the actual configured prefix plus lowercase j
and describe a dynamic most-recent jump, unless `monitor.show_key_hints` is false.

Help opened over the selector will keep the selector's search and highlight
and return to them when it closes, without acquiring or acknowledging
anything.

With toasts, `monitor.bell` false will suppress the explicit host bell, not
toasts or OSC 7501 reports; host settings govern actual audibility.
The [selector specification](docs/monitor-selector.md) records the accepted
layout, read-only disconnected/final history selection and animated toasts.
The later [lifecycle specification](docs/monitor-lifecycle.md) adds planned
default-Cancel Delete confirmation pinned to immutable session identity.
Removal waits for definitive termination and local instance cleanup/join,
dismisses only for this run, and never acquires a fallback session or re-adds
the dismissed GUID in the same run. No connected revoked-cache inspection route
or prefix+h/prefix+r binding is added. Disconnected read-only cache overlays
exactly `Reconnecting...` during scheduled/active automatic passive recovery.
Recoverable transport/startup failures (including missing libraries) and safe
worker panics use exponential backoff; permanent rejection/invalid configuration
stop it. Replacement instances wait for old cleanup/join; no indeterminate
input/request/Kill is replayed. The monitor remains open through empty/all-final
states and preserves selection identity without automatic replacement takeover.

Planned prefix+x and catchable signals coordinate concurrent shutdown of every
owned instance. Accepted Leave, connection closure, final outcome, cleanup and
join are separate evidence. Restore host state and clear root attention before
diagnostics; stalled cleanup uses bounded whole-process failure, never a false
detach/join claim or thread kill. The agreed
[operational limits](docs/monitor-operational-limits.md) define the
`monitor.limits` budgets, deadlines and evidence; only their configuration is
validated so far. Under pressure, local history shrinks to its newest captured
page, never disappears entirely; resource-blocked sessions remain visible. All
of this is unimplemented; no general session-survival or native-crash isolation
is added.
Planned explicit exit succeeds only after required Leave outcomes, local cleanup/
joins and host restoration succeed, never with the selected command's status.
Fatal monitor errors, required Leave failure/uncertainty, incomplete cleanup or
restoration failure use 125; a catchable signal uses signal-style status only
after successful cleanup. Session-local command/client failures alone do not
poison a later clean monitor exit.
The monitor also generates its own
[OSC 7501 aggregate attention reports](docs/protocol/README.md#monitor-to-host-status-and-bell),
never raw session-sequence passthrough.

## Listing

`c2 list` shows live sessions, newest first, with the columns SESSION,
CREATED, VERSION, STATE, CLIENTS, COMMAND and ARGS. STATE is `starting`,
`attached`, `detached`, `terminating`, or `unknown` for a session that does not
report it. A headless session is never `starting`: it is `detached` whenever no
client is attached. CLIENTS counts attached clients, or shows `-` when unknown. A
monitor watching a session without attaching to it (a passive monitor
connection) is not an attachment: it never counts in CLIENTS or changes STATE.

`c2 list --json` prints an array of session metadata objects. Each has a
`state` key holding `{"lifecycle": …, "attachments": …, "controlled": …}`, or
`null` when the session does not report a valid state.

Each session is attached and killed through the client of its own climon2
version, which must still be installed. Older sessions that do not support an
operation fail with 125 and are left undisturbed. `list` needs no client.

## Monitor

`c2 monitor`, or `c2 mon`, opens the monitor: one terminal that will track
every live session. It takes no arguments (any is a usage error, 2) and needs a
terminal on stdin and stdout. The launcher always runs the current version's
monitor, on macOS and x86-64 GNU/Linux alike. Before your terminal changes, it
reads `config.jsonc` once: your `client.escape_prefix` and
`client.take_control_key` settings and their defaults, and the `monitor`
section (see [Configuration](#configuration)). It refuses a `null` escape
prefix, the only way to reach its actions. Startup errors, any invalid setting
included, are reported, with status 125, before your terminal changes. Editing
the file while the monitor runs changes nothing.

The monitor does not discover sessions yet. On its own alternate screen it shows
one permanent top row and an empty state whose key hint names your escape
prefix; with `monitor.show_key_hints` false only `No sessions to show.` remains:

```text
No session selected
                         No sessions to show.
                     Press Ctrl+] then x to exit.
```

The escape prefix followed by `x` exits with status 0; any other key is
discarded, and Ctrl+C is a key, not an exit. Pasted text is never a key: a
paste holding the prefix and `x` does not exit, and a paste cancels a pending
prefix. SIGINT, SIGTERM and SIGHUP leave the same way with status 128 + signal.
Resizing repaints the screen at the new size without blanking it. If your
terminal hangs up or can no longer be read, the monitor leaves with status 125.
On every exit the monitor returns your terminal to the normal shell state
described for clients above, restores its settings (termios) exactly and never
changes its title; only then does it print any error, with status 125. It logs
to the general log in the climon2 home, including its effective
configuration as JSON, never your input.

Until you turn it off, the monitor starts with its key introduction over the
empty state, listing its keys with your escape prefix:

```text
┌ Keys ────────────────────────────────┐
│ Ctrl+] then x  Exit the monitor      │
│ Ctrl+] then s  Open the session list │
│ Ctrl+] then ?  Show this dialog      │
│                                      │
│ [ ] Don't show again (Space)         │
│                                      │
│ Continue (Enter)                     │
└──────────────────────────────────────┘
```

Enter continues to the empty state. Space checks **Don't show again**, or
clears it again, and saves your choice at once: while it is checked, later
starts with the same climon2 home open on the empty state. The introduction
also stays closed while `monitor.show_key_hints` is false. Whatever either
says, the escape prefix followed by `?` opens the same dialog as help,
showing your saved choice; doing so while it is open changes nothing. The
dialog takes every other key and any paste, and none of them does anything
else; the escape prefix followed by `x` still exits. The escape prefix pressed
twice reaches the dialog as that key, so with `Ctrl+M` (Enter) as the prefix,
Enter Enter continues. There is no session list for `s` to open yet, and
Escape does not close the dialog: with `Ctrl+[` as the prefix it never can,
because Escape is the prefix.

The choice lives in `<climon2 home>/monitor-preferences.json`, apart from
`config.jsonc`: `{"version":1,"suppress_key_introduction":true}`. Every
release reads it, whatever its version, ignoring fields it does not know. The
monitor writes it only when you change the box, as a new file readable only by
you (`monitor-preferences.json.<pid>.tmp`) renamed over the old one; before
that, it removes any such file that a monitor no longer running left behind,
interrupted before its rename. If the file cannot be read or is malformed, the
introduction opens with the reason under its unchecked box (with key hints
off, the empty state shows the reason instead), and the file is left as it is
until you change the box. If a save fails, the dialog says why under the box,
which keeps showing the saved choice; with key hints off, the empty state says
so too. The general log records these problems and each saved choice.

## Terminal compatibility

The session daemon keeps its own virtual terminal for each session, built on
Alacritty's `alacritty_terminal` 0.26.0. It answers the command's terminal
queries itself and never asks your terminal anything. `TERM=xterm-256color` and
`COLORTERM=truecolor` describe the subset of xterm that climon2 qualifies, not
complete xterm compatibility; they replace any inherited values.

Interactive shells are qualified on GNU bash 5.x only; macOS `/bin/bash` 3.2 is
not supported for interactive sessions. `install.sh` still runs under
`/bin/bash`.

## Limitations

- **Keyboard input is legacy only.** Kitty keyboard and modifyOtherKeys
  requests are stripped, and application-keypad fidelity is not guaranteed.
- **Unmarked pastes look like typing.** Only bracketed pastes are exempt from
  bindings, so an unbracketed paste containing Ctrl+] or Ctrl+T can trigger them.
- **Bindings hide keys from applications.** The escape prefix is always
  intercepted (press it twice to send it), and the take-control key is
  intercepted in the placeholder. Rebind or disable them in the configuration.
- **Duplicates on reattach.** Every attach resends all retained history, so
  native scrollback can hold copies of it.
- **Not restored on attach:** scroll region, tab stops, character sets, palette
  changes, keyboard-protocol modes and hyperlinks. Your terminal's own title is
  not restored on exit; it keeps the session's title (default
  `climon2 exec <command> <args...>`).
- **Saved cursor on the alternate screen.** A restoration taken while an
  application is on the alternate screen sets the normal screen's saved cursor
  to its current cursor. They usually match, but can differ after a resize while
  hidden.
- **Live output is filtered.** Terminal queries are answered by climon2 or
  stripped. OSC 52 clipboard access, palette changes, every other OSC sequence
  (such as OSC 7 and shell-integration marks) and DCS, APC, PM and SOS strings
  are stripped. Hyperlinks (OSC 8) appear as plain text. Titles have control
  characters percent-encoded. Control strings and CSI sequences over 4096 bytes
  are dropped.
- **Session size is 2 to 500 columns by 1 to 200 rows.** On a larger terminal
  the session occupies the top-left corner, drawn from the daemon's screen
  rather than forwarded unchanged; the scroll region is not carried across a
  switch between the two. Terminals narrower than 2 columns are unsupported.
- **Only direct self-attachment is detected.** Other cycles, such as attaching
  each of two sessions inside the other, are your responsibility.
- **Unexpected loss restores little.** If the session daemon disappears, the
  client can only reset rendition, cursor visibility, mouse, focus,
  bracketed-paste, cursor-key and keypad modes and synchronized output before
  restoring termios. It cannot leave the alternate screen or restore other
  terminal state.
- **Combining marks are unbounded.** Alacritty 0.26.0 stores a cell's combining
  marks without a limit, so a stream of them grows daemon memory. Qualification
  evidence for other Alacritty revisions does not apply to 0.26.0.

## Install

Supports **macOS 15 or newer** on Apple Silicon and Intel, and **x86_64
GNU/Linux** with glibc 2.39 or newer, qualified on Ubuntu 24.04. macOS binaries
are universal and ad-hoc signed, without Developer ID signing or notarisation.
Arch Linux is supported for native development; musl and Linux arm64 are not
qualified. See [distribution](docs/distribution.md) for platform archives.

Latest release:

```sh
curl -fsSL https://github.com/jackgeek/climon2/releases/latest/download/install.sh | /bin/bash
```

Pinned release (replace `0.1.0` with the published version):

```sh
curl -fsSL https://github.com/jackgeek/climon2/releases/download/v0.1.0/install.sh | /bin/bash
```

Each published script downloads only its stamped version and checks its SHA-256
checksum before running the installer. To choose another executable home:

```sh
curl -fsSL https://github.com/jackgeek/climon2/releases/latest/download/install.sh |
  /bin/bash -s -- --dir "$HOME/tools/climon2"
```

The default executable home is `~/.local/share/climon2`. Add it to PATH:

```sh
export PATH="$HOME/.local/share/climon2:$PATH"
```

The installer prints a matching hint for custom destinations and never modifies
shell startup files. Upgrades preserve existing version folders and live sessions.
Fresh installs and upgrades add `c2` in the same executable home, with no PATH
change needed for existing users. An unrelated file or symlink named `c2` there
blocks installation before publication; resolve the reported conflict and retry.
An existing correct relative link is accepted on repeated installs.
There is no `sudo` or Homebrew installation step.

For macOS browser downloads, verify `SHA256SUMS`, extract the archive, then remove
quarantine from the extracted directory before running its installer:

```sh
shasum -a 256 -c SHA256SUMS
tar -xzf climon2-0.1.0-macos-universal.tar.gz
xattr -dr com.apple.quarantine climon2-0.1.0-macos-universal
./climon2-0.1.0-macos-universal/install
```

For Linux manual installation, use the `linux-x86_64-gnu` archive and
`sha256sum`; there is no quarantine-removal step. Linux requires procfs and a
filesystem supporting `renameat2` directory exchange at the installation path.
Processes sharing a climon2 home must share the same PID namespace; sharing
session metadata across containers or PID namespaces is not supported.

## Diagnostics

Operational failures use status 125 and identify a log file. Logs contain climon2 diagnostics,
not terminal content, command arguments or environment values. Release logging
defaults to INFO; `CLIMON2_LOG` accepts a tracing filter. Session logs are capped
at 8 MiB each; every client invocation writes its own session log; inactive session
logs older than seven days are pruned.

## Development and releases

See the [executable and dynamic-library relationships](https://github.com/jackgeek/climon2/blob/main/docs/runtime-artifacts.md)
for the current runtime and client library, test tooling, and the planned
monitor loading design.
See the [workspace crate diagram](https://github.com/jackgeek/climon2/blob/main/docs/crate-relationships.md)
for consumption relationships between climon2 crates, including test tooling.

```sh
cargo test --workspace --locked
cargo xtask setup-quality
cargo xtask check
cargo xtask check --mutate-all
cargo xtask dist
cargo xtask install
cargo xtask install --dir "$HOME/tools/climon2"
```

Development needs the pinned Rust toolchain, a C compiler and libclang,
ShellCheck, Python 3.11 or newer, GNU bash 5.x, vim and less. Interactive tests
use `CLIMON2_TEST_BASH`, then platform defaults: Homebrew bash on macOS, or
`/bin/bash` and `/usr/bin/bash` on Linux. macOS `/bin/bash` 3.2 is not used for
interactive qualification. A missing bash 5 fails in CI and is a reported local
skip; use `CLIMON2_TEST_BASH=/usr/bin/bash` on Arch Linux. `install.sh` remains
compatible with macOS `/bin/bash` 3.2.

On arm64 macOS, install both Rust targets before running workspace tests,
`dist` or `install`: `rustup target add aarch64-apple-darwin x86_64-apple-darwin`.

CI runs full runtime, real-terminal and packaged tests on Linux and both macOS
architectures. Windows runs neutral contracts only. Shared and Linux-specific
CRAP/mutation run on the dedicated Linux development host, with 8 mutation
workers. CI does not repeat those quality checks.

The macOS CI quality job uses `cargo xtask mutate --native-only`: CRAP scores
only registered macOS-exclusive source, and mutation selects changed files
within that set. Shared-only changes skip that quality run; release tags use
`--native-only --mutate-all` to qualify every macOS-exclusive file. Coverage
still executes the workspace to exercise real callers, and mutation still
requires the unchanged full-workspace worker baseline. Native CI parallelism
follows available CPUs, capped at 16.

`xtask check` runs formatting, strict clippy, unit/integration/installer-built
packaged tests, ShellCheck, append-only metadata fixtures, instrumentation and
worker-isolation proofs, coverage-backed CRAP <= 5 and zero-survivor mutation.
Its tests, like `cargo xtask test`, first run the monitor's build test alone,
proving its plain binary carries no test hooks, before the workspace tests
build it with hooks at the same path.
The unchanged workspace must pass in a real mutation overlay before mutation;
worker infrastructure failures cannot count as killed mutants.
`cargo xtask check` also measures mutation load before mutating, even when no
changed shipped source needs mutation; `cargo xtask mutate` does not. It warms
`target/mutation-tests` and times one unloaded full-workspace run there, then
runs five rounds of 8 full-workspace suites at once, each in a private copy of
the warm build (copy-on-write where supported). As in mutation, each suite has
its own session. A suite fails the measurement if any test fails or if it
outlives the mutator's timeout (10 times the unloaded run, at least 2 s),
because the mutator would count either outcome as a killed mutant and hide a
survivor. A surviving mutant's run also stages library and crate tests within
that timeout, so this catches stalled suites rather than every possible mutant
timeout. A failure under load is a test defect; nothing is retried. The
measurement stops after the failing round and names each failing test and test
target with the number of suites it failed in. It then lists their logs. Logs
from every run remain in a unique directory under `target/flake-logs/`. Worker
builds are removed afterward, and the next measurement removes any that a killed
one left.
Each mutant first runs workspace library tests, then its crate's integration
tests. A failing stage stops immediately; every mutant that passes both still
runs the full workspace before it can count as a survivor. The unchanged
baseline always runs the full workspace. Workers receive private copies of
its warm build artifacts (copy-on-write where supported), rather than
rebuilding dependencies from scratch for every changed file.
`CLIMON2_MUTATION_BASE` selects the changed-source comparison (default `HEAD`).
For committed work, set it to the target branch or merge base, for example
`CLIMON2_MUTATION_BASE=origin/main cargo xtask check`.
Native implementations must live in registered source files; shared files
must not hide macOS-only bodies that Linux cannot qualify.

`xtask dist` requires an **arm64 macOS host** with both Rust macOS targets, or
an **x86_64 GNU/Linux host**. macOS builds retain deployment target 15.0,
universal slices and signature checks. Linux builds verify native ELF
executables; published Linux artifacts are built on Ubuntu 24.04, not Arch,
to establish the glibc baseline. Both builds isolate unhooked packages and
write their native archive, stamped `install.sh` and `SHA256SUMS` to
`target/dist/`. Each version folder also holds the client library,
`libclimon2_client.dylib` on macOS or `libclimon2_client.so` on Linux, which
must export only its bootstrap and describe exactly the archive's version; the
Linux library must also have no soname, RPATH or RUNPATH and need only glibc's
libraries. `release-check` applies these library checks to the local
per-package release build too.
Set `CLIMON2_E2E_ARCHIVE` to that archive to test it unchanged.

`xtask install` builds and verifies the same production payload as `dist`, then
runs its installer without creating an archive. It has the same host and build
requirements as `dist`. The default executable home is `~/.local/share/climon2`;
`--dir <directory>` selects another destination. The installer preserves other
versions and prints a PATH hint when needed; shell startup files are unchanged.

A release requires a local `cargo xtask check --mutate-all` pass for the exact
release commit; this is the maintainer's responsibility, not a duplicated CI
job. A `v<workspace version>` tag runs full macOS-only mutation and the native
runtime/archive gates. The same final
macOS archive must pass both architectures, and the Linux archive must pass
Ubuntu 24.04, under bash 5 including vim and less. Both platforms must pass
before one GitHub Release and provenance attestation are published.
`-rc.N` tags are prereleases. Starting with the second published release, actual
previous-release compatibility is mandatory; the first release invents none.

**Intel runner availability is a release blocker.** If `macos-15-intel` retires,
a replacement native Intel macOS 15+ runner must pass the same artifact tests.
An unavailable runner or arm64-only success does not establish Intel support.

## License

climon2 is open source software licensed under the [MIT License](LICENSE).
