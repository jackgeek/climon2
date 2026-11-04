# Resize request

**Status:** agreed behavior; argument/result [schemas](../TERMS.md#schema) pending; not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [monitor](../../../GLOSSARY.md) sends `op: "resize"` through the [request envelope](request.md) for
the selected [controlling](../../../GLOSSARY.md) [session](../../../GLOSSARY.md). It identifies the [control period](../../../GLOSSARY.md) and requested
columns and rows.

A controlling resize may establish initial geometry before [restoration](../../../GLOSSARY.md) becomes
[ready](../../../GLOSSARY.md). This exception does not enable application input. Background [helpers](../../../GLOSSARY.md) get
no monitor resize requests and retain their last size until selected.

Success means [daemon](../../../GLOSSARY.md) acceptance, not proof of the final dimensions or applied
restoration. Use [geometry](event-geometry.md) and
[restoration](event-restoration-start.md) [events](../TERMS.md#event) for those observations.
[Revoked](../../../GLOSSARY.md) or stale-period requests are rejected.

## Unresolved schema

Helper-specific fields are tracked by
[schema completion](https://github.com/jackgeek/climon2/issues/71).

Argument field names/types, actual geometry/clamping, complete result, error-code
mapping, and any restoration effects remain to be completed. Do not copy the
current private daemon `cols`/`rows` schema into this helper interface by
assumption. No complete wire example or binary [payload](../TERMS.md#payload) is assigned.

[Terminal integration](https://github.com/jackgeek/climon2/issues/63) owns
geometry/application rules; [operational limits](https://github.com/jackgeek/climon2/issues/69)
owns remaining numerical budgets and deadlines.
Apply [shared validation](../README.md#compatibility-errors-and-limits).

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
