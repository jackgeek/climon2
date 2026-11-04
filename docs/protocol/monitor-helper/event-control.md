# Control event

**Status:** agreed behavior; complete `data` [schema](../TERMS.md#schema) pending; not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [helper](../../../GLOSSARY.md) reports [grants](../../../GLOSSARY.md) and [revocations](../../../GLOSSARY.md) to the [monitor](../../../GLOSSARY.md) using the
[event envelope](event.md) with `event: "control"`.
The event identifies the applicable [control period](../../../GLOSSARY.md).

A grant starts [restoration](../../../GLOSSARY.md). It does not make application input [ready](../../../GLOSSARY.md).
Revocation invalidates readiness and pending restoration. The monitor must not
show cached content as current or provide live read-only core screen mirroring.

[Hand-back](../../../GLOSSARY.md) can grant a background helper control without selecting its [session](../../../GLOSSARY.md).
That helper resumes screen maintenance but receives no monitor keyboard input
or resize [requests](../TERMS.md#request).

## Schema pending

Tracked by [schema completion](https://github.com/jackgeek/climon2/issues/71).

The exact grant/revocation [discriminator](../TERMS.md#discriminator), period field, and full `data` shape
are not assigned by the resolution. No complete valid control-event example
can be given without choosing those fields.

Control events must preserve restoration ordering. Impossible transitions and
unknown core event names fail the channel; [plugins](../../../GLOSSARY.md) cannot silently suppress
essential state changes. Use [common validation and limits](../README.md#compatibility-errors-and-limits).

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
Related: [take control](request-take-control.md),
[restoration start](event-restoration-start.md), and [input](request-input.md).
