# Take control

**Status:** agreed ownership behavior; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Initial selection, selector confirmation, and explicit
[take control](../../../GLOSSARY.md) acquire control when needed.
The [global lowercase j attention jump](../terminal-integration.md#attention-jump-and-configurable-hints)
is also explicit selection/acquisition when needed. Stale cache inspection is a
separate read-only action and must never acquire.
Confirming a
[revoked](../../../GLOSSARY.md) selected [session](../../../GLOSSARY.md#sessions) can reclaim it; confirming an
already-controlled session retains it. Last-request-wins semantics remain.
Discovery, eager background connections, [plugin](../../../GLOSSARY.md) traffic, and browsing/cancelling
the selector must not request takeover.

Success means [daemon](../../../GLOSSARY.md#sessions) acceptance, not [readiness](../../../GLOSSARY.md).
Observe [control](event-control.md), [restoration](../../../GLOSSARY.md#restoration-and-presentation),
and monitor-side application before enabling input.
Selected input additionally waits for coherent paint; a background grant may
apply restoration/report Ready without selection, resize, paint, or input.
See [restoration start](event-restoration-start.md).

The [shared contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples) apply. Local submission
acceptance is only validated queuing, not takeover acceptance. Check required
[capabilities](../../../GLOSSARY.md#sessions) through both library and daemon
before acquisition; unsupported work fails explicitly. Do not silently execute
an old [connection generation](../TERMS.md#connection-generation)'s queued
takeover against a new connection.

The [ownership decision](https://github.com/jackgeek/climon2/issues/58) owns these
rules. The [daemon contract](https://github.com/jackgeek/climon2/issues/67)
distinguishes passive connections from control acquisition in
[wire Acquire](../daemon-client/request-acquire.md). Acquisition has no implicit
resize or [attention](../../../GLOSSARY.md#status-and-attention) acknowledgement. Already controlling returns no-change
without another [grant](../../../GLOSSARY.md#attachment-and-control)/restoration. [Succession](../daemon-client/core-v1.md#membership-and-succession)
may promote a passive [client](../../../GLOSSARY.md#sessions)
without selecting, resizing, sending input or
acknowledging; promoted membership persists on switch/revocation.
The [communication decision](https://github.com/jackgeek/climon2/issues/66) owns
the shared rules.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) must assign
exact calls, arguments, outcomes, errors, and executable examples.
