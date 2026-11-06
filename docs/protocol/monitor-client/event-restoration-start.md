# Restoration start

**Status:** agreed communication requirements; interface [schema](../TERMS.md#schema) unresolved, not implemented.
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

[Terminal integration](https://github.com/jackgeek/climon2/issues/63) and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must define
exact terminal representation, identity fields, application rules, and
executable examples.
The [shared contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples), agreed in
[#66](https://github.com/jackgeek/climon2/issues/66), require one causal
[response](../TERMS.md#response)/[event](../TERMS.md#event) stream.
The [grant](../../../GLOSSARY.md#attachment-and-control) precedes this start and its ordered updates
and completion. No lossy coalescing or dropped boundaries is permitted.
The [connection generation](../TERMS.md#connection-generation) remains distinct
from both restoration and control identities.
[Operational limits](https://github.com/jackgeek/climon2/issues/69) owns transfer
budgets, deadlines, and counter exhaustion; no encoding is assigned here.
