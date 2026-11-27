# The browser is a disposable renderer of filtered canonical presentation bytes

A [web view](../../GLOSSARY.md#web-monitor) draws session content with xterm.js, a terminal emulator that runs in the browser page. We decided that the [web monitor](../../GLOSSARY.md#web-monitor) forwards the [canonical presentation](../../GLOSSARY.md#restoration-and-presentation) as bytes for xterm.js to interpret, and keeps no terminal model of its own. The only model on the monitor's side is the one the [monitor library](../../GLOSSARY.md#sessions) already keeps for each session ([ADR 0011](0011-monitor-library-and-client-embedding.md)). xterm.js is a renderer whose state is never read back and can be thrown away at any moment. This keeps one authority for terminal state, keeps `climon2-web` free of `climon2-vt`, and lets every recovery case use one mechanism. It is decided in [#240](https://github.com/jackgeek/climon2/issues/240).

This is a plan. Nothing here is built yet.

## What follows from it

- **A restoration on demand.** Whenever a web view needs a session's current state, the monitor library serializes a fresh [restoration](../../GLOSSARY.md#restoration-and-presentation) from its model, followed by live canonical bytes from that snapshot point. The web monitor announces it with a `show` message that carries the session, a restoration number and the session's columns and rows. The web view discards its xterm.js state, sizes xterm.js to exactly that size and writes the bytes. Switching session, reconnecting, a change of the session's size and catching up after falling behind are all the same `show`. Nothing is replayed from a log.
- **Only the shown session's bytes cross.** Every other session reaches the web view as structured rows.
- **The browser filter.** xterm.js treats the code points U+0080 to U+009F as sequence introducers, and the session daemon's filter forwards them as text. The web monitor therefore drops them, and strips the re-emitted `OSC 2` title, from every terminal byte it sends. Only C0 controls, printable text, and `ESC`-introduced CSI sequences and two-byte escapes reach a browser. The engine behind the model ignores those code points too, so dropping them keeps xterm.js and the model in agreement. The title reaches the web view as structured text. [ADR 0007](0007-daemon-owned-virtual-terminal.md) and the session daemon's filter are unchanged.
- **Nothing is read back from xterm.js.** Its title event is unused, and no query reaches it, so it never produces a reply. The web view still disables window operations, loads no clipboard, image or link addon, installs a link handler that does nothing, and swallows every OSC, DCS, APC and query with parser hooks. That is a second layer behind the browser filter, each part with a test.
- **A slow browser loses frames, never correctness.** A page cannot push back on a WebSocket, so the web view acknowledges bytes once xterm.js has parsed them, and the web monitor holds a bounded window of unacknowledged bytes for each web view. When the window and queue are full, the web monitor discards that web view's queued content and, once acknowledgements resume, sends a new `show`. The monitor library is never slowed by a browser.
- **Input needs a painted, current frame.** Input from the web view carries the restoration number it was typed against. The web monitor drops input for any other number, and all input until the web view has acknowledged the whole current restoration. Until then the web view also counts as covered, so [viewing](../../GLOSSARY.md#status-and-attention) is never true for content the browser has not drawn.
- **xterm.js encodes input.** Its modes follow the same bytes as the model's, so the web view sends what xterm.js produces as opaque bytes. Paste is a separate message so that the monitor library does the bracketing, and focus is a structured report.
- **No compatibility between versions.** The page and the web monitor ship in one version folder. The page states its build identifier in its first message, and on a mismatch the web monitor closes the connection with a code that makes the page reload. Either side closes the connection on a message it does not expect.

## Considered Options

- **Send cells from the model's read-only view and paint them in the page.** Rejected. It replaces xterm.js with a renderer of our own, and with it selection, scrollback, input encoding and accessibility in the browser.
- **A second model in the web monitor for each web view.** Rejected. It doubles terminal memory for each browser, adds a second authority that can disagree with the first, and makes `climon2-web` depend on `climon2-vt`.
- **Neutralise U+0080 to U+009F in the session daemon's filter.** Rejected. It changes agreed daemon behaviour for every consumer to fix a problem only xterm.js has. A real terminal reached through a client does not treat these code points as introducers in UTF-8.
- **Keep a log of canonical bytes to replay to a reconnecting browser.** Rejected. It is a second, unbounded copy of what the model already holds, and it repeats output a restoration is defined never to repeat.

## Consequences

- The monitor's model cannot serialize a restoration today: the serializers in `climon2-vt` are private and called only for the session daemon. Exposing one on the model is new work in `climon2-vt` and the monitor library. The daemon wire is untouched.
- A `show` carries the session's whole retained history, up to a cap set with the web monitor's limits. With the default 10,000 rows, switching session on a phone can cost a megabyte or more.
- `climon2-web` does not depend on `climon2-vt`. The rule that only the session daemon and the monitor library do stays as ADR 0011 states it.
- Feature parity with the terminal interface holds by construction: the web view sends exactly one message for each action in the monitor library's closed set, and a test fails when an action has none.
