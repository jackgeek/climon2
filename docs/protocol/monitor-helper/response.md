# Response envelope

**Status:** agreed, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

[Responses](../TERMS.md#response) must travel opposite their [requests](request.md) and report one
explicit outcome, including for [plugin](../../../GLOSSARY.md)-injected work.

## Required fields

| Field | Type | Meaning |
|---|---|---|
| `kind` | String, exactly `"response"` | [Envelope](../TERMS.md#envelope) [discriminator](../TERMS.md#discriminator). |
| `id` | String | Echo of the outstanding [request ID](../TERMS.md#request-id). |
| `ok` | Boolean | Whether the request succeeded. |
| `result` | Object, success only | Operation-specific result. |
| `error` | Object, failure only | Required `code` and `message` strings. |

Do not combine success and failure variants. Optional binary data must be
declared and paired as described in the [overview](../README.md#binary-payloads).
[Extension](../../../GLOSSARY.md)-owned nested [payloads](../TERMS.md#payload) may be any [JSON](../TERMS.md#json) value their [schema](../TERMS.md#schema) permits.

## Examples

```json
{"kind":"response","id":"1","ok":true,"result":{}}
```

```json
{"kind":"response","id":"2","ok":false,"error":{"code":"not_ready","message":"restoration has not completed"}}
```

These demonstrate the shared envelope, not every operation's result schema.
Operation pages explain what success proves. Input acceptance is not application
consumption; [takeover](../../../GLOSSARY.md) acceptance is not [readiness](../../../GLOSSARY.md); [detach](../../../GLOSSARY.md) acceptance is not
clean [helper](../../../GLOSSARY.md) exit.

Unknown or already-completed response IDs are invalid. A broken channel does
not produce implicit success. Plugins cannot silently suppress a response after
execution. Diagnostic prose is not a substitute for stable error codes.

**Pending:** complete per-operation result schemas and most exact error-code
mappings are not assigned by the
[helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
Do not infer new code spellings from prose. Numerical limits remain
[unresolved](https://github.com/jackgeek/climon2/issues/69).
Exact helper results and error mappings are tracked by
[schema completion](https://github.com/jackgeek/climon2/issues/71).
