# Restoration start

**Status:** planned semantics; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [client](../../../GLOSSARY.md) reports a distinct
[restoration](../../../GLOSSARY.md) beginning to the [monitor](../../../GLOSSARY.md).
Identify its [control period](../../../GLOSSARY.md) and a separate
[restoration ID](../../../GLOSSARY.md). Several restorations can occur in one
control period, including after resize. A new one must not reuse an applied
restoration's identity or leave application input enabled prematurely.

Ordered [display updates](event-display.md) follow.
[Completion](event-restoration-complete.md) identifies the same restoration.
[Revocation](../../../GLOSSARY.md) invalidates it. [Plugins](../../../GLOSSARY.md) cannot hide these essential state changes.

[Terminal integration](https://github.com/jackgeek/climon2/issues/63),
[communication](https://github.com/jackgeek/climon2/issues/66), and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must define
representation, ordering, application rules, and valid examples.
[Operational limits](https://github.com/jackgeek/climon2/issues/69) owns transfer
budgets, deadlines, and counter exhaustion; no encoding is assigned here.
