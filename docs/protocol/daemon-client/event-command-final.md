# Command final — `0x4c`

**Status: agreed, not implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [session daemon](../../../GLOSSARY.md#sessions) reports an authoritative
final command result to the [client](../../../GLOSSARY.md#sessions).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame), both fields required:
`command_result: CommandResult`, `status: Status or null`.
Status is terminal Status if `status-v1` is selected, otherwise explicit null.

```json
{"command_result":{"kind":"signal","number":15,"name":"SIGTERM"},"status":{"state":"failed","changed_at":"2026-11-06T12:00:02.000Z","episode_id":null}}
```

[CommandResult](core-v1.md#shared-types) distinguishes normal unsigned 32-bit
exit codes from source-host signal number and nullable name. Exit 143 is not
signal 15. Zero normal exit means completed; all other normal exits and signals
mean failed.

## Order and guarantees

Core support supplies the result independently of optional status support.
The final transition is here, not duplicated in
[Status changed](event-status-changed.md). Final identity is session
[GUID](../TERMS.md#guid) plus
the fixed category `final outcome`, not exit-versus-signal kind. There is one
final result per session, no second final counter; inconsistent repeats cannot
authorize another toast. Terminal status/result must agree: completed is normal
exit zero; failed is nonzero exit or signal.
No later event revives terminal activity/
[attention](../../../GLOSSARY.md#status-and-attention).
Confirmed completion stops reconnect; [EOF](../TERMS.md#eof)/loss alone is not final.

This event is not Kill cleanup confirmation, deliberate detach,
[monitor](../../../GLOSSARY.md#sessions) cleanup
or thread join. [Kill](request-kill.md) success separately requires confirmed
result **and complete daemon cleanup**.
Final first learned while locally [viewing](../../../GLOSSARY.md#status-and-attention)
stays toast-suppressed forever in
that monitor; first learned unviewed notifies once. Another monitor's viewing
does not suppress it; see [local rules](core-v1.md#viewing-acknowledgement-and-local-notifications).

No [request ID](../TERMS.md#request-id)/defaults/unknown members. Invalid records follow
[fatal validation](core-v1.md#framing-and-validation).
Required final delivery cannot be silently lost or fabricated on broken
transport. The [agreed lifecycle](../../monitor-lifecycle.md) permits definitive
completion to satisfy confirmed selector removal intent, but still requires
local delivery/handle release, instance cleanup and thread join before run-local
dismissal. A final event never proves successful Leave. #69 owns numerical
bounds/deadlines; native declarations #71 and final acceptance #62 remain open.
