# climon2-link

The link code of [monitor links](../../GLOSSARY.md#monitor-links): what a
[hub monitor](../../GLOSSARY.md#monitor-links) and a
[relay](../../GLOSSARY.md#monitor-links) say to each other over a plain byte
stream ([ADR 0011](../../docs/adr/0011-monitor-library-and-client-embedding.md)).
Monitor links are planned. So far this crate holds only the interface every
later part is driven through and the test harness that drives it; it speaks
no part of the
[link protocol](../../docs/for-humans/protocol/monitor-link/README.md) yet,
and no executable links it.

## One implementation for every platform

The crate performs no input or output. It opens no socket, reads no file and
no clock, and draws no randomness of its own: its caller owns the byte stream
and hands the crate everything else. The hub monitor's side and the relay's
side therefore run the same code on every operating system, and a test can
decide the time and the randomness exactly.

The architecture guard (`xtask/tests/architecture.rs`) keeps it so. It fails
when this crate:

- depends on a crate outside its allowlist, whatever the kind of dependency.
  The allowlist is empty today. It never holds `climon2-platform`, an
  operating-system or socket crate, an async runtime or a source of
  randomness;
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
- `Output`: the `bytes` to send next and whether to `close` the byte stream
  after them. A caller never steps an end again once it asks to close.

## The Link-layer harness

`tests/support/mod.rs` is the harness of the Link layer of the
[monitor-link acceptance criteria](../../docs/for-humans/behaviour/monitor-link/acceptance.md#evidence-policy).
`Harness::new(hub, relay)` joins a hub monitor's end and a relay's end by two
in-memory byte streams, starting at time zero.

- `run()` steps each end once with `Tick`, the hub monitor's first, then
  carries bytes and closes between them until both are quiet.
- `advance(duration)` moves the supplied time on and runs the ends.
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
