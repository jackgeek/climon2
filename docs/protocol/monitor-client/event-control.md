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
monitor application input or resize. Negotiated [plugin](../../../GLOSSARY.md) data continues even
without control.

The [ownership decision](https://github.com/jackgeek/climon2/issues/58) owns these
rules. The [communication decision](https://github.com/jackgeek/climon2/issues/66)
and [interface schemas](https://github.com/jackgeek/climon2/issues/71) must define
event types, period representation, ordering validation, errors, and examples.
Essential control changes cannot silently disappear through plugin filtering.
