# Sound one host bell per presented toast

**Agreed** from [the selector decision](https://github.com/jackgeek/climon2/issues/60), **implemented** by the monitor ([#115](https://github.com/jackgeek/climon2/issues/115)): it writes the byte right after painting the frame in which a toast begins drawing, once per toast, the queue's overflow toast included.
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

Startup boolean `monitor.bell` defaults to true. Validate one
[climon2-home](../../../GLOSSARY.md#installation)
[JSONC](../TERMS.md#jsonc) snapshot before terminal setup; missing file/object/field uses the default,
wrong types/null and duplicate known fields fail, and there is no live reload.
Unknown/unrelated settings are ignored. False suppresses this byte, not the
toast or [OSC](../TERMS.md#osc) 7501 reporting.

Send once per presentation, including initial and queued toasts, when the
frame first showing the toast's full title (with any ellipsis) has finished
writing to the host, the moment its title hold starts (owner decision on
[#115](https://github.com/jackgeek/climon2/issues/115), 2026-11-17). Do not
send per frame, redraw, resize, hint toggle, dismissal or coalesced request.
A toast dismissed or erased before its full title was shown sends no bell,
and neither does one that never begins drawing.
No acknowledgement is expected, and successful writing does not prove an audible
sound: host settings may suppress it or display a visual bell.
The host may independently sound on program-status changes.
This is the monitor's own [UI](../TERMS.md#ui) signal, never session BEL passthrough.
It cannot acquire control, acknowledge attention or change a session [FSM](../TERMS.md#fsm).
Host-output failure initiates
[coordinated shutdown](../../monitor-lifecycle.md#coordinated-shutdown).
Restoration/root clear and diagnostics are best-effort when output is broken;
report failure, not silent success.
