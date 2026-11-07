# Sound one host bell per presented toast

**Agreed, unimplemented**, from [the selector decision](https://github.com/jackgeek/climon2/issues/60).
The [monitor](../../../GLOSSARY.md#sessions) sends
[BEL](../TERMS.md#bel) to its [host](../../../GLOSSARY.md#sessions)
when a new eligible toast first begins drawing its border.
Use the [selector specification](../../monitor-selector.md),
[shared host contract](../README.md#monitor-to-host-status-and-bell),
[domain glossary](../../../GLOSSARY.md) and [technical terms](../TERMS.md).

There are no fields or payload beyond the single byte:

```rust
b"\x07"
```

Planned startup boolean `monitor.bell` defaults to true. Validate one
[climon2-home](../../../GLOSSARY.md#installation)
[JSONC](../TERMS.md#jsonc) snapshot before terminal setup; missing file/object/field uses the default,
wrong types/null and duplicate known fields fail, and there is no live reload.
Unknown/unrelated settings are ignored. False suppresses this byte, not the
toast or [OSC](../TERMS.md#osc) 7501 reporting.

Send once per presentation, including initial and queued toasts when they
actually become visible. Do not send per frame, redraw, resize, hint toggle,
dismissal or coalesced request. A toast that never begins drawing sends no bell.
No acknowledgement is expected, and successful writing does not prove an audible
sound: host settings may suppress it or display a visual bell.
The host may independently sound on program-status changes.
This is the monitor's own [UI](../TERMS.md#ui) signal, never session BEL passthrough.
It cannot acquire control, acknowledge attention or change a session [FSM](../TERMS.md#fsm).
Host-output failures remain with
[lifecycle](https://github.com/jackgeek/climon2/issues/61), not silent success.
