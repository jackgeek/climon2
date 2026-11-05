# Session end

**Status:** planned semantics; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [client](../../../GLOSSARY.md) reports command/[session](../../../GLOSSARY.md)
completion explicitly. Terminal output is not an exit-status interface.
Session end, accepted [detach](../../../GLOSSARY.md), and client thread
completion are distinct observations. Missing output or a client failure
does not prove an accepted session-end result.

A confirmed zero exit produces `completed`; confirmed nonzero exit or signal
termination produces `failed` in the
[daemon attention FSM](../../session-status.md#attention). Preserve that known
final status and retained terminal after connection closure; stop reconnecting.
Unconfirmed loss remains disconnected/uncertain, not a fabricated final result.
Status notification does not prove client cleanup or a completed thread join.

[Lifecycle](https://github.com/jackgeek/climon2/issues/61) owns survival, recovery,
and visible behavior. [Communication](https://github.com/jackgeek/climon2/issues/66)
and [interface schemas](https://github.com/jackgeek/climon2/issues/71) must specify
termination/status types, pending-request ordering, explicit errors, and valid
examples. On communication failure unfinished outcomes remain
[indeterminate](../TERMS.md#indeterminate-outcome).

See [client outcome](event-outcome.md) for client completion and cleanup, which
do not themselves establish session end.
