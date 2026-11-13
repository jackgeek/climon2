# Initial metadata — `0x41`

**Status: agreed; implemented for the title**, owned by
[#67](https://github.com/jackgeek/climon2/issues/67); status, progress and a
known command result arrive with their families.
The [session daemon](../../../GLOSSARY.md#sessions) supplies an atomic report
to the [client](../../../GLOSSARY.md#sessions) after successful Open.
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame), all fields required:

| Field | Type/value |
|---|---|
| `title` | `Available<string>`; available value may be null, empty, nonempty. |
| `status` | `Available<Status>`; available requires nonnull value. |
| `progress` | `Available<Progress>`; available requires nonnull value. |
| `command_result` | `CommandResult` or explicit null. |

```json
{"title":{"available":true,"value":null},"status":{"available":true,"value":{"state":"needs_attention","changed_at":"2026-11-06T12:00:00.000Z","episode_id":"1"}},"progress":{"available":false,"value":null},"command_result":null}
```

Types, availability and family dependency rules are
[shared](core-v1.md#shared-types). Unavailable always requires null.
When terminal status and result are both present they must agree: completed is
normal exit zero; failed is nonzero exit or signal.
There are no defaults/unknown members.

## Order and guarantees

Core sends this barrier after [Open response](response.md), atomically
capturing/subscribing before subsequent selected-family changes.
Interactive [grant](../../../GLOSSARY.md#attachment-and-control)/
[restoration](../../../GLOSSARY.md#restoration-and-presentation) follows this report.
Optional title/status/
progress availability reflects selected `title-v1`/`status-v1`/`progress-v1`;
it must not fabricate empty title, cleared
[attention](../../../GLOSSARY.md#status-and-attention) or no-progress for
unsupported families.

The report works while passive, background or revoked, without
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts). It does not
grant control, refresh retained screen content, replay notification payloads
or acknowledge attention. A known command result is distinct from uncertain
disconnection and does not prove client cleanup.
Malformed records/wrong directions are fatal under
[shared validation](core-v1.md#framing-and-validation); bounded delivery must
preserve the report or explicitly fail, never silently skip the barrier.
Numeric bounds/policy remain #69.
Related: [Title changed](event-title-changed.md),
[Status changed](event-status-changed.md),
[Progress changed](event-progress-changed.md), [Command final](event-command-final.md).

## Implemented behavior

After each successful monitor-mode Open, the
[session daemon](../../../GLOSSARY.md#sessions) captures the session's
current title and registers the connection for later changes in one step of
its loop, so no change falls between this record and the first
[Title changed](event-title-changed.md). With `title-v1` selected, `title` is
available with the application's title by value: null while unset, `""` when
set empty. Without it, `title` is unavailable. `status` and `progress` are
always unavailable for now, and `command_result` is null: the daemon cannot
yet tell a normal exit from a signal, which a known result needs
([#88](https://github.com/jackgeek/climon2/issues/88)).
