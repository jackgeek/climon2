# Display update

**Status:** agreed exact-terminal-byte representation and communication requirements, not implemented; native declarations pending #71.
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

Ratatui paints read-only Rust views of the monitor's own per-session `climon2-vt`.
These views are planned: current `climon2-vt` exposes no public cells and is
daemon-only. Never write session bytes directly to the host or emit duplicate
parser query replies. The daemon remains the sole query authority.

Never infer control, [readiness](../../../GLOSSARY.md), or daemon acceptance by scraping terminal [bytes](../TERMS.md#byte).
Preserve arbitrary exact ordered display bytes, including invalid
[UTF-8](../TERMS.md#unicode-and-utf-8).
Use bounded transfers and [backpressure](../TERMS.md#backpressure), not silent
loss of required updates or unbounded snapshot allocation.

The [agreed terminal contract](../terminal-integration.md) selects exact terminal
bytes, not native cross-release cells. Monitor-mode daemon bytes are canonically
serialized at effective session geometry, including background grants; parse at
those dimensions and paint local margins. Ordinary attach is unchanged.
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
exact native types, encodings, error codes, and executable examples. Chunk
boundaries never change byte order; numerical transfer budgets remain #69.
[Operational limits](../../monitor-operational-limits.md#queues-and-transfers) specifies numerical
budgets; no binary [frame](../TERMS.md#frame) maximum is assigned.

[Wire Display](../daemon-client/event-display.md) is now specified separately:
a 16-byte period/restoration header followed by nonempty exact binary bytes.
The native record carries the exact bytes with mapped authority, not that wire
header as its C layout. Complete restoration replaces the model/history
transactionally; apply before Ready and coherently paint before selected input.
No old-authority Display crosses a newer begin or revocation.
