# Response — `0x40`

**Status: agreed; implemented for the results of Open, Input, Resize,
Acquire, Ready, Leave, Kill and Focus and for every rejection the
[session daemon](../../../GLOSSARY.md#sessions) sends**, owned by [#67](https://github.com/jackgeek/climon2/issues/67);
the Set viewing and Acknowledge results arrive with their operations.
The daemon's only `failed` outcome is a [Kill](request-kill.md#implemented-behavior)
whose termination or cleanup did not complete, and it sends no
`indeterminate` outcome. A rejected first role carries `known_final` once the
command's result is final, as
[Command final](event-command-final.md#implemented-behavior) describes.
The [session daemon](../../../GLOSSARY.md#sessions) answers a
[client](../../../GLOSSARY.md#sessions) request.
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One bounded [JSON](../TERMS.md#json) [frame](../TERMS.md#frame).
All fields required:

| Field | Type/value |
|---|---|
| `request_id` | [Counter](core-v1.md#shared-types) matching the received request. |
| `outcome` | `ok`, `rejected`, `failed`, `indeterminate`. |
| `result` | Matching result variant below, or null on non-success. |
| `error` | Error on every non-success; null on success. |

```json
{"request_id":"7","outcome":"ok","result":{"kind":"acknowledgement","changed":false},"error":null}
```

`ok` requires nonnull matching result and null error. Non-success may expose
known result instead of hiding partial effects. `rejected` guarantees no
execution of that operation; `failed` does not. Lost responses are not
rejections and do not authorize retry. Unknown error codes remain non-successes.

## Exact result variants

Every member listed is required, with no extras or defaults.
Shared types are in [core wire types](core-v1.md#shared-types).

| Owning operation | Exact success result members |
|---|---|
| [Open](request-open.md) | `kind:"opened"`, `connection_id:Guid`, `selected_capabilities:string[]`. |
| [Input](request-input.md) | `kind:"input_accepted"`, `accepted_bytes:positive u32` equal to full payload. |
| [Resize](request-resize.md) | `kind:"resize_accepted"`. |
| [Acquire](request-acquire.md) | `kind:"acquired"`, `changed:bool`, `period:Counter`, `restoration_id:Counter`. |
| [Ready](request-ready.md) | `kind:"ready"`, `changed:bool`, `period:Counter`, `restoration_id:Counter`. |
| [Set viewing](request-viewing.md) | `kind:"viewing"`, `changed:bool`, `viewing:bool`. |
| [Acknowledge](request-acknowledge.md) | `kind:"acknowledgement"`, `changed:bool`. |
| [Leave](request-leave.md) | `kind:"left"`, `attachment_detached:bool`. |
| [Kill](request-kill.md) | `kind:"termination"`, `command_result:CommandResult`, `cleanup:"complete"`. |
| [Focus](request-focus.md) (#63 amendment) | `kind:"focus"`, `changed:bool`, `focused:bool`, `accepted_bytes:u32` (0 or 3). |

Initial role rejection may carry
`{"kind":"known_final","command_result":<CommandResult>}`.
Failed/[indeterminate](../TERMS.md#indeterminate-outcome) Kill may carry the termination shape with
`command_result:CommandResult or null` and cleanup `incomplete` or `unknown`;
this is never successful confirmation. Success requires a confirmed nonnull
result plus complete daemon cleanup.

## Order, capabilities and failure

Core family supplies responses. Responses consume no [request IDs](../TERMS.md#request-id).
Open success precedes initial metadata; later immediate causal events precede
their success response. Deferred resize acceptance proves no final geometry.
Focus distinguishes desired-state acceptance from complete non-user sequence
acceptance for forwarding, never command consumption or a
[PTY](../TERMS.md#pseudo-terminal-pty) read.
Each operation page specifies its acceptance boundary and errors; all inherit
the [operational/fatal catalog](core-v1.md#errors-and-bounded-delivery).
Malformed/unknown fields, invalid correlation and wrong direction cannot
fabricate success. Numeric bounds remain #69; no native ABI shape is assigned.
