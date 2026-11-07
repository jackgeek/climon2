# Clear monitor program-status advertising on exit

**Agreed, unimplemented**, from [the selector decision](https://github.com/jackgeek/climon2/issues/60).
The [monitor](../../../GLOSSARY.md#sessions) sends this
[OSC](../TERMS.md#osc) 7501 report to its
[host](../../../GLOSSARY.md#sessions) during orderly terminal cleanup.
Use the [shared host contract](../README.md#monitor-to-host-status-and-bell),
[domain glossary](../../../GLOSSARY.md) and [technical terms](../TERMS.md).

| Field | Required value |
|---|---|
| `state` | String `clear` |
| All other fields | Omitted |

Exact bytes:

```rust
b"\x1b]7501;state=clear\x1b\\"
```

This is root clearing: upstream rules remove the addressed root and every
child record on that host terminal. It is not scoped by `app`, does not detach
or terminate any session, and does not acknowledge pending session attention.
An enclosing climon2 daemon follows its existing root-clear fallback rules,
which do not clear its attention [FSM](../TERMS.md#fsm).
No acknowledgement or retry/replay policy is introduced. Unwritable/lost hosts
and abnormal cleanup remain with
[lifecycle](https://github.com/jackgeek/climon2/issues/61); no successful cleanup
or session-survival guarantee is inferred from sending this sequence.
See [attention](report-attention.md) and [monitoring](report-monitoring.md).
