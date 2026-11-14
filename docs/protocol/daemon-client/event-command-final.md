# Command final — `0x4c`

**Status: agreed and implemented**
([#88](https://github.com/jackgeek/climon2/issues/88), with terminal status
for `status-v1` from [#91](https://github.com/jackgeek/climon2/issues/91)),
owned by [#67](https://github.com/jackgeek/climon2/issues/67).
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
Command final means that no more of the command's output follows: the daemon
sends it only after the command's terminal output has been drained to the
connections, so every [Display](event-display.md) and
[Title changed](event-title-changed.md) that the command's output produced
for a connection precedes it (owner decision on
[#88](https://github.com/jackgeek/climon2/issues/88), 2026-11-14).
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

The [session daemon](../../../GLOSSARY.md#sessions) sends Command final once
the command's result is final, which it is when no more of the command's
output can follow:

- When the command exits by itself, the daemon reaps it, then drains the
  output still buffered in its terminal. The drain ends at the first of: the
  end of the terminal's output (every holder of the terminal closed it),
  100 ms without output, or 1 s after the reap, even while a background job
  that holds the terminal still writes. The daemon reads nothing more after
  the drain. It sends Command final in the same step that delivered the
  drained output, after that output's Display and Title changed records, so
  at most about 1 s after the reap.
- A [Kill](request-kill.md), a final unexpected departure, a failure (such
  as a failed read of the command's terminal) or the initial-attachment
  deadline that comes after the command exited does not cut that drain
  short: the daemon finishes the drain, delivers the drained output and
  sends Command final as above, and only then begins the
  [hang-up grace](../../../GLOSSARY.md#attachment-and-control) of the
  session's termination (owner decision on
  [#88](https://github.com/jackgeek/climon2/issues/88), 2026-11-14).
- When the hang-up grace begins while the command still runs, the daemon
  stops reading the command's terminal as the grace begins. It sends
  Command final when it confirms the end of the grace, 5 s later, or when it
  cannot confirm it but had already reaped the command.

Every connection that opened then
receives it once, each
[passive monitor connection](../../../GLOSSARY.md#attachment-and-control) and
interactive [attachment](../../../GLOSSARY.md#attachment-and-control) alike;
a standalone Kill's connection gets the result in its answer instead.
A connection that selected `status-v1` receives the final
[session status](../../../GLOSSARY.md#status-and-attention) in `status`:
`completed` for exit zero, `failed` otherwise, with a null episode and the
daemon's time of the result as its change time; others receive a null
`status`. No [Status changed](event-status-changed.md) carries that final
transition, and none follows it: the result stops activity and attention
observation, so the drained last output, a later deadline, Set viewing or
Acknowledge changes nothing. A normal exit keeps its raw status and a signal its
number, with a null `name`: exit 143 and signal 15 stay apart. A later Open
gets the result in its [Initial metadata](event-initial-metadata.md), and no
Command final follows it; learning the same result again sends nothing.
After a command exits by itself, the session goes on after Command final
until its [controller](../../../GLOSSARY.md#attachment-and-control) confirmed
[Ready](request-ready.md) and the initial attachment had its chance: until
then an interactive Open still attaches and [Acquire](request-acquire.md)
is still granted.

Command final does not wait for
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts). A
controller receives the command's live output as Display before it, then,
when the session ends, the
[release](../../../GLOSSARY.md#restoration-and-presentation) of its
[presentation](../../../GLOSSARY.md#restoration-and-presentation) as Display,
[Control revoked](event-control-revoked.md) with reason `session_ending`,
and the end of its stream. The release is not command output. Not yet
covered: a [restoration](../../../GLOSSARY.md#restoration-and-presentation)
still in progress when the drain ends sends the rest of its Display, and the
live output held back behind it, after Command final, as does the update
of a controller under
[derived presentation](../../../GLOSSARY.md#restoration-and-presentation)
held back by an open [synchronized update](../TERMS.md#synchronized-update);
a grant after Command final replays the screen in its restoration. Holding
Command final until no restoration is in progress and no synchronized update
is held is follow-up
[#203](https://github.com/jackgeek/climon2/issues/203); the replay after a
later grant stays. Neither the end of the command's terminal output
while the command still runs nor the end of a stream is a result. When the
daemon never learns how the command ended, because its hang-up or the kill
of what remained of the command's terminal session failed before it reaped
the command, it sends no Command final: the streams end without one.
