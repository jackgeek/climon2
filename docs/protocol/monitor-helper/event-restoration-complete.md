# Restoration-completion event

**Status:** agreed meaning; wire name and payload schema pending; not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The helper reports completion of an identified restoration after its ordered
updates. Use the [event envelope](event.md), identifying the control period and
the same restoration ID as [start](event-restoration-start.md).

Receiving completion is not enough. The monitor must apply the complete current
restoration before enabling application input. A successful takeover or open
response cannot substitute for this barrier.

Revocation or a newer restoration invalidates the earlier readiness state.
Do not apply completion for an obsolete restoration to a new one.

## Schema pending

Helper-specific fields are tracked by
[schema completion](https://github.com/jackgeek/climon2/issues/71).

The exact event name, identity fields, and terminal application rules are not
fully assigned; a complete valid example is unavailable.
[Terminal integration](https://github.com/jackgeek/climon2/issues/63) owns
application and chunk-completion details.
[Operational limits](https://github.com/jackgeek/climon2/issues/69) owns remaining
deadlines and transfer budgets.

Impossible state transitions fail the channel under
[common validation](../README.md#compatibility-errors-and-limits).

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
Related: [input](request-input.md) and [control](event-control.md).
