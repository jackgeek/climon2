# climon2-relay

The `climon2-relay` executable: the relay of
[monitor links](../../GLOSSARY.md#monitor-links) (spec #488). A user starts it
with `c2 relay` or `c2 r`; the CLI hands it the arguments it was given and
runs it from its own version folder, as it runs the monitor (#194). It runs
in the foreground, owns its terminal until it stops, and its screen is the
only way to control it: there is no control socket and no stop or status
command.

So far it binds its listener, shows a minimal screen and stops cleanly
(#499). Its pair key mints a link invitation, a hub monitor pairs with it,
and it accepts links from the hub monitors it remembers (#501). The sessions
it serves, the dashboard's panes and the upgrade restart come with later
tickets.

## What it does

- **Arguments.** One flag, `--listen <ip>:<port>`, parsed by
  `climon2_core::relay`. The flag overrides the `relay.listen` setting, read
  once at start; with neither, the relay listens on `127.0.0.1:7222`. A listen
  value is an IP literal and a port; a host name or a missing port is a usage
  error. Port 0 lets the system pick, and the screen shows the port bound.
- **Start.** In order, before the terminal changes: a usage error exits 2
  without touching the climon2 home. Then the relay opens its general log
  (`logs/general/relay-*.log`), needs a terminal, reads the configuration,
  takes the home's relay lock (`links/relay.lock`, owner-only, in the
  owner-only `links/` folder), reads its pairing store (making its identity
  in `links/relay-identity.json` on its first start) and binds its listener
  once. Each failure is a
  start failure: status 125, with the reason and the log's name on stderr. A
  second relay in the same home is refused and names the running one.
- **Screen.** A green status bar reading `Listening`, the bound address on
  the row under it, the link invitation under that, and a key bar on the
  last row: `p Pair  s Stop`.
- **Pairing.** `p` mints a link invitation (5 minutes, 3 attempts) and shows
  its PIN and the address to pair with; minting again revokes the last one.
  Once a hub monitor pairs, the screen shows its name and key fingerprint
  instead, and the relay remembers it in `links/hub-monitors.json` before it
  sends its confirmation. An expired or used-up invitation says so.
- **Links.** Every connection gets a thread running the link code's relay
  end. A remembered hub monitor links, and the relay records when, under
  the name its Hello suggests; any other key gets Close "not paired".
- **Stop.** `s` stops it with status 0, SIGTERM with 143, and a closing
  terminal (SIGHUP, or its input closing) with 129. Each stop restores the
  terminal and names no log. The listener and the lock go with the process.

On Windows it refuses to start (`windows.rs`): monitor links are specified
for macOS and Linux only.

## Modules

| Module | What it does |
| --- | --- |
| `unix` | `run`: parse, open the log, start, serve, record how the run ended. |
| `start` | Everything that can refuse the start, checked in order: the terminal, the configuration, the home's lock, the pairing store and the listener. |
| `links` | The listener's connections, each run over the link code on a thread, and the remembered hub monitors with their one writer. |
| `desk` | The link invitation as the relay's user handles it: mint, and what the screen shows of it. |
| `host` | The terminal while the relay owns it: raw mode, the alternate screen, signals, input and why the relay stopped. |
| `screen` | What the screen shows, drawn with Ratatui. |
| `keys` | The relay's keys: one letter per action. |
| `windows` | The refusal on Windows. |

## Rules

The architecture guard (`xtask/tests/architecture.rs`) allows the relay only
`climon2-core`, `climon2-platform` and `climon2-link` among workspace
crates, so it never
depends on the monitor library (ADR 0011, rule 2) or on `climon2-vt`
(rule 4).

## Tests

- `tests/relay.rs` runs the executable in a real terminal with no CLI in
  front of it: what it binds and shows, each start failure and each stop.
- Unit tests in `start.rs`, `host.rs`, `keys.rs`, `screen.rs`, `desk.rs`,
  `unix.rs` and `links.rs`; the last pair and link real hub monitor ends of
  the link code with the relay over loopback.
- `tests/windows.rs` checks the refusal on Windows.
- The CLI's hand-off is tested in `climon2-cli/tests/relay.rs`.
