# Frozen link revisions and opening exchange

A hub monitor and a relay run on different machines that upgrade independently, and a relay restarts itself into a new version while linked, so the two ends of a monitor link often run different releases. We decided that the link protocol is published as numbered link revisions, each frozen once shipped, and that every connection begins with an opening exchange that is frozen forever and selects the highest revision both ends support. Release numbers never decide anything. It is decided in [#196](https://github.com/jackgeek/climon2/issues/196).

One link revision covers pairing, the pinned-key handshake, framing, the link's records and the cryptographic choices; changing any of them means a new revision. Each revision also names which client interface revisions its session streams may carry. Optional behaviour inside a revision is advertised as capability tokens after the handshake, inside the encrypted link.

The opening exchange happens before authentication. The dialer sends a fixed marker, the purpose (pair or link) and the revisions it supports; the relay answers with the chosen revision, or with a refusal carrying its own list. Its layout is fixed binary with no extension room, and nothing else is sent in the clear: no release version, identity or key. Both messages are bound into the key exchange that follows, so an attacker who edits the lists makes the exchange fail and cannot force an older revision.

## Considered Options

- **Exact release match across the link.** Rejected. The two machines upgrade apart, so a link would break on every upgrade of either end.
- **Infer compatibility from release numbers.** Rejected, as for daemon wire revision 1 (ADR 0001): advertised revisions and capabilities say what an end can do, and release numbers do not.
- **An extensible opening exchange, as hello is.** Rejected. It is parsed from unauthenticated peers, and anything that must grow can travel inside the encrypted link.

## Consequences

- Every release supports every published link revision. This is a standing compatibility commitment of the kind ADR 0001 rejected for the daemon protocol; it is accepted here because exact-release matching cannot cross a machine boundary. Retiring a revision needs an ADR.
- Each supported revision keeps its recorded fixtures in every test run.
- Every connection negotiates afresh. Neither end remembers the revision used before, and a remembered pairing is not tied to a revision, so an upgrade never forces re-pairing.
- With no shared revision the link is refused, and each end names the machine that must upgrade. The refusal is unauthenticated, so an on-path attacker can forge one; that gains only the denial of service they already have.
- Within a revision an unknown record tag is a protocol error that ends the link. It is never skipped.
