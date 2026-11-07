# Kill — `0x38`

**Status: agreed, not implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [client](../../../GLOSSARY.md#sessions) sends a standalone first-role
[kill](../../../GLOSSARY.md#attachment-and-control) request to the
[session daemon](../../../GLOSSARY.md#sessions) after hello.
It is not an [attachment](../../../GLOSSARY.md#attachment-and-control)
and cannot repurpose an opened role.
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame), all fields required:
`request_id: Counter`, `revision` numeric unsigned integer 1,
`required_capabilities` a unique case-sensitive
[capability](../../../GLOSSARY.md#sessions) string array.

```json
{"request_id":"1","revision":1,"required_capabilities":[]}
```

## Acceptance and confirmation

Validate revision and required families before effects; core support is
implicit. Unsupported initial role rejects without effects and ends the
attempted role stream. A rejection may carry a `known_final` result.
Committed termination overrides succession; repeated standalone Kill joins
termination without restarting [hang-up grace](../../../GLOSSARY.md#attachment-and-control).
New interactive work is rejected; Leave remains possible.

[Response](response.md) result is
`{"kind":"termination","command_result":<CommandResult or null>,"cleanup":<enum>}`.
Cleanup is `complete`, `incomplete`, or `unknown`. Success requires a nonnull
confirmed command result and complete **daemon** cleanup. Command exit zero
alone is not confirmation; nonzero exit/signal does not itself make Kill fail.
[Monitor](../../../GLOSSARY.md#sessions) cleanup or thread join is never proved.
A failed/[indeterminate](../TERMS.md#indeterminate-outcome) response may expose
known result and incomplete/unknown
cleanup, never successful confirmation.

Revision/capability/state/resource/I/O/cleanup/internal failures use
[explicit errors](core-v1.md#errors-and-bounded-delivery).
Loss is not proof of rejection or completion. No optional fields/defaults or
unknown members; malformed records/IDs are fatal.
[#61](https://github.com/jackgeek/climon2/issues/61) owns cleanup/lifecycle policy;
#69 owns deadlines/bounds. Related: [Command final](event-command-final.md).
