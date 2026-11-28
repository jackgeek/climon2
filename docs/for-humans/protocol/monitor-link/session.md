# Session — `0x10`

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480). The `climon2-link` crate implements its layout and the 128-session limit; the relay advertises its home's sessions with it and the hub monitor lists them (#502); no released program speaks it yet.**
The [relay](../../../../GLOSSARY.md#monitor-links) advertises one of its
machine's [sessions](../../../../GLOSSARY.md#sessions) to the
[hub monitor](../../../../GLOSSARY.md#monitor-links) on stream 0. The same
record means "appeared" and "changed". See the [overview](../README.md),
[glossary](../../../../GLOSSARY.md), [terms](../TERMS.md) and the
[shared rules](README.md).

## Body

| Field | Type | Meaning |
|---|---|---|
| Session ID | 16 [bytes](../TERMS.md#byte) | The session's ID on the relay's machine. |
| Encoding | `u32` | How Command, Arguments and Working directory are encoded: 1 native Unix bytes, 2 native Windows, the client interface's [native path](../TERMS.md#native-path) encodings. |
| Command | `bytes` | The command the session runs. |
| Arguments | `u32` count, then that many `bytes` | The command's arguments. |
| Working directory | `bytes` | Where the session started. Display text only. |
| Depth | `u32` | The session's nesting depth. |
| Created at | 24 bytes | The client interface's [UTC](../TERMS.md#utc) timestamp form. |
| Release | `text` | The climon2 release the session runs. |

The record carries no socket path, process ID or other detail of the relay's
machine, and no status. Status arrives through the session's
[stream](stream-open.md).

## Example

A session running `vim notes.txt` in `/home/sam`:

```text
10 01 23 45 67 89 ab cd ef 01 23 45 67 89 ab cd
ef 00 00 00 01 00 00 00 03 76 69 6d 00 00 00 01
00 00 00 09 6e 6f 74 65 73 2e 74 78 74 00 00 00
09 2f 68 6f 6d 65 2f 73 61 6d 00 00 00 01 32 30
32 36 2d 31 31 2d 32 37 54 30 39 3a 33 30 3a 30
30 2e 30 30 30 5a 00 00 00 05 32 2e 34 2e 30
```

## Rules

- The relay starts discovery when the link is established. It sends one
  Session for every live session its first pass finds, then
  [Snapshot complete](snapshot-complete.md). After that it sends a Session
  each time one appears or changes.
- The hub monitor replaces whatever it held for that Session ID. A link
  connection starts with nothing held: after a reconnect the hub monitor
  discards the previous connection's sessions.
- The hub monitor names a [remote session](../../../../GLOSSARY.md#monitor-links)
  by the relay's pinned public key together with the Session ID. It never
  compares the ID with a local one.
- The relay never advertises the session it is itself running in.
- More than 128 sessions held at once is a
  [protocol violation](README.md#protocol-violations). An honest relay with
  more advertises the first 128 in its listing order.
- A session whose record would be longer than the hub monitor's
  [Hello](hello.md) ceiling is not advertised. The relay says so on its own
  screen.
- The hub monitor treats every field as hostile text.

Owned by [#195](https://github.com/jackgeek/climon2/issues/195) and
[#412](https://github.com/jackgeek/climon2/issues/412).
