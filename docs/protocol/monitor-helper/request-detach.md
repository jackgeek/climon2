# Detach request

**Status:** agreed behavior; complete schema pending; not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The monitor sends `op: "detach"` through the [request envelope](request.md)
to end a retained attachment deliberately.

Success is reported only after the daemon accepts detach. The helper then
completes resource cleanup and exits. Detach acceptance, a final outcome message,
and clean helper process exit are separate observations.

EOF, a crash, a timeout, or incomplete cleanup is not proof of accepted detach.
Application input stops when monitor exit starts. Losing a helper does not by
itself guarantee that its session survives.

Switching away is not detach: it keeps the helper and its background attachment.

## Unresolved schema

Helper-specific fields are tracked by
[schema completion](https://github.com/jackgeek/climon2/issues/71).

The resolution fixes the operation meaning, but not a complete argument/result
schema or all error codes. No complete valid wire example is assigned here.
No binary payload is specified for detach.

[Lifecycle](https://github.com/jackgeek/climon2/issues/61) owns session survival,
recovery, and visible exit behavior.
[Operational limits](https://github.com/jackgeek/climon2/issues/69) owns shutdown
deadlines and exhaustion handling.
Observe [helper outcomes](event-outcome.md) and
[shared failure rules](../README.md#compatibility-errors-and-limits).

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
