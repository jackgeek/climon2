# Progress changed — `0x45`

**Status: agreed, not implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [session daemon](../../../GLOSSARY.md#sessions) supplies retained normalized
progress changes to a subscribed [client](../../../GLOSSARY.md#sessions).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame) with sole required
`progress: Progress`. Both nested fields are required.

```json
{"progress":{"state":"error","percent":null}}
```

State is `none`, `normal`, `indeterminate`, `error`, or `paused`.
Percent is integer 0..100 or explicit null. `normal` requires a number;
`none`/`indeterminate` require null; `error`/`paused` allow either.
There are no defaults/unknown fields.

## Source, order and guarantees

Requires selected `progress-v1`. [Initial metadata](event-initial-metadata.md)
reports the current value, then ordered changes follow without a gap while
passive/background/revoked or not ready. Only current normalized progress is
retained, not a history or decoder cache.

Current source is [OSC](../TERMS.md#osc) 9;4 with the exact
[normalization and primary references](core-v1.md#source-normalization):
clear resets numeric cache, `indeterminate` ignores numbers and preserves cache,
error without number is unknown, paused without number uses cached or unknown.
Invalid/out-of-range application sequences leave the indicator unchanged.
Unchanged normalized values emit no event; 100 is not completion.

Progress does not request [attention](../../../GLOSSARY.md#status-and-attention),
change detector mechanics, [grant](../../../GLOSSARY.md#attachment-and-control) control
or prove a command result. Error progress is not final failed status.
No raw sequence is forwarded for terminal interpretation.
Invalid wire records or unselected families are not ignored application
sequences: apply [fatal validation](core-v1.md#framing-and-validation).
Required delivery is preserved or explicitly failed; numeric bounds #69.
Related: [Command final](event-command-final.md).
