# Report viewing

**Status:** agreed semantics, not implemented; exact declarations and wire
schemas unresolved. See the [overview](../README.md),
[domain glossary](../../../GLOSSARY.md), and
[technical terms](../TERMS.md).

The [monitor](../../../GLOSSARY.md#sessions) reports
[viewing](../../../GLOSSARY.md#status-and-attention) through its
[client](../../../GLOSSARY.md#sessions) to the
[session daemon](../../../GLOSSARY.md#sessions), per
[attachment](../../../GLOSSARY.md#attachment-and-control). A connected selected
controller becomes viewing only after current
[restoration](../../../GLOSSARY.md#restoration-and-presentation) has been applied
and displayed [readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts)
is established. Retaining background control is insufficient.

Switching away, exiting, disconnecting, or revocation clears viewing.
Opening the selector overlay preserves viewing of the selected session;
highlighting another row does not acknowledge it. Ordinary interactive attach
counts as viewing while it controls the session.

Explicit selection/takeover acknowledges pending attention, including
selection of a session whose client already controls it. Merely reporting
continued visibility must not automatically acknowledge a new request.
The [mechanics](../../session-status.md#viewing-is-not-merely-control) separate
viewing facts from the exact attention-triggering actions.

Viewing cannot authorize takeover, input, or resize, and is not proof of
application consumption. Passive reconnect does not restore control/viewing
automatically. Validate reports against current attachment, control and
restoration/readiness state; do not accept stale or unauthorized claims.

The [daemon contract](https://github.com/jackgeek/climon2/issues/67),
[communication decision](https://github.com/jackgeek/climon2/issues/66), and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must define
the operation/acknowledgement distinction, fields, identities, validation,
ordering, explicit errors, compatibility, ownership, and valid examples.
No exact call, message type, or wire example is assigned here.
