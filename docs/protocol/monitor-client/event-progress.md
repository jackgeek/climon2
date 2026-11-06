# Progress metadata

**Status:** agreed semantics, not implemented; exact declarations,
[capabilities](../../../GLOSSARY.md#sessions), and wire
[schemas](../TERMS.md#schema) unresolved. See the [overview](../README.md),
[domain glossary](../../../GLOSSARY.md), and [technical terms](../TERMS.md).

The [session daemon](../../../GLOSSARY.md#sessions) interprets recognized
progress [OSC](../TERMS.md#osc) operations and relays structured information
through the [client](../../../GLOSSARY.md#sessions) to subscribed
[monitors](../../../GLOSSARY.md#sessions) and clients. Background and
[revoked](../../../GLOSSARY.md#attachment-and-control) subscribers can receive
updates without acquiring control or reaching input
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts).

For example, the [OSC 9;4 progress operations](https://conemu.github.io/en/AnsiEscapeCodes.html#ConEmu_specific_OSC)
are progress metadata, not text notifications. See the
[canonical signal policy](../../session-status.md#notification-and-progress-signals).
Distinguish progress from completed notification metadata and unsupported
control operations; merely observing an OSC number is not sufficient.

Progress does not emit `attention_request`, change
[activity](../../../GLOSSARY.md#status-and-attention)/detection mode,
grant control, or establish a confirmed session command outcome. A progress
operation's error indicator is not the daemon's terminal `failed` status.
Relaying progress enables future progress-bar/spinner [UI](../TERMS.md#ui); that UI is not part
of this revision.

Do not forward raw sequences to [host](../../../GLOSSARY.md#sessions) terminals or interpret untrusted metadata
as executable terminal instructions. Delivery is not live terminal-content
mirroring and does not permit application input or resize without control.

The [shared contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples), agreed in
[#66](https://github.com/jackgeek/climon2/issues/66), supply tagged [C-compatible](../TERMS.md#c-abi)
records, immutable library-owned drain objects, the single causal
[response](../TERMS.md#response)/[event](../TERMS.md#event) stream, explicit failures,
and bounded [backpressure](../TERMS.md#backpressure) rather than
silent loss of required records. This assigns no new progress retention or
coalescing policy.

The [daemon contract](https://github.com/jackgeek/climon2/issues/67) and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must specify
accepted sources, required/optional fields, determinate and unknown progress and
clear/reset representation, units/ranges, encoding, identity, ordering,
subscription/initial state, reconnect/staleness, capability tokens, error codes,
and executable examples. Current-progress retention is not
implicitly selected by this document. The
[operational limits](https://github.com/jackgeek/climon2/issues/69) own queue,
rate, retention, and exhaustion budgets.

No exact event identifier, function, [frame](../TERMS.md#frame), or
[payload](../TERMS.md#payload) encoding is assigned
here. No [plugin](../../../GLOSSARY.md#protocol-and-plugin-concepts) implementation
or plugin negotiation is required.
