# Open connection

**Status:** agreed communication requirements, not implemented; exact interface
and passive connection model unresolved.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [monitor](../../../GLOSSARY.md) creates a [client](../../../GLOSSARY.md)
for each discovered live [session](../../../GLOSSARY.md), newest first, with
bounded concurrent progress. Verify [daemon](../../../GLOSSARY.md#sessions) identity and required
[capabilities](../../../GLOSSARY.md)/[extensions](../../../GLOSSARY.md) before
control acquisition or operation submission. Report the support actually
available through both library and daemon. Missing required support fails
explicitly; missing optional support is unavailable/unknown, not a fabricated
empty title or cleared [attention](../../../GLOSSARY.md#status-and-attention).

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
acquisition. Unknown status remains explicit uncertainty; exact capability
tokens, daemon support discovery, and [schemas](../TERMS.md#schema) remain unresolved.

Automatic reconnect uses this passive role even for the selected session;
resuming control requires an explicit user action. Preserve the monitor's
retained terminal model without presenting it as current merely because open
succeeded. See the [recovery mechanics](../../session-status.md#connection-loss-and-retained-terminal-views).

Each connection has a distinct
[generation](../TERMS.md#connection-generation). Delimit initial core metadata
synchronization, then deliver gap-free changes. This metadata barrier does not
make the retained terminal current. Unfinished old-connection work is
[indeterminate](../TERMS.md#indeterminate-outcome); do not replay it or execute
queued authority-sensitive work on a new connection. Confirmed final outcomes
stop reconnecting. Recoverable connection loss need not close the instance's
delivery stream.

The [ownership decision](https://github.com/jackgeek/climon2/issues/58) owns these
requirements. The [daemon contract](https://github.com/jackgeek/climon2/issues/67)
must define passive connection/attachment and negotiation semantics.
The agreed [communication contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples), owned by
[#66](https://github.com/jackgeek/climon2/issues/66), apply here.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) must assign
exact arguments, outcomes, errors, and executable examples. Retry schedules,
recreation, cleanup, and budgets remain with
[lifecycle](https://github.com/jackgeek/climon2/issues/61) and
[operational limits](https://github.com/jackgeek/climon2/issues/69).
