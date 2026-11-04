# Request envelope

**Status:** agreed, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Core [requests](../TERMS.md#request) originate at the [monitor](../../../GLOSSARY.md). Only negotiated [extension](../../../GLOSSARY.md) requests can
originate at the [helper](../../../GLOSSARY.md).

## Required fields

| Field | Type | Meaning |
|---|---|---|
| `kind` | String, exactly `"request"` | [Envelope](../TERMS.md#envelope) [discriminator](../TERMS.md#discriminator). |
| `id` | Positive [canonical decimal string](../TERMS.md#canonical-decimal-string) | [Request ID](../TERMS.md#request-id), increasing within its originating direction. |
| `op` | String | Operation name. |
| `args` | Object | Operation-specific arguments. |

Optional `binary.length` declares the immediately following binary [frame](../TERMS.md#frame).
Its rules and all common limits are in the [overview](../README.md#binary-payloads).
Argument defaults and validation belong to each operation's page.

## Example

The agreed [take-control](../../../GLOSSARY.md) envelope is:

```json
{"kind":"request","id":"1","op":"take_control","args":{}}
```

It is meaningful only after open accepts an [attachment](../../../GLOSSARY.md).

Exactly one [response](response.md) answers an accepted request while the channel
remains healthy. Response order need not match request order. Failure leaves
unfinished outcomes [indeterminate](../TERMS.md#indeterminate-outcome); cancellation and retry are not [baseline](../TERMS.md#baseline)
operations. [Plugins](../../../GLOSSARY.md) share the same ID bookkeeping.

Malformed envelopes and reused IDs fail the channel. A well-formed unknown
operation returns `unsupported_operation`. Unknown optional fields follow the
[common compatibility rules](../README.md#compatibility-errors-and-limits).

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
