# Monitor link: PAKE and secure-channel options with Rust implementations

Research for [Survey PAKE and secure-channel options with mature Rust implementations](https://github.com/jackgeek/climon2/issues/190),
a child of [Chart the monitor link specification](https://github.com/jackgeek/climon2/issues/189).

Surveyed 2026-11-14. This note reports facts and options. It decides nothing.

## How to read this note

- Every claim carries a source link. Sources are the specifications, the crates' own
  docs.rs pages and READMEs, RustSec, and the projects' own documentation.
- **[unverified]** marks a claim that could not be confirmed from a primary source in this
  session. Treat those as leads, not facts.
- Access limits in this session: `crates.io`, `github.com` API, and raw repository files
  were unreachable, so crate facts come from docs.rs (which mirrors the published crate
  source, README, licence, dependency list and release dates). Download counts, commit
  recency, open-issue triage and repository security policies were **not** checked.
  RFC 9807 and the CPace draft are long; only the first ~100k characters of RFC 9807 and
  the first ~160k of CPace draft -17 were read, and draft -21 (the current one) was read
  only at the datatracker status level.

## The two jobs

The map separates two problems, and no single surveyed construction covers both alone.

1. **PIN pairing.** Both sides hold only a short one-time PIN. They need a strong shared
   key, and an attacker on the wire must get no way to test PIN guesses offline. This is
   the job of a *balanced* PAKE (SPAKE2, CPace). OPAQUE is an *augmented* PAKE aimed at a
   different job (see below).
2. **Remembered pairing.** After pairing, each side holds a long-term key for the other.
   Reconnects need mutual authentication from those keys and no PIN. This is the job of a
   static-key authenticated key exchange (Noise `KK`/`IK`/`XX` with pinned keys, or a
   high-entropy PSK pattern).

The seam between them is how long-term keys get bound to the PAKE result: either exchange
static public keys inside a channel keyed by the PAKE output, or derive a 256-bit
high-entropy secret from the PAKE output and use it as a Noise PSK. Both are options; see
"Composition options".

## Summary table

| Construction | Kind | Standardisation | Rust crate (latest stable) | Audit of the Rust crate | Licence |
|---|---|---|---|---|---|
| SPAKE2 | balanced PAKE | RFC 9382, Informational, IRTF stream, Sept 2023; not the CFRG selection | `spake2` 0.4.0 (2023-07-28) | none; crate says so | MIT OR Apache-2.0 |
| CPace | balanced PAKE | `draft-irtf-cfrg-cpace-21`, in RFC Editor queue; CFRG's selected balanced PAKE | `pake-cpace` 0.1.7 (2023-12-15); `pakery-cpace` 0.6.2 (2026-10-26) | none stated for either | `pake-cpace`: not shown on docs.rs **[unverified]**; `pakery-cpace`: MIT OR Apache-2.0 |
| OPAQUE | augmented PAKE | RFC 9807, Informational, IRTF stream, July 2025; CFRG's selected augmented PAKE | `opaque-ke` 4.0.1 (2025-11-03) | NCC Group, June 2021, on v0.5.0; fixes in v1.2.0 | MIT OR Apache-2.0 |
| Noise (static-key and PSK patterns) | AKE / secure channel framework | Not an IETF document; spec revision 34, 2018-07-11, status "official/unstable" | `snow` 0.10.0 (2025-07-19) | none; crate says so | Apache-2.0 OR MIT |

## SPAKE2

### Standardisation

- Published as RFC 9382, an Informational RFC on the IRTF stream, September 2023. It is
  not an IETF standard. ([RFC 9382 on datatracker](https://datatracker.ietf.org/doc/rfc9382/))
- The RFC states SPAKE2 "was not selected as the result of the CFRG PAKE selection
  competition" and was published anyway because variants are already used in Kerberos and
  elsewhere. ([RFC 9382 §1](https://datatracker.ietf.org/doc/rfc9382/))
- The CFRG selection (April 2020) recommended CPace as the balanced PAKE and OPAQUE as the
  augmented PAKE. SPAKE2, SPEKE, CPace and J-PAKE were the balanced candidates.
  ([CFRG selection results slides](https://datatracker.ietf.org/meeting/interim-2020-cfrg-01/materials/slides-interim-2020-cfrg-01-sessa-results-of-the-pake-selection-process-00.pdf))

### Shape

- Two roles, A and B, using fixed group elements M and N. A symmetric variant with M = N
  exists and must be used when roles cannot be assigned.
  ([RFC 9382 §§3, 5](https://datatracker.ietf.org/doc/rfc9382/))
- Two public-value messages, then two key-confirmation MACs. The RFC makes confirmation
  mandatory: neither side may treat the run as complete before verifying the peer's MAC.
  ([RFC 9382 §§3.3, 4](https://datatracker.ietf.org/doc/rfc9382/))
- Party identities are mixed into the transcript. Omitting them is allowed only where
  identities are implicit; otherwise unknown-key-share attacks become possible.
  ([RFC 9382 §3](https://datatracker.ietf.org/doc/rfc9382/))

### Known weaknesses and caveats

- **Trusted setup in M and N.** The proof depends on nobody knowing the discrete logs of M
  and N. The RFC generates them by hashing seeds and offers per-user M and N as a variant.
  ([RFC 9382 §§5, 7, Appendix A](https://datatracker.ietf.org/doc/rfc9382/)) The CFRG
  selection slides list this single discrete-log relationship across all sessions as an
  open question for SPAKE2.
  ([CFRG selection results slides](https://datatracker.ietf.org/meeting/interim-2020-cfrg-01/materials/slides-interim-2020-cfrg-01-sessa-results-of-the-pake-selection-process-00.pdf))
  A consequence usually drawn from this (one discrete-log break, for example by a quantum
  computer, exposes every session's password to offline search) is **[unverified]** here.
- **No augmentation.** Both sides hold a password equivalent.
  ([RFC 9382 §7](https://datatracker.ietf.org/doc/rfc9382/)) For a one-time PIN this
  matches the map's design, since both sides know the PIN anyway.
- The RFC text read here does not state forward secrecy or a one-guess bound in so many
  words; its proof reference is Abdalla–Pointcheval 2005 under a gap Diffie–Hellman
  assumption. ([RFC 9382 §7](https://datatracker.ietf.org/doc/rfc9382/))
- Received points must be validated as group members, and scalar multiplication must be
  constant time. ([RFC 9382 §7](https://datatracker.ietf.org/doc/rfc9382/))

### Rust: `spake2` (RustCrypto/PAKEs)

- Latest stable 0.4.0, released 2023-07-28. A pre-release 0.5.0-pre.0 is dated 2026-01-25.
  No stable release in over three years.
  ([docs.rs/crate/spake2](https://docs.rs/crate/spake2/latest))
- Licence: MIT OR Apache-2.0. ([docs.rs/crate/spake2](https://docs.rs/crate/spake2/latest))
- Audit: none. The crate says it "has never received an independent third party audit for
  security and correctness" and "USE AT YOUR OWN RISK!".
  ([docs.rs/crate/spake2](https://docs.rs/crate/spake2/latest))
- Its own docs say the library is probably not constant time.
  ([spake2 lib.rs](https://docs.rs/crate/spake2/latest/source/src/lib.rs))
- **Not RFC 9382.** The docs cite `draft-irtf-cfrg-spake2-10` and claim compatibility with
  the author's `python-spake2`, with only `Ed25519Group` implemented. RFC 9382 is not
  mentioned, and the RFC's ciphersuite table and mandatory confirmation step are not what
  the crate implements. ([docs.rs/crate/spake2](https://docs.rs/crate/spake2/latest),
  [spake2 lib.rs](https://docs.rs/crate/spake2/latest/source/src/lib.rs))
- API: `start_a` / `start_b` / `start_symmetric`, then `finish`. A wrong password produces
  no error: the two sides simply derive different keys, so the application must add its
  own key confirmation. Messages are 33 bytes. State cannot be serialised.
  ([spake2 lib.rs](https://docs.rs/crate/spake2/latest/source/src/lib.rs))
- Depends on `curve25519-dalek` ^4, `hkdf`, `sha2`, `rand_core`.
  ([docs.rs/crate/spake2](https://docs.rs/crate/spake2/latest)) `curve25519-dalek` had a
  timing-variability advisory fixed in 4.1.3
  ([RUSTSEC-2024-0344](https://rustsec.org/advisories/RUSTSEC-2024-0344)); a `^4`
  requirement permits the fixed version but does not force it.
- RustSec advisories against `spake2` itself: none found by search **[unverified]** (the
  RustSec package page could not be fetched).
- A separate crate, `spake2-conflux` 0.6.0, also unaudited by its own statement, defaults
  to Ristretto and keeps Ed25519 for legacy interoperability; it also cites draft -10, not
  the RFC. ([spake2-conflux README](https://docs.rs/crate/spake2-conflux/latest/source/README.md))
  Its provenance and maintainers were not established **[unverified]**.

### How Magic Wormhole uses SPAKE2

Magic Wormhole is the closest deployed analogue of "short code typed by a human keys a
channel over an untrusted relay".

- Each client sends a `pake` message carrying its SPAKE2 element as soon as the mailbox
  opens, then computes the shared key from the peer's.
  ([client protocol](https://magic-wormhole.readthedocs.io/en/latest/client-protocol.html))
- No dedicated confirmation message exists. Confirmation is implicit: each side sends an
  encrypted `version` message, and successful decryption shows the peer used the same
  code. A decryption failure closes the wormhole with a "scary" mood and
  `WrongPasswordError`.
  ([client protocol](https://magic-wormhole.readthedocs.io/en/latest/client-protocol.html))
- Per-message keys come from HKDF-SHA256 over the SPAKE2 key with a purpose string that
  includes the side and phase; messages use NaCl SecretBox.
  ([client protocol](https://magic-wormhole.readthedocs.io/en/latest/client-protocol.html))
- A hash of the shared key is offered as a verifier that users may compare out of band.
  ([client protocol](https://magic-wormhole.readthedocs.io/en/latest/client-protocol.html))
- Default code entropy is 16 bits. The docs state an attacker gets a 1-in-65536 chance per
  use, codes are single use, and a guess that consumes the code makes the legitimate
  peer's attempt fail visibly.
  ([welcome page](https://magic-wormhole.readthedocs.io/en/latest/welcome.html))
- The Rust port `magic-wormhole` 0.8.1 (2026-09-24, EUPL-1.2) depends on `spake2` ^0.4.0
  and `crypto_secretbox`, so the `spake2` crate has at least one real deployment in this
  exact role. It also has optional `noise-protocol` / `noise-rust-crypto` dependencies
  (not `snow`). ([docs.rs/magic-wormhole](https://docs.rs/magic-wormhole))
- Which SPAKE2 variant (symmetric or A/B) and the exact password and identity inputs
  (nameplate, AppID) are not spelled out on the pages read **[unverified]**.
- Magic Wormhole has no remembered pairings: every transfer uses a fresh code. It offers no
  precedent for the map's long-term keys.

## CPace

### Standardisation

- `draft-irtf-cfrg-cpace-21`, dated April 2026, Informational, IRTF stream (CFRG). IRTF
  state: sent to the RFC Editor, IESG review completed; RFC Editor state: awaiting first
  editor. It has no RFC number yet.
  ([datatracker](https://datatracker.ietf.org/doc/draft-irtf-cfrg-cpace/))
- CPace is the balanced PAKE the CFRG selection recommended in 2020.
  ([CFRG selection results slides](https://datatracker.ietf.org/meeting/interim-2020-cfrg-01/materials/slides-interim-2020-cfrg-01-sessa-results-of-the-pake-selection-process-00.pdf))
- The details below come from draft -17 (December 2025). Differences between -17 and -21
  were not reviewed **[unverified]**.

### Shape

- One round: each side sends one public share plus optional associated data. Both an
  initiator-responder mode and a symmetric mode (either side may speak first) are defined.
  ([draft -17](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-17))
- The session generator is derived by hashing the password-related string, a channel
  identifier `CI`, and a session id `sid`. No fixed M/N elements exist, so the SPAKE2
  trusted-setup concern does not arise.
  ([draft -17](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-17))
- The draft states the bound directly: an adversary gets at most a single password guess
  per active interaction, and offline dictionary attacks are prevented.
  ([draft -17 §1](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-17))
- Recommended suites include `CPACE-X25519-SHA512` and a P-256 suite;
  `CPACE-RISTR255-SHA512` is also defined.
  ([draft -17](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-17))

### Known weaknesses and caveats

- **Key confirmation is not built in.** The application must add it. Without it CPace
  gives only weak forward security; the draft recommends explicit confirmation when
  perfect forward security is required.
  ([draft -17 §9.4](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-17))
- **Identities are the application's job.** Without party identifiers CPace keys with
  anyone holding the same password and `sid`, and relay or loop-back attacks become
  possible. The draft recommends putting identities in `CI`.
  ([draft -17 §9.1](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-17))
- `sid` values should not be reused, and ephemeral scalars must never be reused.
  ([draft -17 §9.9](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-17))
- Implementations must abort when the shared point is the neutral element, and must output
  only the derived key `ISK`, never the raw Diffie–Hellman result, which could enable an
  offline dictionary attack if leaked.
  ([draft -17](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-17))
- The password-to-point mapping must be constant time.
  ([draft -17 §9.10](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-17))
- The draft calls CPace "quantum-annoying": an adversary able to solve discrete logs still
  pays one such computation per password guess.
  ([draft -17 §9.12](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-17))
  With a short PIN that is a small multiplier, so this is a modest property here.
- Not yet an RFC, so test vectors and encodings could in principle still change in
  editing.

### Rust crates

No Rust CPace crate surveyed has a stated audit or an established deployment record.

- **`pake-cpace` 0.1.7** (2023-12-15; previous release 2020-11-30). Owner `jedisct1`,
  described as a port of the libsodium CPace implementation. Ristretto255 with SHA-512.
  Cites `draft-haase-cpace-01`, a pre-CFRG individual draft, so it does not track the
  current CFRG wire format. `no_std` and WebAssembly compatible. No audit statement.
  Licence not shown on the docs.rs page **[unverified]**.
  ([docs.rs/crate/pake-cpace](https://docs.rs/crate/pake-cpace/latest))
- **`pakery-cpace` 0.6.2** (2026-10-26; first release 2026-03-07, ten releases in eight
  months). Part of the `pakery` workspace, which also publishes SPAKE2 and OPAQUE crates.
  MIT OR Apache-2.0. Ristretto255 with SHA-512; claims validation against
  `draft-irtf-cfrg-cpace` test vectors, without naming the draft revision. Forbids unsafe
  code, uses `subtle` and `zeroize`. No audit mentioned. Single crates.io owner shown.
  ([docs.rs/crate/pakery-cpace](https://docs.rs/crate/pakery-cpace/0.1.0))
  Very young and API-unstable by its release cadence.
- A crate named `cpace` 0.1.0 exists; it was not examined **[unverified]**.
- RustSec advisories for these crates: not checked **[unverified]**.

## OPAQUE

### Standardisation

- RFC 9807, Informational, IRTF stream, July 2025, representing CFRG consensus. It is the
  augmented PAKE the CFRG selection recommended.
  ([RFC 9807](https://www.rfc-editor.org/rfc/rfc9807.html),
  [CFRG selection results slides](https://datatracker.ietf.org/meeting/interim-2020-cfrg-01/materials/slides-interim-2020-cfrg-01-sessa-results-of-the-pake-selection-process-00.pdf))

### Shape and fit

- OPAQUE is a client-server protocol. Its distinguishing property is that the server never
  learns the password and a stolen server database resists pre-computation attacks.
  ([RFC 9807](https://www.rfc-editor.org/rfc/rfc9807.html))
- It needs a **registration** phase before any login, and registration "requires a
  server-authenticated channel with confidentiality and integrity".
  ([RFC 9807](https://www.rfc-editor.org/rfc/rfc9807.html))
- Login is three messages (KE1, KE2, KE3) built from an OPRF (RFC 9497), a 3DH key
  exchange and an application-chosen key-stretching function such as Argon2id.
  ([RFC 9807](https://www.rfc-editor.org/rfc/rfc9807.html))
- A server should count any login that never delivers KE3 as a failed attempt; the client
  learns success after KE2. ([RFC 9807](https://www.rfc-editor.org/rfc/rfc9807.html))

Facts that bear on fit, without deciding it:

- In monitor-link pairing no authenticated channel exists yet when the PIN is first used;
  establishing one is the purpose of the PIN. OPAQUE's registration precondition is
  therefore not met by the pairing ceremony as the map describes it.
- The relay mints and displays the PIN, so the "server never sees the password" property
  protects nothing here.
- The PIN is one-time and short-lived, so no long-lived password file exists to protect.
- OPAQUE carries more machinery (OPRF, key stretching, envelope) than a balanced PAKE.

### Known weaknesses and caveats

- A malicious server can still make online guesses against a client.
  ([RFC 9807](https://www.rfc-editor.org/rfc/rfc9807.html))
- Client enumeration needs deliberate fake-record handling.
  ([RFC 9807](https://www.rfc-editor.org/rfc/rfc9807.html))
- The formal security analysis section was beyond the portion of the RFC read
  **[unverified]**.

### Rust: `opaque-ke`

- Latest stable 4.0.1, released 2025-11-03; pre-releases continue (5.0.0-pre.0 on
  2026-11-13). Actively maintained. Repository `facebook/opaque-ke`. MSRV Rust 1.85.
  ([docs.rs/crate/opaque-ke](https://docs.rs/crate/opaque-ke/latest))
- Implements RFC 9807.
  ([opaque-ke README](https://docs.rs/crate/opaque-ke/4.0.0/source/README.md))
- Licence: MIT OR Apache-2.0.
  ([opaque-ke README](https://docs.rs/crate/opaque-ke/4.0.0/source/README.md))
- **Audit:** NCC Group, June 2021, sponsored by WhatsApp for end-to-end encrypted backups.
  It found issues in v0.5.0; fixes landed in v1.2.0.
  ([opaque-ke README](https://docs.rs/crate/opaque-ke/4.0.0/source/README.md))
  The audit predates the 2.x, 3.x and 4.x lines and the final RFC, so current code is
  substantially newer than what was reviewed. The NCC report itself was not read
  **[unverified beyond the README's summary]**.
- It is the only surveyed PAKE crate with any third-party audit.
- RustSec advisories against `opaque-ke`: none found by search **[unverified]**.

## Noise

### Standardisation

- The Noise Protocol Framework is a public specification by Trevor Perrin, revision 34,
  dated 2018-07-11, status "official/unstable". It is not an IETF or IRTF document.
  ([Noise spec](https://noiseprotocol.org/noise.html))

Noise is a framework for Diffie–Hellman handshakes plus a transport cipher state. It
provides the secure channel and static-key authentication. It is **not** a PAKE.

### PSK patterns and short PINs

- A `psk` token mixes a 32-byte secret into the handshake; modifiers `psk0`…`pskN` choose
  where. Listed patterns include `NNpsk0`, `KKpsk0`, `IKpsk2`, `XXpsk3`.
  ([Noise spec §9](https://noiseprotocol.org/noise.html))
- The spec requires that "Pre-shared symmetric keys must be secret values with 256 bits of
  entropy"; the fixed length is meant to discourage passwords.
  ([Noise spec §9](https://noiseprotocol.org/noise.html))
- The spec's author has stated on the Noise list that stretching a password into a PSK for
  `NNpsk0` lacks the PAKE property: a party who obtains a PSK-keyed ciphertext can test
  guesses offline. He sketched possible PAKE-style Noise patterns, which are not part of
  the specification.
  ([Noise mailing list, 2017-06-08](https://moderncrypto.org/mail-archive/noise/2017/001081.html))

So a PIN fed directly into a Noise PSK does **not** bound an attacker to one guess per
attempt. An attacker who completes or observes one handshake can search a short PIN space
offline in negligible time. A Noise PSK is sound only when it is a full-entropy value, for
example one derived from a PAKE output.

### Static-key patterns for remembered pairings

- `KK`: both static keys known in advance. `IK`: initiator knows the responder's key and
  sends its own encrypted in the first message. `XX`: both static keys are transmitted
  during the handshake and must be checked afterwards.
  ([Noise spec §7](https://noiseprotocol.org/noise.html))
- First-message weaknesses of `IK` and `KK`: the initiator's first payload is replayable,
  its sender authentication is vulnerable to key-compromise impersonation, and it has
  forward secrecy only against sender compromise. Later messages reach the stronger
  grades. ([Noise spec §7.7](https://noiseprotocol.org/noise.html))
- Identity hiding differs by pattern: `IK` encrypts the initiator's static key without
  forward secrecy; `XX` and `XK` protect it better.
  ([Noise spec §7.8](https://noiseprotocol.org/noise.html))
- Noise does not decide whether a static key is acceptable: "it's up to the application to
  determine whether the remote party's static public key is acceptable", with pinning and
  preconfigured key lists named as methods.
  ([Noise spec §14](https://noiseprotocol.org/noise.html))
  Remembered pairings are exactly a pinned-key list.
- The handshake hash can be exported for channel binding, for example to tie a separate
  authentication step to this session.
  ([Noise spec §11.2](https://noiseprotocol.org/noise.html))

### Other caveats

- Messages are at most 65535 bytes, so the application must frame larger payloads.
  ([Noise spec §3](https://noiseprotocol.org/noise.html))
- Stream truncation is not detected by Noise; the application must signal termination.
  ([Noise spec §14](https://noiseprotocol.org/noise.html))
- A static key pair should be used with one hash algorithm and not outside Noise.
  ([Noise spec §14](https://noiseprotocol.org/noise.html))
- At most 2^64 − 1 transport messages per direction; rekey changes the key, not the
  counter. ([Noise spec §§11.3, 14](https://noiseprotocol.org/noise.html))
- No post-quantum protection in the base patterns. A PSK adds some, a point the Noise list
  discusses; the specification's own wording on this was not captured **[unverified]**.

### Rust: `snow`

- Latest stable 0.10.0, released 2025-07-19; previous stable 0.9.6 on 2024-01-26.
  Releases since 2017. ([docs.rs/crate/snow](https://docs.rs/crate/snow/latest))
- Licence: Apache-2.0 OR MIT. ([docs.rs/crate/snow](https://docs.rs/crate/snow/latest))
- Audit: none. The README warns "This library has not received any formal audit."
  ([docs.rs/crate/snow](https://docs.rs/crate/snow/latest),
  [snow repository](https://github.com/mcginty/snow))
- Tracks Noise revision 34 and implements everything except the `fallback` modifier.
  ([snow repository](https://github.com/mcginty/snow))
- `Builder` exposes `psk`, `local_private_key`, `remote_public_key`, `prologue` and
  `generate_keypair`, covering both PSK and static-key patterns.
  ([snow Builder docs](https://docs.rs/snow/latest/snow/struct.Builder.html))
- Crypto backends: a default pure-Rust resolver (RustCrypto crates and
  `curve25519-dalek` ^4.1.3, which already requires the timing fix) or `ring` ^0.17.12.
  `no_std` with `alloc` is supported by the default resolver.
  ([docs.rs/crate/snow](https://docs.rs/crate/snow/latest),
  [snow lib.rs](https://docs.rs/crate/snow/latest/source/src/lib.rs))
- **Advisory:** RUSTSEC-2024-0011 (CVE-2024-58265), "Unauthenticated Nonce Increment in
  snow". An attacker able to inject packets could advance the stateful `TransportState`
  nonce with garbage and cause denial of service. Patched in 0.9.5.
  `StatelessTransportState` was unaffected.
  ([RUSTSEC-2024-0011](https://rustsec.org/advisories/RUSTSEC-2024-0011.html))
  This is directly relevant to a hostile-TCP threat model; 0.10.0 includes the fix.
- Deployment: the GitHub page shows 34 open issues and 14 open pull requests.
  ([snow repository](https://github.com/mcginty/snow)) Wide use by other projects (for
  example libp2p) is commonly reported but was not verified here **[unverified]**.
- Other Rust Noise crates exist (`noise-protocol` with `noise-rust-crypto`, used
  optionally by `magic-wormhole`); they were not surveyed **[unverified]**.

## How a short PIN bounds an attacker to one online guess

The mechanism is the same for SPAKE2 and CPace.

- The PIN never crosses the wire and no message is a deterministic function of the PIN
  alone that an eavesdropper can test. A passive observer learns nothing that confirms or
  refutes a PIN guess.
- An active attacker must commit to one PIN guess when it sends its protocol message. It
  learns whether the guess was right only from whether key confirmation (or the first
  encrypted message) verifies. One run tests one guess. CPace's draft states this as "at
  most one single password guess per active interaction"
  ([draft -17](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-17));
  Magic Wormhole's documentation states the same consequence for SPAKE2 with a 16-bit
  code ([welcome page](https://magic-wormhole.readthedocs.io/en/latest/welcome.html)).
- The protocol gives the *per-attempt* bound. The *total* bound comes from the
  application: the PAKE itself does not limit how many attempts happen. With an N-digit
  decimal PIN and at most k failed attempts before the invitation is void, an attacker's
  overall success chance is at most k / 10^N (for example 3 attempts on 6 digits is
  3 in 1,000,000; Magic Wormhole's default is 1 in 65,536 per code). This arithmetic is
  this note's, not a quoted source.
- The bound holds only if the implementation does the following, each of which the specs
  call out:
  - **Count every unfinished or failed run as a guess.** An attacker can abandon a run as
    soon as it has learnt the answer; RFC 9807 says the same of missing KE3 messages.
    ([RFC 9807](https://www.rfc-editor.org/rfc/rfc9807.html))
  - **Confirm the key before using it or revealing anything keyed by it.** RFC 9382
    mandates confirmation; CPace and the `spake2` crate leave it to the application.
  - **Order of confirmation matters.** Whichever side sends key-confirmation material
    first hands the other side a value that can be tested against PIN guesses offline *if
    that other side is the attacker and has not yet committed*. In SPAKE2 and CPace both
    public shares are committed before any confirmation is sent, so the receiver of the
    first confirmation still gets only the one guess it committed to. Sending
    PIN-dependent material before the peer's share arrives would break this. (General
    PAKE reasoning; not a quoted source.)
  - **Never reuse an ephemeral scalar or a PIN across runs**, and expire the PIN.
    ([draft -17 §9.9](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-17))
  - **Bind identities and roles** so a message cannot be relayed or reflected into
    another run. ([RFC 9382 §3](https://datatracker.ietf.org/doc/rfc9382/),
    [draft -17 §9.1](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-17))
  - **Validate received group elements** and keep secret-dependent operations constant
    time. A timing leak on a PIN-derived scalar is an offline oracle.
- An active attacker who guesses can also *burn* the invitation: failed attempts consume
  the small attempt budget, denying the legitimate hub. The map already accepts this by
  specifying expiry after a small number of failures. Magic Wormhole documents the same
  visible-failure behaviour.
  ([welcome page](https://magic-wormhole.readthedocs.io/en/latest/welcome.html))
- Since the map says a relay serves one hub at a time and the relay listens on a reachable
  port, concurrent pairing attempts are a design question: N parallel runs against one PIN
  are N guesses unless the relay serialises or counts them.

OPAQUE gives the same per-run bound for login but, as noted, presumes a prior
authenticated registration. A Noise PSK pattern keyed by a PIN gives **no** such bound.

## Composition options

Options for combining a pairing step with remembered pairings, listed without ranking.

1. **Balanced PAKE, then exchange static keys inside the resulting channel.** Run SPAKE2
   or CPace on the PIN, confirm the key, then each side sends its long-term static public
   key under the PAKE-derived key. Reconnects use a Noise static-key pattern (`KK`, or
   `IK`/`XX` with a pinned-key check). Long-term secrets are asymmetric; forgetting is
   deleting a pinned public key.
2. **Balanced PAKE output as a Noise PSK for the pairing handshake.** Derive 32 bytes from
   the PAKE output and run, for example, `XXpsk3`: the PSK authenticates the handshake in
   which static keys are exchanged, and the handshake hash binds everything. This
   satisfies the Noise 256-bit PSK rule because the PSK is PAKE output, not the PIN.
   Reconnects then use `KK` or `IK` with the learnt keys.
3. **Balanced PAKE, then a long-term symmetric secret.** Derive and store a 256-bit shared
   secret and reconnect with a PSK pattern such as `NNpsk0`. Simpler, but both sides hold
   the same secret, so compromise of either side allows impersonating either direction,
   and there is no key-compromise-impersonation resistance.
4. **Channel binding instead of nesting.** Run a Noise handshake that exchanges static
   keys unauthenticated (`XX`), then run the PAKE with the Noise handshake hash bound into
   its transcript (as `CI`/identity input) to authenticate that channel. The Noise spec
   names the handshake hash for this kind of use.
   ([Noise spec §11.2](https://noiseprotocol.org/noise.html))
5. **OPAQUE for pairing.** Possible only by treating PIN minting as registration, which
   needs an authenticated channel the pairing ceremony does not have. Listed for
   completeness.
6. **A PAKE inside Noise.** Only sketched on the Noise list; no specification and no
   `snow` support. ([Noise mailing list](https://moderncrypto.org/mail-archive/noise/2017/001081.html))

Cross-cutting facts for whichever option is chosen:

- Every surveyed Rust option for the *balanced PAKE* is unaudited. The choice is between an
  older, deployed, draft-era SPAKE2 crate (`spake2`, used by `magic-wormhole`), a stale
  CPace crate on a pre-CFRG draft (`pake-cpace`), and a months-old CPace crate tracking the
  CFRG draft (`pakery-cpace`).
- CPace has the stronger standing on paper (CFRG selection, no trusted setup, stated
  one-guess bound) and the weaker Rust implementation record. SPAKE2 is the reverse.
- `snow` is unaudited but long-lived, and has one low-severity denial-of-service advisory,
  fixed.
- Only `opaque-ke` has an audit, of code four major versions old, for a protocol whose
  shape does not match the pairing ceremony.
- None of the surveyed constructions is post-quantum.
- All surveyed crates are MIT/Apache-2.0 dual licensed where the licence was visible,
  except `pake-cpace` (not shown) and `magic-wormhole` (EUPL-1.2, a reference only, not a
  proposed dependency). climon2's own licence compatibility was not assessed here.

## Not covered

- SPAKE2+ (RFC 9383), SRP, J-PAKE, AuCPace, and TLS 1.3 with PAKE or external PSK
  (`draft-ietf-tls-pake`, `rustls`). These surfaced during the search but were outside the
  ticket's named set.
- Post-quantum or hybrid handshakes (`snow` lists optional `pqcrypto-kyber` dependencies;
  status not established).
- Formal-verification results for Noise patterns (Noise Explorer and similar).
- crates.io download counts, reverse dependencies, repository commit activity and
  maintainers' security policies.

## Sources

Specifications
- RFC 9382, SPAKE2: https://datatracker.ietf.org/doc/rfc9382/
- RFC 9807, OPAQUE: https://www.rfc-editor.org/rfc/rfc9807.html
- CPace draft status: https://datatracker.ietf.org/doc/draft-irtf-cfrg-cpace/
- CPace draft -17 text: https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-17
- CFRG PAKE selection results (April 2020): https://datatracker.ietf.org/meeting/interim-2020-cfrg-01/materials/slides-interim-2020-cfrg-01-sessa-results-of-the-pake-selection-process-00.pdf
- Noise Protocol Framework, revision 34: https://noiseprotocol.org/noise.html
- Noise list, Perrin on PAKE and PSK (2017): https://moderncrypto.org/mail-archive/noise/2017/001081.html

Crates
- `spake2`: https://docs.rs/crate/spake2/latest and https://docs.rs/crate/spake2/latest/source/src/lib.rs
- `spake2-conflux`: https://docs.rs/crate/spake2-conflux/latest/source/README.md
- `pake-cpace`: https://docs.rs/crate/pake-cpace/latest
- `pakery-cpace`: https://docs.rs/crate/pakery-cpace/0.1.0
- `opaque-ke`: https://docs.rs/crate/opaque-ke/latest and https://docs.rs/crate/opaque-ke/4.0.0/source/README.md
- `snow`: https://docs.rs/crate/snow/latest, https://docs.rs/snow/latest/snow/struct.Builder.html, https://github.com/mcginty/snow
- `magic-wormhole` (Rust): https://docs.rs/magic-wormhole

Advisories
- RUSTSEC-2024-0011 (`snow`): https://rustsec.org/advisories/RUSTSEC-2024-0011.html
- RUSTSEC-2024-0344 (`curve25519-dalek`): https://rustsec.org/advisories/RUSTSEC-2024-0344

Magic Wormhole
- Client protocol: https://magic-wormhole.readthedocs.io/en/latest/client-protocol.html
- Welcome and security notes: https://magic-wormhole.readthedocs.io/en/latest/welcome.html
