# Snapshot complete — `0x12`

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480). The `climon2-link` crate implements its layout and the rule that it comes once; the relay sends it after its first pass (#502); no released program speaks it yet.**
The [relay](../../../../GLOSSARY.md#monitor-links) tells the
[hub monitor](../../../../GLOSSARY.md#monitor-links) that it has advertised
every session its first discovery pass found. See the
[overview](../README.md), [glossary](../../../../GLOSSARY.md),
[terms](../TERMS.md) and the [shared rules](README.md).

## Body

Empty. The record is the single tag [byte](../TERMS.md#byte):

```text
12
```

## Rules

- The relay sends it once on each link connection, after the
  [Session](session.md) records of its first pass. A relay with no sessions
  sends it alone.
- Until it arrives, the hub monitor knows the list may be incomplete. After
  it, a session not yet advertised does not exist on that relay.
- A second Snapshot complete on one connection is a
  [protocol violation](README.md#protocol-violations).
- The hub monitor may open [streams](stream-open.md) for advertised sessions
  before it arrives.

Owned by [#195](https://github.com/jackgeek/climon2/issues/195).
