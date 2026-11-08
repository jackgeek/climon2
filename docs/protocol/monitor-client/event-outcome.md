# Client outcome and failure

**Status:** hosting failure boundary and lifecycle agreed, not implemented; native declarations unresolved.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

When communication remains possible, the [client](../../../GLOSSARY.md) reports
an explicit completion or failure to the [monitor](../../../GLOSSARY.md).
A final notification alone proves neither daemon-accepted
[detach](../../../GLOSSARY.md) nor completed worker/resource cleanup.
Observe client thread completion and resource lifetime separately.
Containable per-target presentation/parser incompatibility explicitly makes the
target unavailable, application input/viewing off. Allow disconnected retained
cache read-only and other clients/selector/exit where safe; connected revoked
cache has no inspection route. Rendering, host-output or shared-infrastructure
failure initiates coordinated shutdown. Invalid
[UTF-8](../TERMS.md#unicode-and-utf-8) application
[bytes](../TERMS.md#byte) alone
are not a protocol error. See the [terminal failure contract](../terminal-integration.md#required-implementation-evidence);
cleanup/retry/release policy follows the
[agreed lifecycle](../../monitor-lifecycle.md).

Native crashes or aborts can terminate the monitor and every hosted client.
Ordinary errors and recoverable panics should remain local where possible,
without claiming successful cleanup or detach after failure. [Session](../../../GLOSSARY.md#sessions) survival
is not guaranteed by this hosting model.

[Library lifetime](https://github.com/jackgeek/climon2/issues/65) specifies
panic containment, instance/thread cleanup, and loaded-library safety.
[Lifecycle](../../monitor-lifecycle.md) owns recovery, terminal
[restoration](../../../GLOSSARY.md#restoration-and-presentation), and visible failures.
The [shared communication contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples), agreed in
[#66](https://github.com/jackgeek/climon2/issues/66), require closure/failure reason
to remain observable outside ordinary queue capacity. Preserve already queued
records ahead of the terminal indication. The borrowed
[notification descriptor](../TERMS.md#pollable-notification-descriptor) signals
drain progress or closure, not thread completion.

Stop new calls and polling, release delivered objects, and coordinate
dedicated-thread cleanup; outstanding objects prevent successful destruction.
Thread-safe release does not permit concurrent destruction. Neither a final
[event](../TERMS.md#event) nor stream closure proves cleanup or a joined thread.
Recoverable [daemon](../../../GLOSSARY.md#sessions)
connection loss is distinct from instance-stream closure; this contract does
not require reconnect to close/reopen the stream.
Recoverable transport/startup failures (including missing libraries) and safe
worker panics use automatic exponential passive backoff; permanent rejection
and invalid configuration stop retries. New attempts wait for previous instance
cleanup/join. Never replay indeterminate work or automatically take control.
Concurrent shutdown includes partially started/failing instances; a final
outcome erases no ownership obligation. A lost
[Leave](../daemon-client/request-leave.md) response stays unconfirmed,
even after closure/outcome/join. Cleanup stalls use bounded whole-process
fail-stop, not thread termination, unloading or a false join/detach claim.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) must assign
exact outcome types, reporting/cleanup declarations, and executable examples.
[Operational limits](https://github.com/jackgeek/climon2/issues/69) owns
startup/stall/shutdown deadlines, including clients that cannot join cleanly.
