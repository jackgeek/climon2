# Session end

**Status:** agreed communication requirements; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [client](../../../GLOSSARY.md) reports command/[session](../../../GLOSSARY.md)
completion explicitly. Terminal output is not an exit-status interface.
Session end, accepted [detach](../../../GLOSSARY.md), and client thread
completion are distinct observations. Missing output or a client failure
does not prove an accepted session-end result.

A confirmed zero exit produces `completed`; confirmed nonzero exit or signal
termination produces `failed` in the
[daemon](../../../GLOSSARY.md#sessions)
[attention](../../../GLOSSARY.md#status-and-attention)
[FSM](../TERMS.md#fsm). Preserve that known
final status and retained terminal after connection closure; stop reconnecting.
Unconfirmed loss remains disconnected/uncertain, not a fabricated final result.
Status notification does not prove client cleanup or a completed thread join.
See the [final-status rules](../../session-status.md#attention).

[Lifecycle](https://github.com/jackgeek/climon2/issues/61) owns survival, recovery,
and visible behavior.
The [shared communication contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples), agreed in
[#66](https://github.com/jackgeek/climon2/issues/66), require reliable final-outcome
identity and causal ordering with [responses](../TERMS.md#response) and other
[events](../TERMS.md#event). Closure/failure
reason remains observable outside ordinary queue capacity, with queued records
ahead of the terminal indication. A recoverable daemon connection loss is not
instance-stream closure or command completion.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) and the
[daemon contract](https://github.com/jackgeek/climon2/issues/67) must specify
exact termination/status types, identities, error codes, and executable examples.
On communication failure unfinished outcomes remain
[indeterminate](../TERMS.md#indeterminate-outcome).

See [client outcome](event-outcome.md) for client completion and cleanup, which
do not themselves establish session end.
