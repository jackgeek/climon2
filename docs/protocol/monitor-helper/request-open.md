# Open request

**Status:** agreed behavior; complete schema pending; not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

After a successful [hello](request-hello.md), the monitor sends `op: "open"`
using the [request envelope](request.md).

The helper verifies daemon session/release identity and required capabilities
and extensions before attaching. A channel is permanently bound to that session
and at most one attachment. Discovery alone never takes control.

Missing optional extension support disables that extension. Missing required
support rejects open before takeover. The accepted response reports effective
negotiated support. That set stays fixed for this connection.

Attachment can grant control. Open success is not restoration completion or
permission to send [input](request-input.md).
Observe [control](event-control.md) and restoration events.

## Unresolved schema

Tracked by [schema completion](https://github.com/jackgeek/climon2/issues/71).

The required-capability/extension list fields, complete successful result, and
identity/state error-code mapping are not assigned in the helper resolution.
No complete wire example or default list is available yet. Binary data is not
specified for open.

The daemon's pre-attachment advertisement and relay mapping remain with
[the daemon contract](https://github.com/jackgeek/climon2/issues/67).
Numerical deadlines remain with
[operational limits](https://github.com/jackgeek/climon2/issues/69).
Apply the shared [validation rules](../README.md#compatibility-errors-and-limits).

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
