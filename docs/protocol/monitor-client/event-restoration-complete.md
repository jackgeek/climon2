# Restoration completion

**Status:** agreed communication requirements; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [client](../../../GLOSSARY.md) reports completion after the ordered updates
for the [restoration](../../../GLOSSARY.md) identified at
[start](event-restoration-start.md), in the same
[control period](../../../GLOSSARY.md).

Receiving completion is not enough: the [monitor](../../../GLOSSARY.md) must
apply the complete current restoration before enabling application input.
Transactionally replace the complete model/history; apply before wire Ready,
without needing selection or paint for background models. Selected application
input additionally waits for a coherent current painted target in usable space.
There is no union/dedup with older history, which may be discarded on replacement.
See the [terminal barriers](../terminal-integration.md#host-canvas-geometry-and-coherent-readiness).
Open or [takeover](../../../GLOSSARY.md) acceptance cannot substitute for this barrier. [Revocation](../../../GLOSSARY.md) or
a newer restoration invalidates earlier [readiness](../../../GLOSSARY.md);
obsolete completion must not enable input.

[Terminal integration](../terminal-integration.md) owns agreed application
mechanics; [interface schemas](https://github.com/jackgeek/climon2/issues/71) still
owns exact native identity fields, errors, and executable examples.
The [shared contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples), agreed in
[#66](https://github.com/jackgeek/climon2/issues/66), preserve causal
[grant](../../../GLOSSARY.md#attachment-and-control)/start/update/completion order in one
[response](../TERMS.md#response)/[event](../TERMS.md#event) stream. Completion
is a receipt barrier, not the monitor's applied barrier. Neither may be reused
after a changed [connection generation](../TERMS.md#connection-generation).
Required restoration boundaries cannot be dropped or lossily coalesced.
[Operational limits](https://github.com/jackgeek/climon2/issues/69) owns remaining
deadlines and transfer budgets.

[Wire Restoration complete](../daemon-client/event-restoration-complete.md)
identifies period/restoration; matching complete sent and applied is required
before [Ready](../daemon-client/request-ready.md). A superseded restoration
gets no fabricated completion. Native declarations remain #71.
