# Title state and changes

**Status:** agreed semantics/wire mapping, not implemented; native [schemas](../TERMS.md#schema) unresolved.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [session daemon](../../../GLOSSARY.md#sessions)'s core supplies authoritative
current terminal title state through the
[client](../../../GLOSSARY.md#sessions) to the
[monitor](../../../GLOSSARY.md#sessions). Title delivery is not a
[plugin](../../../GLOSSARY.md#protocol-and-plugin-concepts)
[extension](../../../GLOSSARY.md#protocol-and-plugin-concepts).
Establishing a supported passive connection must provide current title state
followed by ordered changes, with no gap in which a change can be missed.
Updates continue while background,
[revoked](../../../GLOSSARY.md#attachment-and-control), or before
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts), including when
the menu is closed.

Title delivery is independent of
[control](../../../GLOSSARY.md#attachment-and-control). It neither
[grants](../../../GLOSSARY.md#attachment-and-control) control
nor permits screen mirroring, input, or resize. Do not scrape
display output or require [restoration](../../../GLOSSARY.md#restoration-and-presentation)
to obtain the title.

The [status/activity revision](../../session-status.md) supersedes the earlier
built-in title-plugin requirement. Plugin implementation, configuration, and
interface examples are deferred; there is no plugin release prerequisite.
Title changes also count as activity under the
[snapshot strategy](../../session-status.md#snapshot-sampling-and-the-inactivity-deadline).

Older daemons without title support remain usable: absence must not
block monitor connection or control. Other required extensions retain their
fail-before-control rule. Report negotiation/runtime failures explicitly and
mark cached titles stale/unavailable if delivery becomes uncertain.

The menu uses the latest non-empty daemon title, otherwise command plus
arguments. This fallback also applies before a title is received or when a
daemon cannot supply one. Keep otherwise live sessions visible. An empty title
returns to the fallback. Render all text literally, never as terminal commands.
Unsupported/unknown title state is not a fabricated supported empty title,
even though both use the same menu fallback.
The [terminal chrome contract](../terminal-integration.md#chrome-cursor-and-host-copy)
also uses this as a label in one permanent top row, never a host-title change.
Only overflowing label text marquees, with
[display-cell](../TERMS.md#display-cell)-safe clipping and restart
on label changes; short labels and status/control/error remain static. Full
details stay in the selector.

The [shared contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples) require explicit
initial core metadata synchronization boundaries per
[connection generation](../TERMS.md#connection-generation), followed by
gap-free changes. This metadata barrier does not make stale terminal content
current. On passive reconnect, refresh cached title state without acquiring
control. Reports use the single [response](../TERMS.md#response)/[event](../TERMS.md#event)
stream and its bounded ownership/[backpressure](../TERMS.md#backpressure) rules,
without silently losing required updates.

Never display [session IDs](../../../GLOSSARY.md#protocol-and-plugin-concepts)
in the menu. Use them internally to preserve identity, highlight, and
confirmation across duplicate or changing titles. Other identifying metadata
may distinguish rows; layout belongs to the
[selector decision](https://github.com/jackgeek/climon2/issues/60).

The [startup/discovery resolution](https://github.com/jackgeek/climon2/issues/59)
owns these semantics. The [daemon contract](https://github.com/jackgeek/climon2/issues/67)
assigns `title-v1`, [Initial metadata](../daemon-client/event-initial-metadata.md)
with explicit availability, then [Title changed](../daemon-client/event-title-changed.md).
Available title permits null (unset), empty or nonempty
[Unicode](../TERMS.md#unicode-and-utf-8) text.
Text is inert [JSON](../TERMS.md#json) metadata, not terminal presentation encoding or
percent-encoded [OSC](../TERMS.md#osc); source invalid
[UTF-8](../TERMS.md#unicode-and-utf-8) replacement is a text-boundary choice.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) still owns
native fields/tags, declarations and executable examples, preserving
[#66's agreed communication rules](https://github.com/jackgeek/climon2/issues/66).
[Operational limits](../../monitor-operational-limits.md#payloads-and-parser-state) specifies numerical
bounds and progress.

Users do not set terminal titles through climon2. Future user-assigned session
names and persisted renaming are separate: the name will appear alongside the
terminal title when the title is non-empty. Naming remains future work and
would require revisiting the
[write-once metadata rule](../../adr/0005-frozen-session-metadata-core.md).
