# Take control

**Status:** agreed ownership behavior; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Initial selection, selector confirmation, and explicit
[take control](../../../GLOSSARY.md) acquire control when needed. Confirming a
[revoked](../../../GLOSSARY.md) selected session can reclaim it; confirming an
already-controlled session retains it. Last-request-wins semantics remain.
Discovery, eager background connections, [plugin](../../../GLOSSARY.md) traffic, and browsing/cancelling
the selector must not request takeover.

Success means daemon acceptance, not [readiness](../../../GLOSSARY.md).
Observe [control](event-control.md), [restoration](event-restoration-start.md),
and monitor-side application before enabling input.

The [ownership decision](https://github.com/jackgeek/climon2/issues/58) owns these
rules. The [daemon contract](https://github.com/jackgeek/climon2/issues/67)
must distinguish passive connections from control acquisition.
The [communication decision](https://github.com/jackgeek/climon2/issues/66) and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must assign
calls, arguments, outcomes, errors, and valid examples.
