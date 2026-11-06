# Control event

**Status:** agreed ownership behavior; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [client](../../../GLOSSARY.md) reports [grants](../../../GLOSSARY.md) and
[revocations](../../../GLOSSARY.md) to the [monitor](../../../GLOSSARY.md),
identifying the applicable [control period](../../../GLOSSARY.md).
A grant starts [restoration](../../../GLOSSARY.md), not input [readiness](../../../GLOSSARY.md).
Revocation invalidates readiness and pending restoration; show the
[placeholder](../../../GLOSSARY.md), not cached content as current.

[Hand-back](../../../GLOSSARY.md) may restore background control without
changing selection. That client resumes screen maintenance but receives no
monitor application input or resize. Core title/status reports continue even
without control; [plugin](../../../GLOSSARY.md) implementation is deferred.

[Viewing](../../../GLOSSARY.md#status-and-attention) is separate from retained control.
See [viewing reports](request-viewing.md). A grant or
hand-back alone does not establish a selected, displayed presentation.
Revocation clears viewing. Explicit selection/takeover can acknowledge
[attention](../../../GLOSSARY.md#status-and-attention), but merely being visible must
not automatically acknowledge a new attention request.

The [ownership decision](https://github.com/jackgeek/climon2/issues/58) owns these
rules. The [communication decision](https://github.com/jackgeek/climon2/issues/66)
owns the agreed [shared contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples). One per-instance
[FIFO](../TERMS.md#fifo) [response](../TERMS.md#response)/[event](../TERMS.md#event) stream preserves causal grant,
restoration, revocation, and result order. Deliveries cannot be dropped,
overwritten, or lossily coalesced under [backpressure](../TERMS.md#backpressure). Identify the current
[connection generation](../TERMS.md#connection-generation) separately from the
control period; old authority never transfers across reconnect.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) must define
exact event tags, identity fields, error codes, and executable examples.
Essential control changes cannot silently disappear through plugin filtering.
