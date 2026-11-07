# Report viewing

**Status:** agreed semantics and wire mapping, not implemented; native
[schemas](../TERMS.md#schema) unresolved. See the [overview](../README.md),
[domain glossary](../../../GLOSSARY.md), and
[technical terms](../TERMS.md).

The [monitor](../../../GLOSSARY.md#sessions) reports
[viewing](../../../GLOSSARY.md#status-and-attention) through its
[client](../../../GLOSSARY.md#sessions) to the
[session daemon](../../../GLOSSARY.md#sessions), per
[attachment](../../../GLOSSARY.md#attachment-and-control). A connected selected
[controller](../../../GLOSSARY.md#attachment-and-control) becomes viewing only after current
[restoration](../../../GLOSSARY.md#restoration-and-presentation) has been applied
and displayed [readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts)
is established. Retaining background control is insufficient.

Switching away, exiting, disconnecting, or
[revocation](../../../GLOSSARY.md#attachment-and-control) clears viewing.
Opening any menu clears this monitor's viewing without releasing control or
restoration; this supersedes the earlier selector-overlay example.
Highlighting/browsing never acknowledges. Ordinary interactive attach may
report viewing once ready and unobstructed.

Explicit ordinary attach, focus/selection and menu close/cancel may separately
acknowledge currently pending
[attention](../../../GLOSSARY.md#status-and-attention), including
selection of a session whose client already controls it, once ready and
unobstructed. Cancel deferred acknowledgement if local target context changes
before submission. Opening/browsing menus never acknowledges. Merely reporting
continued visibility must not automatically acknowledge a new request.
The [mechanics](../../session-status.md#viewing-is-not-merely-control) separate
viewing facts from the exact attention-triggering actions.

Viewing cannot authorize takeover, input, or resize, and is not proof of
application consumption. Passive reconnect does not restore control/viewing
automatically. Validate reports against current attachment, control and
restoration/readiness state; do not accept stale or unauthorized claims.

The [shared contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples) apply. Local acceptance
only queues copied data, not a daemon-confirmed viewing or acknowledgement
change. Recheck current
[connection generation](../TERMS.md#connection-generation) and authority at
execution; reject known unexecuted stale claims asynchronously rather than
discarding or assigning them to a new connection.

The [daemon contract](https://github.com/jackgeek/climon2/issues/67) assigns
[Set viewing](../daemon-client/request-viewing.md) and distinct
[Acknowledge](../daemon-client/request-acknowledge.md), with no observed episode
ID. The latter requires current controller, matching period/restoration,
readiness and latest reported viewing true; no pending episode is explicit
no-change success.

No aggregate viewing field/broadcast exists. Only this monitor's own
unobstructed presentation suppresses its toasts; another viewer never does.
Attention in a menu toasts immediately over it, without waiting for close or a
daemon round trip. See [local notification rules](../daemon-client/core-v1.md#viewing-acknowledgement-and-local-notifications).
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) still owns
native declarations and executable examples, preserving
[#66's communication rules](https://github.com/jackgeek/climon2/issues/66).
