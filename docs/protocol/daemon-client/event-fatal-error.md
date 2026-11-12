# Fatal error — `0x4d`

**Status: agreed and implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [session daemon](../../../GLOSSARY.md#sessions) may report a connection-fatal
failure to the [client](../../../GLOSSARY.md#sessions).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One bounded [JSON](../TERMS.md#json) [frame](../TERMS.md#frame), required
`code` nonempty stable string and `message` diagnostic string:

```json
{"code":"invalid_request_id","message":"Request ID did not increase"}
```

## Delivery and meaning

Core support includes this event. Stable fatal codes are
`invalid_frame_length`, `frame_too_large`, `truncated_frame`, `invalid_json`,
`invalid_record`, `invalid_request_id`, `unknown_message_type`,
`wrong_direction`, `internal_error`; meanings are in the
[shared error catalog](core-v1.md#errors-and-bounded-delivery).
No [request ID](../TERMS.md#request-id), optional fields, defaults or unknown members exist.
Unknown error codes remain explicit failures, not retry instructions.
Text is diagnostic metadata, never terminal output.

Delivery is **best effort**, followed by connection failure; a broken stream
cannot guarantee its own final error. Absence of this event proves neither
success nor rejection. It does not confirm command completion, accepted Leave,
Kill cleanup, [monitor](../../../GLOSSARY.md#sessions) cleanup or thread join.

Malformed/wrong-direction records remain fatal; they are not plugin extensions.
Queue exhaustion must preserve required records or explicitly fail the affected
connection under #69 policy, without requiring successful fatal-event delivery.
Other connections/control must progress independently.
Related: [Response](response.md), [Command final](event-command-final.md).

## Implemented behavior

The [session daemon](../../../GLOSSARY.md#sessions) sends this event for
every connection-fatal failure on a revision-1 connection, with a fixed
diagnostic message; the code says what failed. It replaces any output not
yet started on the stream, then closes the connection. The daemon decides
`frame_too_large` and `invalid_frame_length` from the length prefix before
any body arrives, and `truncated_frame` when the stream ends inside a frame; a
stream that ends between frames closes without this event. The first frame
after hello is judged as revision 1 only once its type selects revision 1, as
the [post-hello boundary](post-hello.md#choosing-a-protocol) explains.
