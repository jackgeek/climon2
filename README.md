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

## Sessions

`exec` starts the command and attaches to it. There is no start-detached option.

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
command's signal-derived status.

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
    "escape_prefix": "Ctrl+]",     // null disables escape interception
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

Requires **macOS 15 or newer**, Apple Silicon or Intel. Release binaries are
universal and ad-hoc signed, without Developer ID signing or notarisation.

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

For browser downloads, verify `SHA256SUMS`, extract the archive, then remove
quarantine from the extracted directory before running its installer:

```sh
shasum -a 256 -c SHA256SUMS
tar -xzf climon2-0.1.0-macos-universal.tar.gz
xattr -dr com.apple.quarantine climon2-0.1.0-macos-universal
./climon2-0.1.0-macos-universal/install
```

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
```

Install ShellCheck before `xtask check`. Full tests need GNU bash 5.x, vim and
less. Interactive shells are qualified on bash 5.x only: tests use
`CLIMON2_TEST_BASH`, else `/opt/homebrew/bin/bash`, else `/usr/local/bin/bash`,
never macOS `/bin/bash` (3.2). A missing bash 5 fails in CI and is a reported
local skip. `install.sh` stays compatible with, and is tested under, `/bin/bash`
3.2. Linux and Windows run only neutral contracts (Linux also tests the Unix
launcher), not the macOS product.

`xtask check` runs formatting, strict clippy, unit/integration/installer-built
packaged tests, ShellCheck, append-only metadata fixtures, instrumentation and
worker-isolation proofs, coverage-backed CRAP <= 5 and zero-survivor mutation.
`CLIMON2_MUTATION_BASE` selects the changed-source comparison (default `HEAD`).

`xtask dist` requires an **arm64 macOS host** and both Rust macOS targets. It
builds isolated, unhooked packages with deployment target 15.0, merges and
ad-hoc signs each executable, and checks signatures, architectures and versions.
It writes the universal archive, version-stamped `install.sh` and `SHA256SUMS`
to `target/dist/`. Set `CLIMON2_E2E_ARCHIVE` to that archive to test it unchanged.

A `v<workspace version>` tag runs full mutation and all gates. The same final
archive must pass both architectures under bash 5, including vim and less,
before a non-draft GitHub Release and provenance attestation are published.
`-rc.N` tags are prereleases. Starting with the second published release, actual
previous-release compatibility is mandatory; the first release invents none.

**Intel runner availability is a release blocker.** If `macos-15-intel` retires,
a replacement native Intel macOS 15+ runner must pass the same artifact tests.
An unavailable runner or arm64-only success does not establish Intel support.

MIT licensed; see [LICENSE](LICENSE).
