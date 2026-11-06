# Notification metadata

**Status:** agreed semantics, not implemented; exact declarations,
[capabilities](../../../GLOSSARY.md#sessions), and wire
[schemas](../TERMS.md#schema) unresolved. See the [overview](../README.md),
[domain glossary](../../../GLOSSARY.md), and [technical terms](../TERMS.md).

The [session daemon](../../../GLOSSARY.md#sessions) recognizes notification
[OSC](../TERMS.md#osc) sequences from its command and supplies structured
metadata through the [client](../../../GLOSSARY.md#sessions) to subscribed
[monitors](../../../GLOSSARY.md#sessions) and clients. Delivery does not require
[control](../../../GLOSSARY.md#attachment-and-control), selected
[viewing](../../../GLOSSARY.md#status-and-attention), or input
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts); background and
[revoked](../../../GLOSSARY.md#attachment-and-control) subscribers can receive it.

Use the [canonical signal policy](../../session-status.md#notification-and-progress-signals)
and its upstream references. Recognized text notifications, complete logical
OSC 99 notifications, and OSC 777 `notify` produce notification metadata.
Incomplete chunks, progress operations, unsupported queries, closing operations,
and unrelated extension commands are not independent notification [events](../TERMS.md#event).
This does not advertise a complete desktop-notification implementation or
fabricate application activation/close acknowledgements.

[Daemon attention](../../../GLOSSARY.md#status-and-attention) handling and metadata relay are separate effects. A recognized
notification emits the internal `attention_request`; it does not change
[activity](../../../GLOSSARY.md#status-and-attention) or detection mode. Repeated requests while attention is already
pending do not create a new episode. Metadata delivery alone must not bypass
the status-based [toast](../TERMS.md#toast) rules in the
[notification policy](../../session-status.md#titles-and-notifications)
and generate a second notification for the same episode.

Metadata is untrusted application data, not terminal instructions or markup.
Do not forward raw sequences to the [host](../../../GLOSSARY.md#sessions), execute external actions, or infer
application consumption from delivery. This interaction permits neither live
screen mirroring nor application input/resize without control.

The [shared contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples), agreed in
[#66](https://github.com/jackgeek/climon2/issues/66), provide the single causal
[response](../TERMS.md#response)/event stream, immutable library-owned drain objects, explicit failures,
and bounded [backpressure](../TERMS.md#backpressure). Attention episodes cannot be lossily coalesced or
deduplicated by [UTC](../TERMS.md#utc) timestamps; reliable identity is required.
Arbitrary binary fields preserve their [bytes](../TERMS.md#byte), including
[NUL](../TERMS.md#nul) and invalid
[UTF-8](../TERMS.md#unicode-and-utf-8); this does not choose text encoding.

The [daemon contract](https://github.com/jackgeek/climon2/issues/67) and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) own field
names/types, source/id representation, text encoding, chunk assembly and
correlation, subscription/reconnect details, capability tokens, error codes,
and executable examples. The
[operational limits](https://github.com/jackgeek/climon2/issues/69) own queue,
assembly, rate, and exhaustion budgets.

No exact event identifier, [frame](../TERMS.md#frame), function, or
[payload](../TERMS.md#payload) encoding is assigned
here. Unknown traffic must follow those completed contracts, not an invented
silent-success rule. Unsupported operation filtering in the external terminal
protocol is not permission to hide structured-interface failures.
