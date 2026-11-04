# Resize

**Status:** planned semantics; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [monitor](../../../GLOSSARY.md) requests geometry only for the selected
[controlling](../../../GLOSSARY.md) [session](../../../GLOSSARY.md), identifying
its [control period](../../../GLOSSARY.md). A controlling resize may establish
initial geometry before [restoration](../../../GLOSSARY.md) is ready; this does
not enable application input. Reject stale-period or revoked resize explicitly.

Background clients receive no monitor resize requests. Daemon acceptance does
not prove final geometry or applied restoration; observe
[geometry](event-geometry.md) and restoration separately.

[Terminal integration](https://github.com/jackgeek/climon2/issues/63) owns
columns/rows, clamping, available area, and restoration effects.
The [communication decision](https://github.com/jackgeek/climon2/issues/66) and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must assign
types, results, errors, and valid examples. Numerical budgets belong to
[operational limits](https://github.com/jackgeek/climon2/issues/69).
