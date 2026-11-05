# Input

**Status:** planned semantics; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [monitor](../../../GLOSSARY.md) sends literal application
[bytes](../TERMS.md#byte) only to the selected, [controlling](../../../GLOSSARY.md),
[ready](../../../GLOSSARY.md) [client](../../../GLOSSARY.md).
Preserve arbitrary bytes without lossy text conversion. Identify the current
[control period](../../../GLOSSARY.md); reject stale, revoked, or not-ready input
explicitly. The monitor must also have applied the complete current
[restoration](../../../GLOSSARY.md).

Never queue input for a future controller or send it to a background client.
Do not interpret monitor-routed bytes a second time as client shortcuts.
Success means accepted for forwarding, not consumed by the application.
[Take control](request-take-control.md) is a separate operation.

Accepted real user application input, including paste, clears either
`needs_attention` or `acknowledged` under the
[daemon attention FSM](../../session-status.md#attention). Monitor navigation,
terminal-query replies, and injected work do not count as that user event.

[Input routing](https://github.com/jackgeek/climon2/issues/64) owns literal prefix
and paste behavior. [Terminal integration](https://github.com/jackgeek/climon2/issues/63)
owns non-key input representation; the
[communication decision](https://github.com/jackgeek/climon2/issues/66) and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must define
types, ownership, errors, and examples. No binary frame format or maximum is
assigned here; numerical bounds belong to
[operational limits](https://github.com/jackgeek/climon2/issues/69).
