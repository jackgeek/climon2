# climon2-link

The link code of [monitor links](../../GLOSSARY.md#monitor-links): what a
[hub monitor](../../GLOSSARY.md#monitor-links) and a
[relay](../../GLOSSARY.md#monitor-links) say to each other over a plain byte
stream ([ADR 0011](../../docs/adr/0011-monitor-library-and-client-embedding.md)).
Monitor links are planned. So far this crate holds the interface every part
is driven through, the test harness that drives it, and the first parts of the
[link protocol](../../docs/for-humans/protocol/monitor-link/README.md): the
opening exchange and pairing. Nothing that follows the opening exchange on a
connection opened to link is built, and no executable links the crate.

## One implementation for every platform

The crate performs no input or output. It opens no socket, reads no file and
no clock, and draws no randomness of its own: its caller owns the byte stream
and hands the crate everything else. The hub monitor's side and the relay's
side therefore run the same code on every operating system, and a test can
decide the time and the randomness exactly.

The architecture guard (`xtask/tests/architecture.rs`) keeps it so. It fails
when this crate:

- depends on a crate outside its allowlist, whatever the kind of dependency.
  It never holds `climon2-platform`, an operating-system or socket crate, an
  async runtime or a source of randomness. Today it holds the cryptography
  of pairing, listed [below](#the-cryptography-of-pairing);
- has a dependency for one platform only, or a build script;
- writes a `cfg` that names a platform: `unix`, `windows` or any `target_`
  key;
- names `std::net`, `std::os`, `std::fs`, `std::process` or `std::env`,
  renames `std` or imports all of it;
- writes the name of a standard clock, `Instant`, `SystemTime` or
  `UNIX_EPOCH`, or of the standard library's randomly keyed `HashMap`,
  `HashSet` or `RandomState`. Use `BTreeMap` and `BTreeSet`.

A change that needs a new dependency adds it to `LINK_ALLOWLIST` in that
test, where a reviewer sees it.

## Public boundary

- `Time`: a moment on the caller's clock, as the time elapsed since a start
  the caller fixed. `Time::since_start(elapsed)` makes one and `elapsed()`
  reads it back.
- `Randomness`: the caller's source of random bytes, `fill(&mut [u8])`.
- `End`: one end of one byte stream. Its only method is
  `step(now, input, randomness) -> Output`.
- `Input`: what a step hands an end. `Tick` means nothing arrived (the byte
  stream started, or time passed), `Bytes` carries the next bytes in order,
  and `Closed` says the other end closed the byte stream.
- `Output`: the `bytes` to send next, whether to `close` the byte stream
  after them, and the end's `deadline` if it has one. A caller never steps an
  end again once it asks to close. When nothing arrives sooner, the caller
  steps the end with `Tick` at the deadline; each step's answer replaces the
  deadline before.
- `HubEnd` and `RelayEnd`: the two ends, described next.

## The opening exchange

Every connection starts with the
[opening request](../../docs/for-humans/protocol/monitor-link/opening-request.md)
and the
[opening reply](../../docs/for-humans/protocol/monitor-link/opening-reply.md),
which pick the highest
[link revision](../../GLOSSARY.md#monitor-links) both ends support
([ADR 0010](../../docs/adr/0010-frozen-link-revisions-and-opening-exchange.md)).

- `Revisions::new(&[u16])` is the list of link revisions one end supports. It
  gives nothing unless the list is one to eight revisions, ascending, with no
  zero and no repeat.
- `HubEnd::new(purpose, revisions)` is the hub monitor's end. `Purpose` is
  `Pair` or `Link`. Its first step sends the 26-byte request and names a
  deadline 10 seconds on, by which the whole 20-byte reply must have arrived.
- `RelayEnd::new(revisions, invitations)` is the relay's end. Its first step
  is the moment it accepted the connection, and the whole request must arrive
  within 10 seconds of it. `invitations` is the relay's link invitation,
  described [below](#pairing): a hub monitor that connects to pair gets a
  pairing run only when it allows one.
- `opening()` on either end gives the `Opening`:
  - `Pending`: the exchange has not finished.
  - `Opened`: a revision was chosen. It holds the `purpose`, the `revision`
    and the `transcript`.
  - `Incompatible { theirs }`: the ends share no revision. `theirs` is the
    other end's list, so the caller can say which machine must upgrade.
  - `PairingUnavailable`: the relay can start no pairing run.
  - `Failed`: the other end's message was malformed, partial or late. A relay
    sends no byte in answer; for a hub monitor it is a failed dial.

  Every outcome but `Opened` closes the byte stream.
- `Transcript::as_bytes()` is the 46 bytes of the request followed by the
  reply, exactly as they crossed the byte stream. The key exchange that
  follows takes them as input, so an attacker who edits either message leaves
  the two ends with different transcripts and that exchange fails. Pairing's
  key exchange takes them; the pinned-key handshake is not built.

An end keeps nothing between connections: each one is a new `HubEnd` or
`RelayEnd` and negotiates from scratch.

Three points go beyond the letter of the protocol pages, each the stricter or
simpler reading:

- A hub monitor treats a reply as a failed dial unless it is what a relay
  holding the list in that reply would answer. A chosen revision must be the
  highest in both lists, and a "no shared revision" refusal must come with a
  list that shares none.
- A relay that shares no revision with a hub monitor that came to pair
  answers "no shared revision", whether or not it could start a pairing run.
- Bytes that arrive behind a whole opening message are not part of it. On a
  connection opened to pair they are the start of the pairing run. On one
  opened to link the ends drop them today, because nothing follows the
  opening exchange there yet.

## Pairing

A connection opened to pair carries the four messages of the
[pairing share](../../docs/for-humans/protocol/monitor-link/pairing-share.md)
and
[pairing confirmation](../../docs/for-humans/protocol/monitor-link/pairing-confirmation.md)
pages, then closes.

- `Pin` is the 8-digit PIN of a link invitation. `Pin::parse(typed)` ignores
  hyphens and spaces and gives nothing unless 8 digits remain. It shows as
  `1234-5678`; its `Debug` form hides the digits.
- `Introduction::new(key, name)` is what one end hands the other once the PIN
  is proven: its X25519 static public key and a suggested name of at most 64
  bytes. A name from the other end is hostile text.
- `Invitations::new(relay)` is a relay's link invitation, made with the
  relay's own introduction: its identity key and name. Copies share one
  invitation, so the relay keeps one and hands one to every `RelayEnd`.
  - `mint(now, randomness)` gives a new PIN and revokes the invitation before
    it. The invitation expires 5 minutes after `now` and survives two failed
    runs; the third uses it up.
  - `cancel()` ends a live invitation.
  - `invitation(now)` gives the `Invitation` as it stands: `None`,
    `Live { expires, attempts_left }`, `Paired(introduction)`, `Expired`,
    `UsedUp` or `Cancelled`. An ended invitation stays ended; nothing mints
    one by itself.
  - One pairing run at a time: while one is in flight, a second connection
    that comes to pair is refused with "pairing unavailable" and does not
    count.
- `HubEnd::pair(revisions, pin, introduction)` is a hub monitor's end that
  pairs. `introduction` holds the public key of a key pair made for this
  pairing alone.
- `pairing()` on either end gives the `PairingRun`: `Pending`,
  `Paired(introduction)` with the other end's key and name, or
  `Failed { counted }`, where `counted` says whether the relay's share had
  crossed the byte stream, the moment from which a failure is one of the
  invitation's attempts. Either way the byte stream closes.

The relay pins the hub monitor's key, and the invitation is consumed, at the
step whose answer carries the relay's confirmation. A hub monitor's `Paired`
is not yet "paired": it reports that only once its first link with the
relay's key succeeds, which joins pairing to the pinned-key handshake and is
built later.

Each run must end 30 seconds after the relay accepted the connection, and
after the hub monitor's first step. A run whose invitation is revoked,
cancelled or expires before it ends fails at the relay's next step.

### The cryptography of pairing

Each crate is pinned to one exact version in `Cargo.toml` and is in the
architecture guard's allowlist. None has been audited.

| Crate | Version | Job |
|---|---|---|
| `pakery-cpace`, `pakery-crypto` | 0.6.2 | CPace on ristretto255 with SHA-512, following `draft-irtf-cfrg-cpace-21` |
| `rand_core` | 0.10.1 | The traits `pakery` draws the caller's randomness through; it holds no generator |
| `hkdf`, `hmac`, `sha2` | 0.13.0, 0.13.0, 0.11.2 | The pairing key schedule and the confirmation |
| `chacha20poly1305` | 0.11.0 | The box that carries each end's key and name |

The known-answer tests catch an upgrade that changes a byte:
`src/pairing/cpace.rs` runs the draft's published vectors of appendix B.3,
`src/pairing/keys.rs` holds climon2's frozen key schedule, and
`tests/pairing_vectors.rs` a whole frozen pairing connection.
`tests/vectors/pairing_reference.py`, which shares no code with this crate,
computed every frozen byte from the protocol pages after reproducing the
draft's and RFC 8439's own vectors. Run it with `python3` to see them again.

## The Link-layer harness

`tests/support/mod.rs` is the harness of the Link layer of the
[monitor-link acceptance criteria](../../docs/for-humans/behaviour/monitor-link/acceptance.md#evidence-policy).
`Harness::new(hub, relay)` joins a hub monitor's end and a relay's end by two
in-memory byte streams, starting at time zero.

- `run()` steps each end once with `Tick`, the hub monitor's first, then
  carries bytes and closes between them until both are quiet.
- `advance(duration)` moves the supplied time on and runs the ends. On the
  way it stops at every deadline an end has named and runs the ends there.
- `hub_deadline()` and `relay_deadline()` show the deadline each end named
  last.
- `fragment(n)` hands bytes over at most `n` at a time, because a byte stream
  keeps no message boundaries.
- `with_seeds` gives each end its own `Seeded` randomness; the same seed
  always gives the same bytes.
- `sent_by_hub()`, `sent_by_relay()`, `hub_closed()` and `relay_closed()`
  show what each end did to its byte stream.

## Specification mapping

| Scenario | Evidence |
|---|---|
| ONE-01: the guard rejects a dependency outside the allowlist, a `cfg` naming a platform, the platform modules of `std` and the standard clocks | `xtask/tests/architecture.rs`: `link_code_keeps_its_rules`, `link_guard_rejects_each_sample_crate_for_its_own_rule`, `link_guard_accepts_a_sample_crate_that_keeps_every_rule`, `link_source_scan_sees_through_every_spelling` |
| ONE-02: time and randomness reach the link code only as inputs | the ONE-01 tests above, which reject every other source, and `tests/harness.rs`: `one_02_the_test_supplies_the_time_of_every_step`, `one_02_the_test_supplies_each_end_its_randomness` |
| REV-01: the opening request is 26 bytes and the reply 20, in the published layout; the ends use the highest revision they share; every connection negotiates from scratch | `tests/opening.rs`: the `rev_01_` tests |
| REV-03, as far as the opening exchange goes: edited revision lists leave the two ends with different transcripts or a failed dial, and neither end tries another revision. That the key exchange then fails is proven with each key exchange: for pairing, by the PAIR-06 row. | `tests/opening.rs`: the `rev_03_` tests |
| PAIR-01, the Link layer: four messages on a connection of their own, each end handed the other's key and name, the invitation consumed, a PIN typed with or without hyphens and spaces | `tests/pairing.rs`: the `pair_01_` tests |
| PAIR-02, the Link layer: one live invitation, expiry at 5 minutes with supplied time, cancelling, one hub monitor per invitation | `tests/invitation.rs`: the `pair_02_` tests |
| PAIR-03, the Link layer: three counted failures use the invitation up; nothing before the relay's share counts; a concurrent run is refused and does not count; the 30-second deadline with supplied time; oversized messages | `tests/invitation.rs`: the `pair_03_` tests |
| PAIR-05: a party without the PIN gets one guess a run and never the PIN's digits | `tests/pairing.rs`: the `pair_05_` tests |
| PAIR-06 and REV-03 on a pairing connection: edited openings, reflected and spliced messages and flipped bits fail the key exchange, count and pin nothing | `tests/pairing.rs`: the `pair_06_` tests |
| PAIR-07: a hub monitor that stops after the relay checked its confirmation leaves the relay paired and itself not | `tests/pairing.rs`: `pair_07_a_hub_monitor_that_stops_at_the_end_leaves_the_relay_paired` |
| PAIR-08, the Link layer: an invitation changes nothing for a connection opened to link until a second hub monitor pairs | `tests/pairing.rs`: `pair_08_an_invitation_leaves_link_connections_alone_until_a_second_pairing` |
| SEC-08, for pairing: the published CPace vectors and climon2's frozen key schedule and pairing connection | `src/pairing/cpace.rs` and `src/pairing/keys.rs`: the `sec_08_` tests; `tests/pairing_vectors.rs` |
| SEC-04, the opening half: a malformed, partial or late opening request is closed without a byte in reply, at 10 seconds at the latest | `tests/opening.rs`: the `sec_04_` tests |
