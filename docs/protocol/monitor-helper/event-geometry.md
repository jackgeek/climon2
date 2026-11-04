# Geometry event

**Status:** agreed meaning; wire name and [payload](../TERMS.md#payload) [schema](../TERMS.md#schema) pending; not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [helper](../../../GLOSSARY.md) reports actual geometry, separately from accepting a
[resize request](request-resize.md). Use the [event envelope](event.md).

Only the selected [controlling](../../../GLOSSARY.md) [session](../../../GLOSSARY.md) follows the [monitor](../../../GLOSSARY.md)'s available size.
Background sessions retain their last size until selected; [hand-back](../../../GLOSSARY.md) alone
does not trigger a monitor resize.

## Schema pending

Helper-specific fields are tracked by
[schema completion](https://github.com/jackgeek/climon2/issues/71).

The event name, size fields/types, clamping and available-area rules, relationship
to [restoration](../../../GLOSSARY.md), and complete example remain with
[terminal integration](https://github.com/jackgeek/climon2/issues/63).
Do not assume the current private [daemon](../../../GLOSSARY.md) geometry schema is the helper schema.

Common [frame](../TERMS.md#frame) limits apply. Other numerical bounds remain with
[operational limits](https://github.com/jackgeek/climon2/issues/69).
Use [shared compatibility/validation](../README.md#compatibility-errors-and-limits).

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
Related: [restoration start](event-restoration-start.md).
