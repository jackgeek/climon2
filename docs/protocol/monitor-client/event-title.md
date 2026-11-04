# Title state and changes

**Status:** agreed semantics, not implemented; exact interface and wire schemas unresolved.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [session daemon](../../../GLOSSARY.md#sessions) supplies its current terminal
title through the [client](../../../GLOSSARY.md#sessions) to the
[monitor](../../../GLOSSARY.md#sessions). Establishing a passive connection must
provide current title state followed by ordered changes, with no gap in which a
change can be missed. Updates continue while background,
[revoked](../../../GLOSSARY.md#attachment-and-control), or before
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts), including when
the menu is closed.

This is core metadata, independent of [plugin](../../../GLOSSARY.md#protocol-and-plugin-concepts)
support and [control](../../../GLOSSARY.md#attachment-and-control). It neither
grants control nor permits screen mirroring, input, or resize. Do not scrape
display output or require [restoration](../../../GLOSSARY.md#restoration-and-presentation)
to obtain the title.

The menu uses the latest non-empty daemon title, otherwise command plus
arguments. This fallback also applies before a title is received or when a
daemon cannot supply one. Keep otherwise live sessions visible. Mark a cached
title stale/unavailable when connection freshness becomes uncertain. An empty
title returns to the fallback. Render all text literally, never as terminal
commands.

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

Future user-assigned names and persisted renaming are outside this resolution.
They would require revisiting the
[write-once metadata rule](../../adr/0005-frozen-session-metadata-core.md).
