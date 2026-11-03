# Hello request

**Status:** agreed behavior; complete schema pending; not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The monitor sends `op: "hello"` first on a new helper channel. It establishes
baseline protocol 1 and helper support before daemon attachment.

Use the [request envelope](request.md). No binary payload is specified for hello.
The helper response must report its intended session ID, exact packaged release,
baseline, and capabilities. Monitor extension support participates in negotiation.
The monitor compares identity with its selected session.

Saying hello does not connect an attachment, take control, or establish
readiness. Only [open](request-open.md) proceeds to daemon checks and attachment.
Missing required support or identity mismatch must fail before takeover.

## Unresolved schema

Tracked by [schema completion](https://github.com/jackgeek/climon2/issues/71).

The resolution does not assign every `args`/`result` field name, capability-list
shape, or identity-error code. It therefore does not supply a complete valid
hello wire example. Do not send an empty argument object or invent those fields
from this page. The operation name and shared envelope are agreed.

Startup deadlines and numerical limits remain with
[operational limits](https://github.com/jackgeek/climon2/issues/69).
Unknown optional fields and malformed traffic follow
[common validation](../README.md#compatibility-errors-and-limits).

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
