# Report pending monitor attention

**Agreed, unimplemented**, from [the selector decision](https://github.com/jackgeek/climon2/issues/60).
The [monitor](../../../GLOSSARY.md#sessions) sends this
[OSC](../TERMS.md#osc) 7501 report to its
[host](../../../GLOSSARY.md#sessions) when any currently connected
[session](../../../GLOSSARY.md#sessions) reports `needs_attention`.
Use the [shared host contract](../README.md#monitor-to-host-status-and-bell),
[domain glossary](../../../GLOSSARY.md) and [technical terms](../TERMS.md).

| Field | Required value |
|---|---|
| `state` | String `blocked` |
| `kind` | String `question` |
| `app` | String `climon2-monitor` |
| `id`, `msg`, `title`, `progress` | Omitted |

Send as the initial report if attention is pending, or once when aggregate
state changes from no pending attention. Toast dismissal/expiry, animation
frames and changes between eligible sessions do not resend it.
Acknowledged, final and stale disconnected reports do not qualify.
This is monitor-level state, not an exported session record or session title.

Exact bytes:

```rust
b"\x1b]7501;state=blocked:kind=question:app=climon2-monitor\x1b\\"
```

No acknowledgement is expected. Successful writing proves neither host support,
audible notification nor user attention. No session acknowledgement, acquisition
or daemon [FSM](../TERMS.md#fsm) change follows. Optional host support, limits and output-failure
ownership follow the shared contract.
When no eligible attention remains, send [monitoring](report-monitoring.md);
on orderly exit, send [clear](report-clear.md). The [bell](bell.md) is separate.
