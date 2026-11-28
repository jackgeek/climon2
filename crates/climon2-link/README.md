# climon2-link

The link code of [monitor links](../../GLOSSARY.md#monitor-links): what a
[hub monitor](../../GLOSSARY.md#monitor-links) and a
[relay](../../GLOSSARY.md#monitor-links) say to each other over a plain byte
stream ([ADR 0011](../../docs/adr/0011-monitor-library-and-client-embedding.md)).
Monitor links are planned. So far this crate holds the interface every part
is driven through, the test harness that drives it, and three parts of the
[link protocol](../../docs/for-humans/protocol/monitor-link/README.md): the
opening exchange, a link connection between two ends that already hold
each other's keys, and the records that follow Hello. Pairing, credit,
sending keepalives, the silence timer, the stream-0 queue limit and ending
one stream for a stream violation are not built, and no executable links
the crate.

## One implementation for every platform

The crate performs no input or output. It opens no socket, reads no file and
no clock, and draws no randomness of its own: its caller owns the byte stream
and hands the crate everything else. The hub monitor's side and the relay's
side therefore run the same code on every operating system, and a test can
decide the time and the randomness exactly.

The architecture guard (`xtask/tests/architecture.rs`) keeps it so. It fails
when this crate:

- depends on a crate outside its allowlist, whatever the kind of dependency.
  The allowlist holds `snow` (see [Cryptography](#cryptography)). It never
  holds `climon2-platform`, an operating-system or socket crate, an async
  runtime or a source of randomness;
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

## Cryptography

The [Noise](../../docs/for-humans/protocol/TERMS.md#noise) handshake and the
units use the `snow` crate, pinned to exactly 0.10.0 in `Cargo.toml`. It is
built without default features: only X25519, ChaCha20-Poly1305 and SHA-256,
from the pure-Rust RustCrypto and `curve25519-dalek` crates, and none of
`snow`'s own randomness. Each ephemeral key is drawn from the caller's
`Randomness`.

`snow` has had no formal audit, and says so. Using it is an accepted risk,
chosen in the
[cryptography survey](https://github.com/jackgeek/climon2/blob/research/monitor-link-pake/docs/research/monitor-link-pake.md):
it implements Noise revision 34, has been released since 2017, and its one
advisory, RUSTSEC-2024-0011, was fixed in 0.9.5. The crates beneath it are
pinned only by `Cargo.lock`. Two known-answer tests catch any replacement or
upgrade that changes the bytes (SEC-08): the published cacophony vector for
`Noise_IK_25519_ChaChaPoly_SHA256` in `src/noise.rs`, and climon2's frozen
vector of a whole link connection in `tests/handshake.rs`. An implementation
of the pattern that shares no code with `snow`, `tests/vectors/noise_ik.py`,
reproduces the cacophony vector and computed the frozen one.

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
- `StaticKey` and `PublicKey`: a long-term X25519 key pair and its public
  half. `StaticKey::from_private(bytes)` takes the 32 random bytes the caller
  drew once and stores; `public()` gives the half the other end pins.
  `Debug` never shows the private half.

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
- `RelayEnd::new(revisions, pairing)` is the relay's end. Its first step is
  the moment it accepted the connection, and the whole request must arrive
  within 10 seconds of it. `Pairing` says whether a hub monitor that connects
  to pair can start a pairing run now; the caller knows, and the pairing code
  that will tell it is not built.
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
  the two ends with different transcripts and that exchange fails. The
  pinned-key handshake does this; pairing is not built.

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
- Bytes that arrive with a whole opening message, behind it, make it
  malformed. Neither end sends anything after its opening message until the
  other end has answered it, so only a broken or hostile end sends them. The
  relay closes without a byte in reply, and for the hub monitor it is a
  failed dial. Bytes that arrive later belong to whatever follows the
  opening exchange.

## The link connection

An end that holds the keys of a pairing goes on, after an opening exchange
that chose a revision for a link connection, to the
[handshake](../../docs/for-humans/protocol/monitor-link/handshake-initiation.md),
[Hello](../../docs/for-humans/protocol/monitor-link/hello.md) each way and
[units](../../docs/for-humans/protocol/monitor-link/README.md#units).

- `HubEnd::dialling(revisions, own, relay, hello)` is a hub monitor's end
  that links: `own` is its static key for this relay, `relay` the relay's
  pinned key, and `hello` the `Hello` it states.
- `RelayEnd::new(revisions, pairing).linking(identity, remembered, hello)` is
  a relay's end that links: `identity` is its identity key and `remembered`
  the public keys of the hub monitors it has paired with. A relay answers the
  handshake of any key, then sends Close with "not paired", and no Hello, to
  one it does not remember.
- `Hello::new(release, name, largest_record)` gives nothing unless the
  release is 1 to 64 bytes, the name at most 64 and the largest record at
  least 4,096. A Hello lists no capability token; tokens received are read
  and ignored.
- `link()` on either end gives the `Link`:
  - `Pending`: the opening exchange, the handshake or the two Hellos are not
    done. A relay treats nothing as linked until the hub monitor's Hello has
    arrived, so a replayed handshake changes nothing.
  - `Established(Peer)`: the other end's proven `key` and its `hello`.
  - `Ended(Ended)`: `Failed` before the link was established, without a
    Close (for a hub monitor, a failed dial); `Left(reason)` when this end
    sent Close; `Closed(reason)` when the other end did; `Lost` when an
    established link ended without one.
- `send(record)` queues one record, described [next](#records-after-hello).
  It is `Refused` before the link is established or after it ends
  (`NotLinked`), above the other end's largest record (`TooLong`), and when
  it breaks an order rule (`OutOfTurn`). Hello and Close are not records a
  caller sends: the link code sends them itself.
- `close(reason)` ends an established link with Close. A relay's reasons are
  `RelayClose` and a hub monitor's `HubClose`, each with its code on the
  wire.
- `records()` takes the records that have arrived since the last call, in
  the order they ended.

Every framing, Hello or Close rule broken is a protocol violation: the end
sends Close with "protocol violation" and closes. A unit that does not
decrypt, because it was altered, dropped, repeated, reordered or injected,
ends the connection without a Close, as a lost link once established.

Two points go beyond the letter of the protocol pages, each the stricter
reading:

- The relay allows 30 seconds from accepting the connection to the hub
  monitor's Hello. The hub monitor allows the same 30 seconds from dialling
  to the relay's Hello or Close, so a relay that holds a dial open after the
  opening exchange cannot hold it for ever.
- Bytes that arrive with a whole handshake message, behind it, end the
  connection with no reply, as the opening messages do.

## Records after Hello

Once the link is established, the hub monitor sends a `HubRecord` and the
relay a `RelayRecord`; each end's `records()` gives the other's. The layouts
are those of the
[record pages](../../docs/for-humans/protocol/monitor-link/README.md#contents).

- `RelayRecord::Advertisement` is stream 0: `Session`, `Withdrawn` with a
  `SessionId`, or `SnapshotComplete`.
- `RelayRecord::Stream { stream, record }` is a `RelayStream` on a session
  stream: `Opened`, `Refused`, `Ended`, `RequestRefused` or a client
  interface `Event`.
- `HubRecord { stream, record }` is a `HubStream`: `Open`, `Close` or a
  client interface `Request`.
- The `client` module holds `Request` and `Event` and the groups they are
  made of. Each mirrors its structure in the
  [client interface header](../../docs/for-humans/protocol/monitor-client/client-v1.h),
  with reserved fields left out, and carries every number as the library
  gave it.

`src/layout.rs` states the one mechanical rule once, as two macros: a
`layout!` struct crosses as its fields in the order written, and a
`tagged!` enum as its tag, then its active variant's fields. Every record
type is declared with them, so a type's field order and its layout cannot
drift apart. Because the codecs are generated, the CRAP and mutation tools
score none of them; `tests/encoding.rs` checks every record against bytes
written by hand from the pages.

Each end judges every record, either way, by one model of the order rules
(`src/rules.rs`):

- Streams are numbered from 1, each the previous plus one, and never
  reused. A reconnect is a new end and starts from 1 again.
- The relay answers Open stream with Stream opened or Stream refused before
  anything else, and sends nothing after Stream ended or Stream refused.
- The hub monitor sends requests only after Stream opened and before its
  own Close stream, and nothing after Close stream.
- A request or Close stream on a stream that has ended may have crossed the
  ending on the wire: the relay ignores it. After Close stream, the hub
  monitor discards what the relay sends on that stream, except the Stream
  ended or Stream refused that ends it.
- The relay holds at most 128 advertised sessions, withdraws only one it
  holds, and sends Snapshot complete once.

An end asked to send a record out of turn refuses it. A record that arrives
out of turn, with a tag its receiver does not know or with a body that
breaks its layout is a protocol violation and ends the link.

Units go out stream 0 first, then one unit for each session stream in turn,
so a control record overtakes a large event. An established end sends at
most 8 units in one step. When it has more, it names the present moment as
its deadline: its caller steps it again once those are sent, and a record
queued meanwhile goes ahead of the rest.

Four points go beyond the letter of the protocol pages, each the stricter
or simpler reading:

- Every protocol violation ends the link, including one the pages make a
  stream violation. Ending only that stream comes with credit.
- Any request or Close stream on a stream that has ended is ignored, even
  one that could not have crossed the ending, such as a request on a
  refused stream. Telling them apart would mean remembering every ended
  stream for the life of the link.
- An Open stream for a client interface revision other than 1, and a Stream
  refused cause or Stream ended reason the pages do not list, break the
  layout. `OpenStream` has no revision field: it is always 1.
- The relay answers Open stream with Stream opened or Stream refused even
  after Close stream, so Stream ended always follows Stream opened.

## The Link-layer harness

`tests/support/mod.rs` is the harness of the Link layer of the
[monitor-link acceptance criteria](../../docs/for-humans/behaviour/monitor-link/acceptance.md#evidence-policy).
`Harness::new(hub, relay)` joins a hub monitor's end and a relay's end by two
in-memory byte streams, starting at time zero.

- `run()` steps each end once with `Tick`, the hub monitor's first, then
  carries bytes and closes between them until both are quiet. An end that
  sent bytes and named the present moment as its deadline is stepped again.
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

`tests/wire/mod.rs` holds the two test tools of the Fault layer, in memory:

- `OnPath` is the on-path proxy. It wraps the relay's end, so every batch of
  bytes either way passes through its `edit`, which may change, drop, hold or
  add bytes.
- `StandIn` is the link peer stand-in. It runs the opening exchange and the
  handshake against one real end, as a hub monitor or as a relay, then seals
  and opens whatever units a test asks for.

## Specification mapping

| Scenario | Evidence |
|---|---|
| ONE-01: the guard rejects a dependency outside the allowlist, a `cfg` naming a platform, the platform modules of `std` and the standard clocks | `xtask/tests/architecture.rs`: `link_code_keeps_its_rules`, `link_guard_rejects_each_sample_crate_for_its_own_rule`, `link_guard_accepts_a_sample_crate_that_keeps_every_rule`, `link_source_scan_sees_through_every_spelling` |
| ONE-02: time and randomness reach the link code only as inputs | the ONE-01 tests above, which reject every other source, and `tests/harness.rs`: `one_02_the_test_supplies_the_time_of_every_step`, `one_02_the_test_supplies_each_end_its_randomness` |
| REV-01: the opening request is 26 bytes and the reply 20, in the published layout; the ends use the highest revision they share; every connection negotiates from scratch | `tests/opening.rs`: the `rev_01_` tests |
| REV-03, as far as the opening exchange goes: edited revision lists leave the two ends with different transcripts or a failed dial, and neither end tries another revision | `tests/opening.rs`: the `rev_03_` tests |
| REV-03 on a link connection: the key exchange that follows an edit fails | `tests/handshake.rs`: `rev_03_an_edited_revision_list_fails_the_key_exchange`. A pairing connection is proven with pairing. |
| SEC-01: Noise `IK` over X25519, ChaCha20-Poly1305 and SHA-256; fresh ephemeral keys, so two connections share no ciphertext | `tests/handshake.rs`: the `sec_01_` tests and `sec_08_a_link_connection_gives_climon2s_frozen_bytes` |
| SEC-02: no altered, repeated or out-of-order unit is delivered, and the link ends | `tests/attacks.rs`: the `sec_02_` tests |
| SEC-03: a replayed initiation or connection links nothing and delivers nothing | `tests/attacks.rs`: the `sec_03_` tests |
| SEC-04, the opening half: a malformed, partial or late opening request is closed without a byte in reply, at 10 seconds at the latest | `tests/opening.rs`: the `sec_04_` tests |
| SEC-04, the rest: an unknown key gets only Close with "not paired", and a wrong relay learns nothing that identifies the hub monitor | `tests/attacks.rs`: the `sec_04_` tests |
| STR-02, the Link-layer parts: stream numbers rise from 1 and are never reused, a reconnect starts again from 1, the relay answers first and Stream ended is last and carries the run report | `tests/streams.rs`: the `str_02_` tests and the tests of crossing and discarding |
| STR-03: every unit is at most 16,384 bytes and one fragment of one record, control records overtake a large event, records arrive complete and in order, and each client interface record has the layout of its structure | `tests/encoding.rs` and `tests/streams.rs`: the `str_03_` tests. Content fixtures are not recorded yet. |
| ADV-02, the Link-layer part: a Session carries the ID, command, arguments, working directory, depth, creation time and release, and nothing more | `tests/advertisement.rs`: the `adv_02_` tests |
| ADV-05, the Link-layer part: a remote session is the relay's pinned key and the session ID | `tests/advertisement.rs`: `adv_05_a_remote_session_is_the_relays_key_and_the_session_id` |
| REV-04: an unknown record tag ends the link and is never skipped; unknown capability tokens are ignored, and link revision 1 sends none | `tests/encoding.rs`: `rev_04_an_unknown_record_tag_ends_the_link`; `tests/records.rs`: `rev_04_unknown_capability_tokens_are_ignored` |
| SEC-08: the published Noise vector, and climon2's frozen vector binding the opening exchange into the handshake | `src/noise.rs`: `sec_08_the_published_noise_ik_vector_gives_its_bytes`; `tests/handshake.rs`: `sec_08_a_link_connection_gives_climon2s_frozen_bytes` |
