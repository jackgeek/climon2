# Extension request

**Status:** agreed [envelope](../TERMS.md#envelope) and relay behavior; [plugin](../../../GLOSSARY.md) [payload](../TERMS.md#payload) and full
negotiation [schemas](../TERMS.md#schema) depend on the [extension](../../../GLOSSARY.md); not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Either endpoint can [request](../TERMS.md#request) plugin-defined work after negotiating support.
The [helper](../../../GLOSSARY.md) relays [daemon](../../../GLOSSARY.md)-originated requests to the [monitor](../../../GLOSSARY.md) and monitor-originated
requests to the daemon. It does not load plugins.

## Required structure

Use the [request envelope](request.md) with `op: "extension"`.

| `args` field | Meaning |
|---|---|
| `name` | [Namespaced extension name](../TERMS.md#namespaced-name); a string in the agreed example. |
| `revision` | Explicit negotiated revision; the example uses integer 1. |
| `operation` | Plugin-defined operation name. |
| `payload` | Plugin-owned [JSON](../TERMS.md#json) value, validated by the endpoint's schema. |

```json
{"kind":"request","id":"2","op":"extension","args":{"name":"org.example.attention","revision":1,"operation":"subscribe","payload":{}}}
```

This example is valid only after the named revision and its payload schema are
agreed by both endpoints. It does not implement an attention plugin.
An extension may also define an adjacent binary payload under the
[shared framing rules](../README.md#binary-payloads).

The [client](../../../GLOSSARY.md) preserves payloads and unknown optional extension fields without
interpreting them as core commands. Missing required support rejects open
before [attachment](../../../GLOSSARY.md). Unknown extension requests get `unsupported_extension`.
Support is fixed for the connection.

## Outcomes and unresolved details

Missing common extension fields/validation are tracked by
[schema completion](https://github.com/jackgeek/climon2/issues/71).
Plugin-specific payload schemas remain with their extensions.

Use the common [response envelope](response.md). Injected work gets its own [ID](../TERMS.md#request-id)
and outcome. Filtering before execution returns rejection; replacement work
must account for the original outcome. Cancellation or retry is not implied.

The name grammar/registration, revision bounds, complete negotiation fields,
and per-extension result/error schemas remain unspecified. Numerical payload,
queue, and request limits beyond framing remain with
[operational limits](https://github.com/jackgeek/climon2/issues/69).
No permissive relay may bypass core validation.

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
See also [extension events](event-extension.md) and
[pending daemon mapping](../daemon-client/post-hello.md).
