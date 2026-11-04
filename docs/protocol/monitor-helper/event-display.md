# Display event

**Status:** agreed delivery rules; wire name and [payload](../TERMS.md#payload) [schema](../TERMS.md#schema) pending;
not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [helper](../../../GLOSSARY.md) sends ordered display updates to the [monitor](../../../GLOSSARY.md), using the
[event envelope](event.md). Updates identify their [control period](../../../GLOSSARY.md) and any
[restoration](../../../GLOSSARY.md) they belong to.

The helper maintains its screen while [controlling](../../../GLOSSARY.md), including in the background.
[Revoked](../../../GLOSSARY.md) helpers do not provide core live read-only screen mirroring. On regaining
control, restoration brings the view current.

Bulk [bytes](../TERMS.md#byte) can use the declared [binary frame](../README.md#binary-payloads).
Each binary body is at most 1,048,575 bytes. Larger transfers require ordered
bounded chunks; required updates are not silently dropped under [backpressure](../TERMS.md#backpressure).

## Schema pending

The helper envelope fields are tracked by
[schema completion](https://github.com/jackgeek/climon2/issues/71).

Whether the payload represents cells/state, encoded terminal output, or another
explicit form remains with
[terminal integration](https://github.com/jackgeek/climon2/issues/63).
The event name, exact identity/payload fields, chunk schema, and application rules
are not yet assigned. There is no complete valid display-event example.

Display content is not the control protocol: never infer [readiness](../../../GLOSSARY.md), [grants](../../../GLOSSARY.md), or
[daemon](../../../GLOSSARY.md) acknowledgements by scraping terminal bytes. [Plugins](../../../GLOSSARY.md) cannot bypass
control or query authority by hiding core commands in a binary payload.
Use [common validation and limits](../README.md#compatibility-errors-and-limits).

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
Related: [restoration completion](event-restoration-complete.md).
