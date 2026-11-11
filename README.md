# climon2

Run any terminal command in a managed session, detach from it and reattach
later, end it deliberately, or list live sessions:

```sh
climon2 exec bash --noprofile --norc
climon2 exec vim
climon2 attach 3f2a9c1e
climon2 kill 3f2a9c1e
climon2 list
climon2 list --json
```

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
required real-monitor, packaged-library and measured evidence, not tests
already delivered.

## Sessions

`exec` starts the command and attaches to it. There is no start-detached option.
Future `climon2 exec --headless <command> [args...]` is agreed but **not
implemented**. It will return success only after daemon-confirmed creation,
without an interactive client. A headless session will survive zero clients
throughout its lifetime, including after later attachment loss, until command
completion or explicit Kill. Ordinary leave policy below remains unchanged.
The [lifecycle specification](docs/monitor-lifecycle.md#future-headless-creation-and-existing-cli-kill)
requires preserving existing `climon2 kill <session>` and automated headless
termination tests, not adding another kill command.

**Detach** with the escape prefix, Ctrl+], then `d`. The session keeps running
with no idle timeout, and the client exits 0 after printing
`climon2: detached from session <id>; reattach with: climon2 attach <id>` to
stderr. Ctrl+] Ctrl+] sends one Ctrl+] to the command; Ctrl+] followed by any
other key sends both. A lone Ctrl+] waits for the next key, without a timeout.
Every other key, including Ctrl+C and Ctrl+D, goes to the command. No binding
is interpreted inside a bracketed paste.

**Attach** with `climon2 attach <session>`. It needs a terminal on stdin and
stdout, and takes control immediately. `<session>` is a full session ID or a
unique prefix of at least 8 hex digits, case-insensitive, with or without
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
  connection lost), the session ends at once: its terminal is hung up and
  remaining jobs are killed after five seconds. There is no reconnection grace.
  A client also counts as lost if its terminal accepts no output for five
  seconds, falls more than 16 MiB behind, or does not finish restoring within
  30 seconds plus one second per 4 MiB restored.
- A session that no client attaches to within five seconds of starting ends.
- When the command exits, every attached client exits with its status.

**Kill** one session with `climon2 kill <session>` (same selector rules), whether
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

Exit statuses: 0 for a confirmed detach or kill; the command's own status when
it finishes while the client is attached; 2 for usage errors; 125 for climon2
failures; 128 + signal when the client is terminated by a signal; 129 when the
client's terminal goes away.

## Configuration

climon2 reads an optional `<climon2 home>/config.jsonc`. These are the defaults:

```jsonc
{
  "client": {
    // Ctrl+[ is Escape: delays bare Escape; planned monitor Alt+s/Alt+x/Alt+j may trigger s/x/j.
    "escape_prefix": "Ctrl+]",     // null disables escape interception in ordinary attach
    "take_control_key": "Ctrl+T"   // null disables keyboard takeover
  },
  "scrollback": {
    "max_rows": 10000
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

`exec` validates the whole file before starting the session, `attach` validates
only `client`, and `list` and `kill` do not read it. Any error, including
malformed JSONC, duplicate keys, wrong types, invalid keys, conflicting bindings
and out-of-range values, exits 125 before anything starts. The file is read once
per command; there is no environment override, auto-creation or live reload.

### Planned monitor settings (not implemented)

The [agreed terminal integration](docs/protocol/terminal-integration.md) provides
interactive terminal input/rendering, internal normal-screen history, one top
status row, and explicit control/restoration gates. There is no monitor command
or automated monitor implementation evidence yet.
The future monitor reads one startup snapshot, reusing `client` bindings; it
rejects a null escape prefix before changing terminal state. Null takeover is
allowed because selector confirmation can acquire control. Lowercase prefix+s
opens the selector, prefix+x exits, and **prefix+j** jumps to the most recent
currently connected session needing attention, not necessarily a toast's session.
Prefix+? opens the [key introduction/help dialog](docs/monitor-selector.md#key-introduction-and-help).
Uppercase J is unknown literal input; bracketed pasted shortcuts remain data.

Full future startup example (the current parser does **not** implement the
`monitor` field):

```jsonc
{
  "client": {
    // Ctrl+[ is Escape: delays bare Escape; Alt+s/Alt+x/Alt+j/Alt+? may trigger s/x/j/?.
    "escape_prefix": "Ctrl+]",
    "take_control_key": "Ctrl+T"
  },
  "scrollback": {
    "max_rows": 10000
  },
  "monitor": {
    "show_key_hints": true, // planned: implicit monitor hints, including the toast hint row
    "bell": true           // planned: one host bell when each toast starts drawing
  },
}
```

Planned `monitor.show_key_hints` is boolean, default true when missing. It hides
implicit monitor hints, never bindings or application text. Needs-attention toast
hints use the actual configured prefix plus lowercase j and describe a dynamic
most-recent jump. Wrong types and duplicate known fields are explicit startup
errors before terminal changes; unknown/unrelated settings remain ignored.
All monitor settings above are future behavior, with no live reload.

The planned modal introduction opens automatically on startup when key hints
are enabled and **Don't show again** has not been saved. It initially introduces
the configured prefix followed by x (exit), s (session list), and ? (help).
The checkbox stores a reversible preference separately from configuration,
per climon2 home across restarts/releases; save failures are explicit.
Prefix+? always opens the dialog, even with hints disabled or startup display
suppressed. Enter closes it; Escape follows configured-prefix handling.

Planned `monitor.bell` is boolean, default true, with the same pre-terminal
startup validation and no live reload. False suppresses the explicit host bell,
not toasts or OSC 7501 reports; host settings govern actual audibility.
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
[operational limits](docs/monitor-operational-limits.md) define planned
`monitor.limits` configuration, numerical budgets, deadlines and evidence.
Under pressure, local history shrinks to its newest captured page, never
disappears entirely; resource-blocked sessions remain visible. Complete native
declarations (#71) and final automated acceptance (#62) remain open. All of this
is unimplemented; no general session-survival or native-crash isolation is added.
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

`climon2 list` shows live sessions, newest first, with the columns SESSION,
CREATED, VERSION, STATE, CLIENTS, COMMAND and ARGS. STATE is `starting`,
`attached`, `detached`, `terminating`, or `unknown` for a session that does not
report it. CLIENTS counts attached clients, or shows `-` when unknown.

`climon2 list --json` prints an array of session metadata objects. Each has a
`state` key holding `{"lifecycle": …, "attachments": …, "controlled": …}`, or
`null` when the session does not report a valid state.

Each session is attached and killed through the client of its own climon2
version, which must still be installed. Older sessions that do not support an
operation fail with 125 and are left undisturbed. `list` needs no client.

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
for the current runtime, test tooling, and planned monitor/client-library design.
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

CI runs full runtime, real-terminal and packaged tests on Linux and both macOS
architectures. Windows runs neutral contracts only. Shared and Linux-specific
CRAP/mutation run on the dedicated Linux development host, with 16 mutation
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
The unchanged workspace must pass in a real mutation overlay before mutation;
worker infrastructure failures cannot count as killed mutants.
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
`target/dist/`. Set `CLIMON2_E2E_ARCHIVE` to that archive to test it unchanged.

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
