# Pairing share

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480). The `climon2-link` crate implements it; no released program speaks it yet.**
The first two messages of a pairing connection. Each end sends its public
share of a [CPace](../TERMS.md#cpace) key exchange keyed by the PIN of a
[link invitation](../../../../GLOSSARY.md#monitor-links). The PIN itself
never crosses the wire. See the [overview](../README.md),
[glossary](../../../../GLOSSARY.md), [terms](../TERMS.md) and the
[shared rules](README.md).

## Layout

The [hub monitor](../../../../GLOSSARY.md#monitor-links) sends its share
first. The [relay](../../../../GLOSSARY.md#monitor-links) then sends its own.
Both have the same layout, in the clear:

| Field | Size | Meaning |
|---|---|---|
| Length | `u16` | Always 32. |
| Share | 32 [bytes](../TERMS.md#byte) | A ristretto255 group element. |

A share is random and cannot be told apart from any other, so an example
shows nothing beyond the layout: `00 20` followed by 32 bytes.

## CPace inputs

| Input | Value |
|---|---|
| Specification | [`draft-irtf-cfrg-cpace-21`](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-21), April 2026. Its test vectors for this suite, in appendix B.3, are the ones an implementation must reproduce. |
| Suite | CPace on ristretto255 with SHA-512, the draft's `CPACE-RISTR255-SHA512`. |
| Mode | Initiator and responder. The hub monitor is the initiator. |
| Password | The 8 PIN digits as ASCII, without the hyphen. |
| Channel identifier | The ASCII text `climon2 link pairing` followed by the [opening transcript](README.md#cryptography). |
| Session identifier | Empty. |
| Associated data | Empty for both ends. |

## Rules

- The relay sends its share only to a valid hub monitor share. From that
  moment the run counts as one of the invitation's three attempts, unless it
  completes successfully. A hang-up, a timeout or a malformed message after
  this point counts.
- A Length other than 32, or a share that is not a valid group element, ends
  the connection with no reply. The neutral element is not a valid share.
  Each end judges a Length as soon as it arrives, before the share behind
  it.
- The hub monitor sends its share as soon as the
  [opening reply](opening-reply.md) has chosen a revision. Bytes that arrive
  behind a whole opening message on a pairing connection are the start of
  the next pairing message.
- Neither share lets its receiver test a guess at the PIN. The relay reveals
  nothing a guess can be tested against until the hub monitor has sent its
  [pairing confirmation](pairing-confirmation.md).
- The whole run must finish within 30 seconds of the relay accepting the
  connection. A message whose last byte arrives exactly at 30 seconds is in
  time. The hub monitor gives its own run the same 30 seconds, from the
  moment its connection opened.
- A run whose invitation is revoked by a new one, cancelled or expires before
  the run ends fails. The relay sends nothing more on that connection, and
  closes it when the next bytes arrive or the 30 seconds end, whichever
  comes first.

Owned by [#193](https://github.com/jackgeek/climon2/issues/193).
