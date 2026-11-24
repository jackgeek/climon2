# climon2

Run all your terminal sessions from one terminal, and know the moment one of
them needs you.

You have a build running, tests in another tab, a dev server, and a couple of
AI coding agents working away. You keep flicking between windows to see
whether anything has finished or is waiting for an answer. climon2 does that
watching for you.

Each command runs in a climon2 **session**. The **monitor** is one terminal
that tracks every session:

- **Switch between sessions from one place.** Open the session list, type a
  few letters to find a session, and press Enter. You are now typing into it,
  as if it were a normal terminal.
- **Find out when a session needs you.** climon2 watches each session's
  output. When a busy session goes quiet, such as a build that finished or an
  agent waiting for an answer, it needs your attention. A notice pops up in the
  corner, the bell rings, and the session moves to the top of the list.
- **Jump straight to it.** One key takes you to the session that most recently
  started needing attention.
- **Leave sessions running.** Sessions keep going when you switch away, or
  when you close the monitor. Open it again later and pick up where you left
  off.

## Quick start

Install climon2 (macOS 15+ or x86_64 Linux) and add it to your PATH:

```sh
curl -fsSL https://github.com/jackgeek/climon2/releases/latest/download/install.sh | /bin/bash
export PATH="$HOME/.local/share/climon2:$PATH"
```

Start a few sessions in the background. `--headless` starts a session without
showing it:

```sh
c2 exec --headless make test
c2 exec --headless npm run dev
c2 exec --headless bash
```

Open the monitor:

```sh
c2 monitor
```

## Monitor keys

Every monitor key starts with the **escape prefix**, Ctrl+]. Press Ctrl+],
let go, then press the key:

| Keys | Action |
| --- | --- |
| Ctrl+] then `s` | Open the session list |
| Ctrl+] then `j` | Jump to the session that needs attention |
| Ctrl+] then `?` | Show the keys |
| Ctrl+] then `x` | Exit the monitor; sessions keep running |

In the session list, type to search, use the arrow keys to move, Enter to
switch, Esc to cancel, and Del to end a session. Everything else you type goes
to the session you are viewing.

## Without the monitor

You can also use one session at a time, in any terminal:

| Command | Short | What it does |
| --- | --- | --- |
| `c2 exec <command>` | `c2 x` | Start a session and show it here |
| `c2 exec --headless <command>` | `c2 x -h` | Start a session in the background and print its ID |
| `c2 attach <id>` | `c2 a` | Show a running session here |
| `c2 list` | `c2 l` | List running sessions |
| `c2 kill <id>` | `c2 k` | End a session |
| `c2 monitor` | `c2 m` | Open the monitor |

Press Ctrl+] then `d` to step away from a session (detach); it keeps running.
An ID can be shortened to its first few characters.

## Learn more

- The [user guide](docs/for-humans/guide/README.md) covers installing,
  the monitor, every command, configuration and known limitations.
- [Development and releases](docs/for-humans/development.md) is for people
  who want to work on climon2.
- The [documentation index](docs/for-humans/README.md) lists everything else,
  including the protocols and the design.

## License

climon2 is open source software under the [MIT License](LICENSE).
