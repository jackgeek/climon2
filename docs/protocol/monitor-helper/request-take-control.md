# Take-control request

**Status:** agreed behavior and request example; complete response pending;
not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The monitor sends `op: "take_control"` after open accepts its attachment.

```json
{"kind":"request","id":"1","op":"take_control","args":{}}
```

There are no required arguments or declared binary bytes in the agreed example.
Use the [request envelope](request.md) and its increasing ID rules.

Success means the daemon accepted the takeover request. It does not mean that
restoration is complete. Wait for [control](event-control.md), restoration, and
monitor-side application before enabling input. Last-request-wins semantics
remain in force.

Initial selection, selector confirmation, and explicit takeover can acquire
control when needed. Discovery, browsing/cancelling the selector, and background
maintenance do not request takeover. Confirming a revoked selected session can
reclaim control; confirming an already-controlled session retains it.

Complete result/error schemas and numerical request limits are pending.
The missing helper fields are tracked by
[schema completion](https://github.com/jackgeek/climon2/issues/71).
Use the [common compatibility rules](../README.md#compatibility-errors-and-limits);
unknown optional arguments do not redefine takeover semantics.

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936)
and [ownership decision](https://github.com/jackgeek/climon2/issues/58#issuecomment-6597610447).
