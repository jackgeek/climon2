# Monitor library and client embedding

The web monitor and the relay both need behaviour that today lives inside the monitor's executable, beside its Ratatui terminal interface. We decided to split that code into two layers of ordinary Rust crates, linked statically into each executable. The lower layer, **client embedding** (`climon2-embed`), runs client library instances; the monitor, the web monitor and the relay all use it. The upper layer, the **monitor library** (`climon2-monitor`), is a whole monitor with no interface of its own; the monitor's terminal interface (`climon2-monitor-tui`) and the web monitor (`climon2-web`) are consumers of it, and the relay never depends on it. This lets the terminal interface be replaced and iterated without changing the library, and gives the terminal interface and the web view the same capabilities from one implementation. It is decided in [#237](https://github.com/jackgeek/climon2/issues/237).

This is a plan, built in parts. Client embedding (#489), and the monitor library with its terminal interface (#490), are built, and [ADR 0004](0004-crate-layout-and-dependency-direction.md) records them; until the rest is, ADR 0004 describes the workspace as it is.

## The two layers

**Client embedding** holds exactly what it takes to run one client instance for one session on this machine: loading and qualifying a release's client library, the owner thread, requests in and deliveries and the run report out, the input stream, per-session backlog accounting, the stall deadline, and the arithmetic of the admission ledger, so that a monitor and a relay count memory the same way. It defines the client-instance interface. A local instance (a loaded library) is one adapter of that interface, and lives in this layer. A remote instance (a [session stream](../../GLOSSARY.md#monitor-links)) is the other, and lives in the monitor library.

**The monitor library** holds everything that decides when instances start and what they mean: discovery scheduling, fair queues, admission, backoff and retry, the registry, selection and initial selection, control, status, viewing, acknowledgement, the attention jump, run-local dismissal, Kill and Delete, each session's `climon2-vt` model, memory relief, the selector's rows, order, labels and search matching, the toast queue with its expiry and dismissal rules, reading configuration and opening the log, shutting clients down within their deadlines with a verdict on the cleanup, and the hub monitor's side of monitor links.

**Link code** (`climon2-link`) is a third crate: pairing, the pinned-key handshake, framing and link revisions over a plain byte stream, with no platform dependency. The monitor library holds the hub side built on it; the relay's package (`climon2-relay`) holds the relay side.

## The seam between the library and its consumers

The seam is a Rust interface inside one process.

- **Driving.** The library is a synchronous object that one consumer thread owns and steps. A step takes the current time and returns the events since the last step; the consumer borrows session state directly. The library imposes no async runtime and offers something to wait on, so a consumer need not poll.
- **Inwards.** The actions are one closed set: select, take control, input, paste, resize, kill, delete, dismiss, acknowledge, the attention jump, the link actions and the rest. Each consumer must handle every member, so an action missing from one fails its build. Application input crosses as opaque bytes for the current target, and paste as its own action. The library keeps the rules that belong to no device: which session is the target, cutting input when the target changes, bracketing a paste, refusing input without control.
- **Outwards.** The library offers session rows, current toasts, each session's input modes, and terminal content in two forms: a read-only cell view and the canonical presentation as bytes. Effects on a device are events the consumer delivers its own way: host attention, a toast's bell, an accepted clipboard copy.
- **What a consumer reports.** Only what the library cannot know: whether something covers the selected session (a modal or a toast), whether the user's window has focus, and the size it can show.
- **What stays in a consumer.** Decoding its device's input (the client escape prefix, key bindings, host mouse events), navigation inside the selector (open or closed, highlight, paging, clicks), placement and hit-testing of toasts, painting, entering and releasing its device, the exit status, and calling fail-stop when the library's verdict demands it. The key introduction and its preference file belong to the terminal interface alone. Browser pairing and managing connected browsers belong to the web monitor alone.
- **Testing.** The library takes the time, the client library loader and the climon2 home as inputs, so tests drive a whole monitor with no terminal and no browser.

## Rules the architecture guard enforces

1. `climon2-monitor` and `climon2-embed` never depend on Ratatui, crossterm or an HTTP or WebSocket crate, and never read or write the process's terminal.
2. `climon2-relay` never depends on `climon2-monitor`.
3. `climon2-link` has no platform dependency and no per-platform conditional.
4. Only the session daemon and the monitor library depend on `climon2-vt`. Consumers of the library get its read-only view types re-exported.

Rules 1 and 4 amend ADR 0004 when the split is built: Ratatui moves from "this package only" to `climon2-monitor-tui` and `climon2-relay`, and the `climon2-vt` exception moves from the monitor to the monitor library.

## Considered Options

- **A loaded dynamic library with a frozen interface, like the client library.** Rejected. That form exists so one program can host other releases' code. The monitor, the web monitor and the relay ship in one version folder and never load each other's releases, and a frozen interface would make every iteration of the terminal interface a compatibility question.
- **One layer that the relay also links.** Rejected. The relay creates a client instance only when the hub monitor opens a session stream, never retries by itself, and has no selection, attention or viewing ([ADR 0009](0009-relay-terminates-daemon-wire.md)). It would carry a scheduler and a registry it must never use.
- **A library that stops at sessions and their clients, leaving selection and viewing to each consumer.** Rejected. The attention rules, toast timing and search matching would be written twice and drift, which defeats parity.
- **A library that runs its own thread and talks over channels.** Rejected. Every read of a terminal's cells would need a lock or a copied snapshot, slowing the paint path. The web monitor instead runs one thread that owns the library and fans events out to its connections.

## Consequences

- The split is a behaviour-preserving restructuring of the implemented monitor, done before any web monitor or relay code. The existing real-terminal tests must pass unchanged, and no agreed monitor behaviour, daemon wire revision or other ADR is reopened.
- The supervisor's state, which today mixes running instances with scheduling them, is divided along the client-embedding line. Viewing no longer reads the terminal interface's rectangles: the consumer reports what is covered. Selector navigation and toast hit-testing stop taking Ratatui types.
- Canonical presentation bytes are not kept today: they are fed to the model and dropped. Offering them is new work, and [#240](https://github.com/jackgeek/climon2/issues/240) decides which form the web monitor consumes.
- Whether one web monitor holds one selection and one toast queue, or one per web view, is decided by [#239](https://github.com/jackgeek/climon2/issues/239). The interface must not rule either out.
- The `test-hooks` feature is plumbed through the new crates.
