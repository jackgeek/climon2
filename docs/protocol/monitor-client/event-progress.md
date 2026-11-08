# Progress metadata

**Status:** agreed semantics/wire mapping, not implemented; native
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
[grant](../../../GLOSSARY.md#attachment-and-control) control, or establish a confirmed session command outcome. A progress
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
silent loss of required records.

The [daemon contract](https://github.com/jackgeek/climon2/issues/67) assigns
`progress-v1`, retained current normalized Progress in
[Initial metadata](../daemon-client/event-initial-metadata.md), and
[Progress changed](../daemon-client/event-progress-changed.md).
States are `none`/`normal`/`indeterminate`/`error`/`paused`; percent is integer 0..100 or
null with the shared variant constraints.
The [source normalization](../daemon-client/core-v1.md#source-normalization)
retains an internal last-number cache, exposes only current Progress, emits
nothing for unchanged values and never equates 100 with command completion.
Invalid application sequences leave the indicator unchanged; invalid wire
records are not silently ignored.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) still owns
native representation, error codes, declarations and executable examples. The
[operational limits](../../monitor-operational-limits.md#payloads-and-parser-state) specify queue,
retention and exhaustion budgets.

No native event identifier, function or layout is assigned here.
No [plugin](../../../GLOSSARY.md#protocol-and-plugin-concepts) implementation
or plugin negotiation is required.
