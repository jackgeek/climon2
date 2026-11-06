# Display update

**Status:** agreed communication requirements; terminal representation unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [client](../../../GLOSSARY.md) delivers ordered updates to the
[monitor](../../../GLOSSARY.md), identifying their
[control period](../../../GLOSSARY.md) and any [restoration](../../../GLOSSARY.md).
[Controlling](../../../GLOSSARY.md) clients maintain screens even in the background. [Revoked](../../../GLOSSARY.md) clients
do not provide live core read-only mirroring; restoration brings the view
current on regaining control. [Plugin](../../../GLOSSARY.md) status is independent of screen access.

[Daemon title updates](event-title.md) and
[session status](../../../GLOSSARY.md#status-and-attention) reports are core data independent of
screen access. Supported updates reach passive/background/revoked clients
without display scraping or implementing plugins.
See [status delivery](event-session-status.md).

The monitor retains a per-session `climon2-vt` model and bounded local scrollback
for [disconnected inspection](../../session-status.md#connection-loss-and-retained-terminal-views).
Cached content is explicitly stale/read-only, not a continuation of live
mirroring; reconnection alone does not make it current.

Ratatui is the required monitor renderer; its session-state integration remains
with the terminal decision.

Never infer control, [readiness](../../../GLOSSARY.md), or daemon acceptance by scraping terminal [bytes](../TERMS.md#byte).
Preserve arbitrary display bytes when the chosen representation requires them.
Use bounded transfers and [backpressure](../TERMS.md#backpressure), not silent
loss of required updates or unbounded snapshot allocation.

[Terminal integration](https://github.com/jackgeek/climon2/issues/63) must choose
cells/state, terminal-output bytes, or another explicit representation.
The [shared communication contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples), agreed in
[#66](https://github.com/jackgeek/climon2/issues/66), require tagged [C-compatible](../TERMS.md#c-abi)
records, literal binary preservation, and immutable library-owned drain objects
released through the same library. There is one causal
[response](../TERMS.md#response)/[event](../TERMS.md#event) stream;
full queues backpressure ordinary production and socket reading rather than
lose required updates. Bounds include bytes, records, and unreleased deliveries.
Connection loss does not refresh the retained monitor-owned
[virtual terminal](../../../GLOSSARY.md#sessions); input still needs fresh
control/restoration/application after reconnect.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) must specify
exact types, encodings, error codes, and executable examples. Chunking remains
with terminal integration and operational limits.
[Operational limits](https://github.com/jackgeek/climon2/issues/69) owns numerical
budgets; no binary [frame](../TERMS.md#frame) maximum is assigned.
