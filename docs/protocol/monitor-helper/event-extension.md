# Extension event

**Status:** agreed envelope and relay behavior; extension semantics require
negotiation; not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Either endpoint may emit an event for a negotiated extension. The helper
relays it without interpreting its plugin-owned payload as a core command.

Use the [event envelope](event.md) with `event: "extension"`.

| Required `data` field | Meaning |
|---|---|
| `name` | Namespaced extension name. |
| `revision` | Explicit negotiated revision. |
| `event` | Plugin-defined event name. |
| `payload` | Plugin-owned JSON value. |

```json
{"kind":"event","event":"extension","data":{"name":"org.example.attention","revision":1,"event":"status_changed","payload":{"status":"waiting_for_user"}}}
```

This illustrates a valid envelope for a hypothetical negotiated attention
extension, not a released plugin or a frozen status vocabulary.
An extension may define an adjacent binary payload using
[shared framing](../README.md#binary-payloads).

Events do not receive request responses. Unknown optional fields are preserved
by the relay; an unnegotiated extension event is a protocol violation.
Optional events may be filtered only when the extension permits it.
Status notification while revoked must never imply core read-only mirroring
or permission to send input or resize.

Naming grammar, revision bounds, and extension-specific schemas remain pending.
Common helper extension validation is tracked by
[schema completion](https://github.com/jackgeek/climon2/issues/71);
each plugin owns its payload schema.
All core validation and resource limits still apply; numerical budgets remain
with [operational limits](https://github.com/jackgeek/climon2/issues/69).

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
Related: [extension requests](request-extension.md) and
[pending daemon mapping](../daemon-client/post-hello.md).
