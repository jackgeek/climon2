# Hello request

**Status:** agreed behavior; complete [schema](../TERMS.md#schema) pending; not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [monitor](../../../GLOSSARY.md) sends `op: "hello"` first on a new [helper](../../../GLOSSARY.md) channel. It establishes
[baseline](../TERMS.md#baseline) protocol 1 and helper support before [daemon](../../../GLOSSARY.md) [attachment](../../../GLOSSARY.md).

Use the [request envelope](request.md). No binary [payload](../TERMS.md#payload) is specified for hello.
The helper [response](../TERMS.md#response) must report its intended [session ID](../../../GLOSSARY.md), exact packaged release,
baseline, and [capabilities](../../../GLOSSARY.md). Monitor [extension](../../../GLOSSARY.md) support participates in negotiation.
The monitor compares identity with its selected [session](../../../GLOSSARY.md).

Saying hello does not connect an attachment, [take control](../../../GLOSSARY.md), or establish
[readiness](../../../GLOSSARY.md). Only [open](request-open.md) proceeds to daemon checks and attachment.
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
