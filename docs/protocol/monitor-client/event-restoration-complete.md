# Restoration completion

**Status:** planned semantics; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [client](../../../GLOSSARY.md) reports completion after the ordered updates
for the [restoration](../../../GLOSSARY.md) identified at
[start](event-restoration-start.md), in the same
[control period](../../../GLOSSARY.md).

Receiving completion is not enough: the [monitor](../../../GLOSSARY.md) must
apply the complete current restoration before enabling application input.
Open or [takeover](../../../GLOSSARY.md) acceptance cannot substitute for this barrier. [Revocation](../../../GLOSSARY.md) or
a newer restoration invalidates earlier [readiness](../../../GLOSSARY.md);
obsolete completion must not enable input.

[Terminal integration](https://github.com/jackgeek/climon2/issues/63),
[communication](https://github.com/jackgeek/climon2/issues/66), and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must define
identity fields, completion/application ordering, rejection, and valid examples.
[Operational limits](https://github.com/jackgeek/climon2/issues/69) owns remaining
deadlines and transfer budgets.
