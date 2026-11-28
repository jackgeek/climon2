# climon2-embed

[Client embedding](../../GLOSSARY.md#sessions): what it takes to run one
[client library](../../GLOSSARY.md#sessions) instance for one session on this
machine. It is the lower of the two layers
[ADR 0011](../../docs/adr/0011-monitor-library-and-client-embedding.md) splits
the monitor into (#489). The monitor uses it today; the relay will. It is an
ordinary Rust library, linked statically, and depends only on `climon2-core`
and `climon2-platform`.

It decides nothing about when an instance starts or what its records mean.
Scheduling, admission decisions, retry and the reading of records belong to
its consumer: for the monitor, the client supervisor
([`climon2-monitor`](../climon2-monitor/README.md)).

## What it holds

| Module | What it does |
| --- | --- |
| `load` | The one shared, validated load of a release's client library (`climon2_platform::library::Loader`), giving the release's calls and qualified requirements. A `Refusal` says whether the dynamic loader opened a handle, which then keeps its reservation, and whether a later load may succeed. |
| `owner` | An instance's owner thread. It creates the instance from its `Startup`, submits the first request (monitor Open as request 1, or a Kill-only instance's Kill as request 2), publishes the handles, runs the instance, and destroys it only once the consumer has released the handles. It tells the consumer each step as a `Signal`: `Published`, `Stopped` with the run report's word on its workers, then `Done`. An `Owner` dropped without `release` never destroys its instance. |
| `instance` | The client-instance interface, the `Instance` trait: the notification descriptor, shutdown, submit, Kill, drain and release. `Local` is its adapter over a loaded library's caller handles. A drain takes one delivery, hands each record to the consumer's reader and releases the delivery at once, or reports terminal evidence: its message, its retry class and what it proves of the departure. |
| `request` | The requests a consumer submits, with the authority each is judged by, and the parts of a paste. |
| `stream` | One instance's application input as its library receives it: chunks of at most `chunk_bytes`, a paste between its markers, and what a paste cut short still owes its application. Pure. |
| `backlog` | Per-session accounting of what was copied out of deliveries and not yet processed by the consumer, in bytes. |
| `stall` | The delivery stall deadline: how long a client may stay congested before its consumer fails it. Pure. |
| `admission` | The arithmetic of the admission ledger: what a release, a load and an instance each reserve, and whether reservations fit a capacity, so that every consumer counts memory and descriptors the same way. Pure. |

A remote instance, a [session stream](../../GLOSSARY.md#monitor-links) of a
monitor link, is the `Instance` interface's other adapter. It is planned and
belongs to the monitor library, not to this crate.

## Rules

The architecture guard (`xtask/tests/architecture.rs`) enforces ADR 0011's
first rule on this crate:

- Its only workspace dependencies are `climon2-core` and `climon2-platform`.
- Its only third-party dependency is `tracing`, so it links no terminal, HTTP
  or WebSocket crate.
- Its sources never name the standard streams, a printing macro, `/dev/tty` or
  the platform crate's terminal seam.

## Tests

```sh
cargo test -p climon2-embed
```

| Layer | Tests |
| --- | --- |
| Pure, in each module | Admission's reservations and amounts (`src/admission.rs`); the backlog (`src/backlog.rs`); the stall deadline (`src/stall.rs`); chunked input and pastes (`src/stream.rs`); native requests, terminal evidence, run reports, retry classes, Leave codes and failed calls (`src/instance.rs`); what each refusal means (`src/load.rs`); control periods (`src/request.rs`). |
| Over a fake library | `tests/local.rs` drives a local instance on its owner thread through a table of fake native calls: the startup create receives, the order of calls from create to destroy, both first requests, a refused first request, a refused create, a failed run, failed calls, and that an instance is never destroyed before its handles are released. |
| Over the stand-in | `tests/load.rs` (macOS and Linux) loads the [client library stand-in](../../GLOSSARY.md#testing) from a private executable home and reads a missing library's refusal. |

The monitor's own tests drive the same code through its supervisor, and the
packaged tests in `climon2-e2e` drive it against real client libraries.
