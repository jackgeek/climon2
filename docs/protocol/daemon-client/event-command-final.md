# Command final — `0x4c`

**Status: agreed and implemented** without terminal status
([#88](https://github.com/jackgeek/climon2/issues/88)), owned by [#67](https://github.com/jackgeek/climon2/issues/67);
status arrives with `status-v1` ([#91](https://github.com/jackgeek/climon2/issues/91)).
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
bounds/deadlines; [native declarations](../monitor-client/core-v1.md) are agreed
in #71; the [acceptance matrix](../../monitor-acceptance.md) is agreed in #62,
not executed conformance evidence.

## Implemented behavior

The [session daemon](../../../GLOSSARY.md#sessions) sends Command final as
soon as it first knows how the command ended: when it reaps a command that
exited by itself, or when it confirms the end of a
[hang-up grace](../../../GLOSSARY.md#attachment-and-control) that a
[Kill](request-kill.md), a final unexpected departure, a failure or the
initial-attachment deadline began. Every connection that opened then
receives it once, each
[passive monitor connection](../../../GLOSSARY.md#attachment-and-control) and
interactive [attachment](../../../GLOSSARY.md#attachment-and-control) alike;
a standalone Kill's connection gets the result in its answer instead.
`status` is null, since no connection can select `status-v1` yet, and no
Status changed is sent. A normal exit keeps its raw status and a signal its
number, with a null `name`: exit 143 and signal 15 stay apart. A later Open
gets the result in its [Initial metadata](event-initial-metadata.md), and no
Command final follows it; learning the same result again sends nothing.

Command final is metadata, sent whatever the
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts): the
[controller](../../../GLOSSARY.md#attachment-and-control) may still receive
the command's last output as [Display](event-display.md) after it, then the
release of its
[presentation](../../../GLOSSARY.md#restoration-and-presentation),
[Control revoked](event-control-revoked.md) with reason `session_ending`,
and the end of its stream. When the daemon cannot confirm how the command
ended, because its hang-up or the kill of what remained of the command's
terminal session failed, it sends no Command final: the streams end
without one.
