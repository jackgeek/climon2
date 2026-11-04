# Title plugin state and changes

**Status:** agreed semantics, not implemented; exact interface and wire schemas unresolved.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

A built-in [session daemon](../../../GLOSSARY.md#sessions)
[plugin](../../../GLOSSARY.md#protocol-and-plugin-concepts) publishes the daemon's
authoritative current terminal title through negotiated
[extension](../../../GLOSSARY.md#protocol-and-plugin-concepts) traffic. The
[client](../../../GLOSSARY.md#sessions) relays it opaquely to the
[monitor](../../../GLOSSARY.md#sessions), without a dedicated core title message
or a special bypass of the plugin interfaces. With support negotiated,
establishing a passive connection must provide current title state followed by
ordered changes, with no gap in which a change can be missed. Updates continue
while background,
[revoked](../../../GLOSSARY.md#attachment-and-control), or before
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts), including when
the menu is closed.

Title delivery is independent of
[control](../../../GLOSSARY.md#attachment-and-control). It neither grants control
nor permits screen mirroring, input, or resize. Do not scrape
display output or require [restoration](../../../GLOSSARY.md#restoration-and-presentation)
to obtain the title.

The built-in plugin may be contained in the daemon executable. Release is
blocked until it is implemented and automated integration evidence demonstrates
the actual plugin interfaces, publication, relay, and monitor consumption.
Conforming test endpoints alone do not prove that integration. General-purpose
plugin loading/configuration remains deferred.

This mandatory implementation is distinct from a required peer capability.
Older daemons without title-extension support remain usable: absence must not
block monitor connection or control. Other required extensions retain their
fail-before-control rule. Report negotiation/runtime failures explicitly and
mark cached titles stale/unavailable if delivery becomes uncertain.

The menu uses the latest non-empty daemon title, otherwise command plus
arguments. This fallback also applies before a title is received or when a
daemon cannot supply one. Keep otherwise live sessions visible. An empty title
returns to the fallback. Render all text literally, never as terminal commands.

Never display [session IDs](../../../GLOSSARY.md#protocol-and-plugin-concepts)
in the menu. Use them internally to preserve identity, highlight, and
confirmation across duplicate or changing titles. Other identifying metadata
may distinguish rows; layout belongs to the
[selector decision](https://github.com/jackgeek/climon2/issues/60).

The [startup/discovery resolution](https://github.com/jackgeek/climon2/issues/59)
owns these semantics. The [daemon contract](https://github.com/jackgeek/climon2/issues/67),
[monitor-client communication](https://github.com/jackgeek/climon2/issues/66),
and [interface schemas](https://github.com/jackgeek/climon2/issues/71) must
define exact fields, encoding, initial-state/change ordering, compatibility,
support detection, validation, errors, and valid examples.
[Operational limits](https://github.com/jackgeek/climon2/issues/69) owns numerical
bounds and progress. No event name, signature, capability token, or wire example
is assigned here.

Users do not set terminal titles through climon2. Future user-assigned session
names and persisted renaming are separate: the name will appear alongside the
terminal title when the title is non-empty. Naming remains future work and
would require revisiting the
[write-once metadata rule](../../adr/0005-frozen-session-metadata-core.md).
