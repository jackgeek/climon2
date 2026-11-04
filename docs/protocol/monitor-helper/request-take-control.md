# Take-control request

**Status:** agreed behavior and [request](../TERMS.md#request) example; complete [response](../TERMS.md#response) pending;
not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [monitor](../../../GLOSSARY.md) sends `op: "take_control"` after open accepts its [attachment](../../../GLOSSARY.md).

```json
{"kind":"request","id":"1","op":"take_control","args":{}}
```

There are no required arguments or declared binary [bytes](../TERMS.md#byte) in the agreed example.
Use the [request envelope](request.md) and its increasing [ID](../TERMS.md#request-id) rules.

Success means the [daemon](../../../GLOSSARY.md) accepted the [takeover](../../../GLOSSARY.md) request. It does not mean that
[restoration](../../../GLOSSARY.md) is complete. Wait for [control](event-control.md), restoration, and
monitor-side application before enabling input. Last-request-wins semantics
remain in force.

Initial selection, selector confirmation, and explicit takeover can acquire
control when needed. Discovery, browsing/cancelling the selector, and background
maintenance do not request takeover. Confirming a [revoked](../../../GLOSSARY.md) selected [session](../../../GLOSSARY.md) can
reclaim control; confirming an already-controlled session retains it.

Complete result/error [schemas](../TERMS.md#schema) and numerical request limits are pending.
The missing [helper](../../../GLOSSARY.md) fields are tracked by
[schema completion](https://github.com/jackgeek/climon2/issues/71).
Use the [common compatibility rules](../README.md#compatibility-errors-and-limits);
unknown optional arguments do not redefine takeover semantics.

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936)
and [ownership decision](https://github.com/jackgeek/climon2/issues/58#issuecomment-6597610447).
