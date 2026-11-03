# Input request

**Status:** agreed behavior; argument/result schemas pending; not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The monitor sends `op: "input"` through the [request envelope](request.md) only
for its selected, controlling, ready session.

Application bytes use a declared binary payload. For example, `binary.length: 2`
followed by body bytes `00 ff` preserves those exact bytes. This is a payload
example, not a complete input request: the period argument's field schema is
still unspecified.

Input must identify the current control period. The helper rejects stale,
revoked, or not-ready requests. The monitor also waits until it has applied the
complete current restoration. No application input is queued for a future
controlling session or sent to a background helper.

Success means accepted for forwarding, not consumed by the application.
Helper mode never interprets these bytes as client shortcuts. Literal prefix+d,
take-control bytes, and pasted shortcut text remain application data.
Explicit [take control](request-take-control.md) is a separate request.

## Unresolved schema and limits

Helper-specific fields are tracked by
[schema completion](https://github.com/jackgeek/climon2/issues/71).

The exact period field, result shape, and most rejection-code mappings remain
unstated in the [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
`not_ready` is an agreed shared error example, not a complete error catalog.
No complete valid input-request example is available yet.

The binary body maximum is 1,048,575 bytes per paired frame. Input decoding and
non-key event representation belong to
[terminal integration](https://github.com/jackgeek/climon2/issues/63);
queue/deadline limits belong to
[operational limits](https://github.com/jackgeek/climon2/issues/69).
See [common framing and validation](../README.md).
