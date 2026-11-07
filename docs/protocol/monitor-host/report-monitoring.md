# Report continued monitoring without pending attention

**Agreed, unimplemented**, from [the selector decision](https://github.com/jackgeek/climon2/issues/60).
The [monitor](../../../GLOSSARY.md#sessions) sends this
[OSC](../TERMS.md#osc) 7501 report to its
[host](../../../GLOSSARY.md#sessions) when no currently connected
[session](../../../GLOSSARY.md#sessions) reports `needs_attention`.
Use the [shared host contract](../README.md#monitor-to-host-status-and-bell),
[domain glossary](../../../GLOSSARY.md) and [technical terms](../TERMS.md).

| Field | Required value |
|---|---|
| `state` | String `working` |
| `app` | String `climon2-monitor` |
| `id`, `kind`, `msg`, `title`, `progress` | Omitted |

Send initially when no attention is pending and once when aggregate pending
attention ends. `working` describes continuing monitoring, not any individual
session's activity. Replacing the complete record removes an earlier `kind`.
Do not report `idle`: an enclosing climon2 daemon interprets root idle reports
as new attention requests under its existing
[status mechanics](../../session-status.md#explicit-program-status-and-fallback).

Exact bytes:

```rust
b"\x1b]7501;state=working:app=climon2-monitor\x1b\\"
```

No acknowledgement is expected. Writing does not prove host support or clear
an enclosing daemon's attention state. There is no heartbeat, timer refresh,
session acknowledgement or application-input injection.
Optional support, limits and output-failure ownership follow the shared contract.
See [pending attention](report-attention.md) and [exit clear](report-clear.md).
