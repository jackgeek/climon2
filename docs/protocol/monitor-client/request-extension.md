# Extension request

**Status:** required relay behavior; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Either endpoint may request negotiated [plugin](../../../GLOSSARY.md) work
through the [client](../../../GLOSSARY.md). Identify the
[extension](../../../GLOSSARY.md) by [namespaced name](../TERMS.md#namespaced-name)
and revision. Preserve plugin-owned [payloads](../TERMS.md#payload) without
interpreting them as core commands or loading plugins in the client.

Relay must work for passive, background, and revoked clients, independent of
control. Negotiate effective monitor, daemon, and client-relay support before
using an extension. Missing required support must fail before takeover.
No extension may bypass core validation, control, [readiness](../../../GLOSSARY.md), or resource rules.

Injected work needs its own correlation and explicit outcome. Removing work
before execution requires rejection; replacement work must account truthfully
for the original outcome. Retry safety is not implied.

The [communication decision](https://github.com/jackgeek/climon2/issues/66),
[daemon contract](https://github.com/jackgeek/climon2/issues/67), and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must assign
negotiation, types, encoding, errors, and valid examples. Plugin payload schemas
belong to their extensions, not this map. See [extension events](event-extension.md).
