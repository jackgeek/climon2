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

Background connections must receive negotiated [plugin](../../../GLOSSARY.md) data without taking
control. Current ordinary attach immediately requests control; it is not the
passive connection mechanism. Initial selection or explicit user actions acquire
control separately. Open success alone is not [readiness](../../../GLOSSARY.md).

Opening also establishes [current title state and ongoing changes](event-title.md)
without a subscription gap, independently of control and readiness. Exact
title support discovery and compatibility behavior remain unresolved.

The [ownership decision](https://github.com/jackgeek/climon2/issues/58) owns these
requirements. The [daemon contract](https://github.com/jackgeek/climon2/issues/67)
must define passive connection/attachment and negotiation semantics.
The [communication decision](https://github.com/jackgeek/climon2/issues/66) and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must assign
arguments, outcomes, errors, and examples. No open function or complete call
example is currently assigned.
