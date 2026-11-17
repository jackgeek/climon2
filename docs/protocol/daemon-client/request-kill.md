# Kill — `0x38`

**Status: agreed and implemented**
([#88](https://github.com/jackgeek/climon2/issues/88)), owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [client](../../../GLOSSARY.md#sessions) sends a standalone first-role
[kill](../../../GLOSSARY.md#attachment-and-control) request to the
[session daemon](../../../GLOSSARY.md#sessions) after hello.
It is not an [attachment](../../../GLOSSARY.md#attachment-and-control)
and cannot repurpose an opened role.
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame), all fields required:
`request_id: Counter`, `revision` numeric unsigned integer 1,
`required_capabilities` a unique case-sensitive
[capability](../../../GLOSSARY.md#sessions) string array.

```json
{"request_id":"1","revision":1,"required_capabilities":[]}
```

## Acceptance and confirmation

Validate revision and required families before effects; core support is
implicit. Unsupported initial role rejects without effects and ends the
attempted role stream. A rejection may carry a `known_final` result.
Committed termination overrides succession; repeated standalone Kill joins
termination without restarting [hang-up grace](../../../GLOSSARY.md#attachment-and-control).
New interactive work is rejected; Leave remains possible.

[Response](response.md) result is
`{"kind":"termination","command_result":<CommandResult or null>,"cleanup":<enum>}`.
Cleanup is `complete`, `incomplete`, or `unknown`. Success requires a nonnull
confirmed command result and complete **daemon** cleanup. Command exit zero
alone is not confirmation; nonzero exit/signal does not itself make Kill fail.
[Monitor](../../../GLOSSARY.md#sessions) cleanup or thread join is never proved.
A failed/[indeterminate](../TERMS.md#indeterminate-outcome) response may expose
known result and incomplete/unknown
cleanup, never successful confirmation.

Revision/capability/state/resource/I/O/cleanup/internal failures use
[explicit errors](core-v1.md#errors-and-bounded-delivery).
Loss is not proof of rejection or completion. No optional fields/defaults or
unknown members; malformed records/IDs are fatal.
The [agreed lifecycle](../../monitor-lifecycle.md#selector-delete-terminate-and-remove)
adds a default-Cancel selector modal pinned to immutable
[session ID](../../../GLOSSARY.md#protocol-and-plugin-concepts), not another wire
Kill. Confirmed removal waits for authoritative termination, then local
instance delivery/handle release, cleanup and thread join. Rejection or lost
response keeps the row. Never automatically replay Kill; passive recovery may
establish later definitive completion. Dismissal is run-local and blocks same-run
rediscovery; fallback selection never acquires another session.
Existing `c2 kill <session>` also terminates
[headless sessions](../../../GLOSSARY.md#sessions), whose zero-client lifetime
survives later attachment loss until completion or Kill; that is implemented,
and `c2 kill` sends this standalone Kill through the client library. The other amendments are unimplemented; operation
fields/results are unchanged.
#69 owns agreed deadlines/bounds; [native declarations](../monitor-client/core-v1.md)
are agreed in #71; the [acceptance matrix](../../monitor-acceptance.md)
is agreed in #62, not executed conformance evidence.
Related: [Command final](event-command-final.md).

## Implemented behavior

The [session daemon](../../../GLOSSARY.md#sessions) validates a first-role
Kill like Open: `unsupported_revision` unless revision 1, and
`unsupported_capability` for an unknown or unadvertised required family. Each
rejection has no effects, carries the command's result as `known_final` once
it is final (see [Command final](event-command-final.md#implemented-behavior)),
and ends the stream. A Kill on a connection that already
has a role, such as a registered
[passive monitor connection](../../../GLOSSARY.md#attachment-and-control), is
`invalid_state` and keeps that role.

A valid standalone Kill gives its connection the Kill role, which is not an
[attachment](../../../GLOSSARY.md#attachment-and-control), and commits the
session's termination at once: new
interactive work is refused, the command's terminal is hung up, and after the
five-second [hang-up grace](../../../GLOSSARY.md#attachment-and-control) the
daemon kills what remains of the command's terminal session and cleans up.
If the command already exited and its remaining output is still being
drained, the hang-up waits for that drain: the output and then
[Command final](event-command-final.md#implemented-behavior) reach the
connections first (owner decision on
[#88](https://github.com/jackgeek/climon2/issues/88), 2026-11-14). It also
waits, as Command final does, while a
[restoration](../../../GLOSSARY.md#restoration-and-presentation) is in
progress or a [synchronized update](../TERMS.md#synchronized-update) is open
([#203](https://github.com/jackgeek/climon2/issues/203)). The update ends
when the application ends it or its 150 ms latch expires. Once termination
is committed, the restoration deadline for a missing
[Ready](request-ready.md) no longer applies, so a restoration ends only when
it completes or its reader is lost after 5 s without progress. A
later Kill sent while termination is committed joins it without
restarting the grace. Every Kill is answered once the session has ended, at
once if it already had:

- success, `{"kind":"termination","command_result":<CommandResult>,"cleanup":"complete"}`,
  when the daemon confirmed how the command ended and its cleanup completed;
- otherwise outcome `failed` with `cleanup_failed` and
  `{"kind":"termination","command_result":<CommandResult or null>,"cleanup":"incomplete"}`:
  the daemon could not confirm the command's end, or its cleanup of the
  session metadata and socket failed. The result is the one the daemon
  knew, even when it could not confirm the end; it is null only when the
  daemon never learned it.

The stream then ends. An authoritative failure of the session, such as a
failed write to the command's terminal, does not by itself fail a Kill whose
termination was confirmed and cleaned up. While a Kill awaits its answer, a
later Open or Kill on its connection is `invalid_state`, as are Leave and
Acquire, since it opened nothing; Input, Resize and Ready are
`not_attached`, and Focus is `unsupported_capability`. The Kill's connection
receives no [Command final](event-command-final.md): its answer carries the
result. `c2 kill` sends this Kill through its own release's client library
([#100](https://github.com/jackgeek/climon2/issues/100)).
