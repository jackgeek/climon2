# Open connection

**Status:** planned semantics; interface and passive connection model unresolved.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [monitor](../../../GLOSSARY.md) creates a [client](../../../GLOSSARY.md)
for each discovered live [session](../../../GLOSSARY.md), newest first, with
bounded concurrent progress. Verify daemon identity and required
[capabilities](../../../GLOSSARY.md)/[extensions](../../../GLOSSARY.md) before
control acquisition. Missing required support fails explicitly; missing optional
support disables that extension.

Background connections receive supported core title/status data without taking
control. [Plugin](../../../GLOSSARY.md) implementation is deferred.
Current ordinary attach immediately requests control; it is not the
passive connection mechanism. Initial selection or explicit user actions acquire
control separately. Open success alone is not [readiness](../../../GLOSSARY.md).

Opening establishes supported core [title state and ongoing changes](event-title.md)
and [effective session status/time](event-session-status.md), without a
subscription gap and independently of control/readiness. Plugin implementation
is deferred. Older daemons without title support remain usable with
command/arguments fallback; missing title support must not block control
acquisition. Unknown status remains explicit uncertainty; exact compatibility,
support discovery, and schemas remain unresolved.

Automatic reconnect uses this passive role even for the selected session;
resuming control requires an explicit user action. Preserve the monitor's
retained terminal model without presenting it as current merely because open
succeeded. See the [recovery mechanics](../../session-status.md#connection-loss-and-retained-terminal-views).

The [ownership decision](https://github.com/jackgeek/climon2/issues/58) owns these
requirements. The [daemon contract](https://github.com/jackgeek/climon2/issues/67)
must define passive connection/attachment and negotiation semantics.
The [communication decision](https://github.com/jackgeek/climon2/issues/66) and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must assign
arguments, outcomes, errors, and examples. No open function or complete call
example is currently assigned.
