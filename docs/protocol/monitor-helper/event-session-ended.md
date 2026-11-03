# Session-end event

**Status:** agreed meaning; wire name and payload schema pending; not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The helper reports command/session completion explicitly through the
[event envelope](event.md). Terminal output is not an exit-status protocol.

Session end, deliberate detach, and helper process exit are different events.
Do not interpret missing output, EOF, or a crashed helper as an accepted
session-end result. Unfinished request outcomes remain indeterminate on failure.

## Schema pending

Helper-specific fields are tracked by
[schema completion](https://github.com/jackgeek/climon2/issues/71).

The wire name, termination/status representation, ordering relative to pending
requests, and full payload example are not assigned.
[Lifecycle](https://github.com/jackgeek/climon2/issues/61) owns session-survival,
recovery, and user-visible behavior; the helper resolution supplies the common
envelope, not these missing fields.

Use [shared framing and failure rules](../README.md#compatibility-errors-and-limits).
Numerical deadlines remain with
[operational limits](https://github.com/jackgeek/climon2/issues/69).

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
Related: [detach](request-detach.md) and [helper outcome](event-outcome.md).
