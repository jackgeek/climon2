# Extension request

**Status:** future relay requirements only;
[plugin](../../../GLOSSARY.md#protocol-and-plugin-concepts) [APIs](../TERMS.md#api), configuration,
implementation, and examples are deferred, not release prerequisites.
Revision 1 deliberately publishes no extension [schemas](../TERMS.md#schema).
The [native compatibility rules](core-v1.md#representation-and-compatibility)
require a separately specified revision for these future calls.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Either endpoint may request negotiated [plugin](../../../GLOSSARY.md) work
through the [client](../../../GLOSSARY.md). Identify the
[extension](../../../GLOSSARY.md) by [namespaced name](../TERMS.md#namespaced-name)
and revision. Preserve plugin-owned [payloads](../TERMS.md#payload) without
interpreting them as core commands or loading plugins in the client.

Relay must work for passive, background, and revoked clients, independent of
control. Negotiate effective [monitor](../../../GLOSSARY.md#sessions),
[daemon](../../../GLOSSARY.md#sessions), and client-relay support before
using an extension. Missing required support must fail before takeover.
No extension may bypass core validation, control, [readiness](../../../GLOSSARY.md), or resource rules.

Injected work needs its own correlation and explicit outcome. Removing work
before execution requires rejection; replacement work must account truthfully
for the original outcome. Retry safety is not implied.

The agreed [communication contract](../README.md#shared-in-process-communication-contract)
and its [core semantic examples](../README.md#semantic-examples), owned by
[#66](https://github.com/jackgeek/climon2/issues/66), preserve these future relay
guarantees without assigning a plugin API. Unknown core tags are invalid, not
implicit extensions. Future negotiation must use an explicit name/revision
intersection including relay support; binary payloads preserve arbitrary [bytes](../TERMS.md#byte).
Exact daemon support remains with
[#67](https://github.com/jackgeek/climon2/issues/67), and core declarations with
[#71](https://github.com/jackgeek/climon2/issues/71). Neither publishes plugin
schemas or examples here. Plugin payload schemas belong to their extensions,
not this map. See [extension events](event-extension.md).
