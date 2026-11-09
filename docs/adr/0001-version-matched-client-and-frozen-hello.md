# Version-matched client and frozen hello handshake

**[Monitor](../../GLOSSARY.md#sessions) planning:** the
[ownership decision](https://github.com/jackgeek/climon2/issues/58)
requires in-process, version-matched
[client libraries](../../GLOSSARY.md#sessions) with one client thread per
[live session](../../GLOSSARY.md#sessions). The [daemon-client decision](https://github.com/jackgeek/climon2/issues/67)
now specifies an agreed [portable wire revision 1](../protocol/daemon-client/core-v1.md),
not implemented. The private post-hello description
below records current behavior, not a constraint on that planned contract.

The planned contract intentionally reopens the private-only boundary: future
monitor forwarding needs a transport-independent core while exact-release
client matching remains mandatory. Frozen hello required fields, limits and
unknown-optional-field behavior are unchanged. Additive advertisements and the
first role request validate support before effects, without another exchange.
There is no initial checksum. This does not implement forwarding or settle
remote routing, multiplexing, discovery or security.

The [#63 terminal amendment](../protocol/terminal-integration.md) adds optional
core [Focus `0x39`](../protocol/daemon-client/request-focus.md), negotiated as
`focus-v1` through existing additive capability advertisement and Open selection.
It is not a plugin extension, release-number inference, or change to frozen
hello required fields. Missing library/daemon focus support leaves sessions
usable with an explicit limitation. Monitor-mode presentation is canonically
serialized at effective session geometry; ordinary attach remains unchanged.
These additions are agreed, not implemented.

The later [lifecycle contract](../monitor-lifecycle.md) agrees automatic
exponential passive recovery, prior-instance cleanup/join before replacement,
concurrent shutdown and bounded whole-process fail-stop. Accepted Leave,
closure, final outcome and local cleanup/join remain separate evidence; no
indeterminate work, including Kill, is replayed.
`climon2 exec --headless <command> [args...]` is implemented: it returns after
daemon-confirmed creation and permits zero clients for the session's entire
lifetime, including after later attachment loss, until completion or Kill.
Existing CLI kill is preserved and tested with headless sessions. The CLI hands
the headless policy to the daemon it starts, which is always of its own
version, as a private startup argument (`--lifetime`). Frozen hello and wire
schemas are unchanged: a headless session without attachments advertises the
existing `detached` lifecycle. Ordinary leave policy and native crash exposure
are unchanged. [Numerical budgets](../monitor-operational-limits.md) and
[native declarations](../protocol/monitor-client/core-v1.md) are now agreed;
the [acceptance matrix](../monitor-acceptance.md) is agreed in #62.
Executable conformance of the other plans remains future work.

A [session daemon](../../GLOSSARY.md#sessions) is only ever reached by the client from its own version. Session metadata records the daemon's version folder, and the CLI runs `<climon2 executable home>/<version>/climon2-client`, even when a different version is current. This keeps the protocol after hello version-specific, with no backward compatibility to maintain, and lets upgrades install alongside running sessions without breaking them. For the same reason, the hello handshake (framing and fields) is frozen forever: it is the one exchange that crosses versions, so any version of `climon2 list` can tell whether any daemon is live.

## Frozen layout

- **[Frame](../protocol/TERMS.md#frame):** a [big-endian](../protocol/TERMS.md#big-endian) `u32` length, then a 1-byte message type, then the body. The length covers type and body and must be between 1 and 1 MiB (1,048,576); any other length makes the reader close the connection.
- **Hello:** type `0x01` in both directions with [JSON](../protocol/TERMS.md#json) bodies, where unknown fields are ignored. The client speaks first with `{"protocol":1}`, and the daemon replies `{"session_id":"<guid>","climon2_version":"<v>","protocol":<n>}`.
- **Deadline:** 2 s. The daemon closes a connection that hasn't sent a valid hello within 2 s of accept. A prober treats no full reply within 2 s of starting to connect as not live.
- **Growth:** the frame layout and existing hello fields never change, but a daemon may add optional fields to its hello reply, which every reader ignores if unknown. This is how any version's `list` can learn dynamic session state, because session metadata is write-once (ADR 0005).
- **Client command line:** `<climon2 executable home>/<version>/climon2-client attach --metadata <absolute path>`, and nothing else. Every other input (`CLIMON2_HOME`, `CLIMON2_LOG`) reaches the client through the environment, where unknown variables are ignored. Any version's CLI can therefore start any version's client executable. The planned monitor uses the separate library embedding interface instead.

## Considered options

- **Backward-compatible protocol across versions:** rejected. Every future change would carry negotiation and compatibility tests.
- **Refuse sessions from other versions:** rejected. An upgrade would strand live sessions.

## Consequences

- Uninstalling a version that still has live sessions makes them unreachable. The CLI exits with the climon2 error code and names the missing version.
- The executable home is derived from the CLI's own canonicalised path, so version folders must stay siblings in the install layout.
- `climon2 exec` resolves its client through the metadata version from the first release, so this path is tested before attach exists.
- A future monitor loads each session's version-matched client library directly.
  Its cross-release embedding interface is specified separately from the
  daemon-client wire contract; neither interface is implemented by this ADR.
