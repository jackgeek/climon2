# Restoration-start event

**Status:** agreed meaning; wire name and [payload](../TERMS.md#payload) [schema](../TERMS.md#schema) pending; not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [helper](../../../GLOSSARY.md) tells the [monitor](../../../GLOSSARY.md) that a distinct [restoration](../../../GLOSSARY.md) is beginning.
Use the [event envelope](event.md). This page's filename does not assign its
wire `event` value.

Identify the [control period](../../../GLOSSARY.md) and a separate, monotonically increasing [restoration
ID](../../../GLOSSARY.md). Several restorations can occur within one control period, including after
resize. Beginning a new one must not reuse an already-applied restoration's
identity or leave application input enabled prematurely.

Ordered [display updates](event-display.md) follow.
[Completion](event-restoration-complete.md) identifies the same restoration.
[Revocation](../../../GLOSSARY.md) invalidates it.

## Schema pending

Helper-specific fields are tracked by
[schema completion](https://github.com/jackgeek/climon2/issues/71).

The exact event name, identity field names, and other `data` fields are not yet
assigned. No complete valid wire example is available.
[Terminal integration](https://github.com/jackgeek/climon2/issues/63) must complete
the representation/application rules.
[Operational limits](https://github.com/jackgeek/climon2/issues/69) must bound
transfer state and counter exhaustion.

Apply [shared framing and validation](../README.md). A [plugin](../../../GLOSSARY.md) cannot hide this
essential state change or turn it into permission to send input.

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
