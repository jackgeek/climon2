# Display update

**Status:** planned semantics; representation unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [client](../../../GLOSSARY.md) delivers ordered updates to the
[monitor](../../../GLOSSARY.md), identifying their
[control period](../../../GLOSSARY.md) and any [restoration](../../../GLOSSARY.md).
[Controlling](../../../GLOSSARY.md) clients maintain screens even in the background. [Revoked](../../../GLOSSARY.md) clients
do not provide live core read-only mirroring; restoration brings the view
current on regaining control. [Plugin](../../../GLOSSARY.md) status is independent of screen access.

[Daemon title updates](event-title.md) are also independent of screen access.
They must reach passive/background/revoked clients without display scraping.

Ratatui is the required monitor renderer; its session-state integration remains
with the terminal decision.

Never infer control, [readiness](../../../GLOSSARY.md), or daemon acceptance by scraping terminal [bytes](../TERMS.md#byte).
Preserve arbitrary display bytes when the chosen representation requires them.
Use bounded transfers and [backpressure](../TERMS.md#backpressure), not silent
loss of required updates or unbounded snapshot allocation.

[Terminal integration](https://github.com/jackgeek/climon2/issues/63) must choose
cells/state, terminal-output bytes, or another explicit representation.
[Communication](https://github.com/jackgeek/climon2/issues/66) and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must specify
types, ownership, ordering, errors, and examples.
[Operational limits](https://github.com/jackgeek/climon2/issues/69) owns numerical
budgets; no binary frame maximum is assigned.
