# Version-matched client and frozen hello handshake

A session daemon is only ever reached by the client from its own version. Session metadata records the daemon's version folder, and the CLI runs `<climon2 executable home>/<version>/climon2-client`, even when a different version is current. This keeps the protocol after hello version-specific, with no backward compatibility to maintain, and lets upgrades install alongside running sessions without breaking them. For the same reason, the hello handshake (framing and fields) is frozen forever: it is the one exchange that crosses versions, so any version of `climon2 list` can tell whether any daemon is live.

## Frozen layout

- **Frame:** a big-endian `u32` length, then a 1-byte message type, then the body. The length covers type and body and must be between 1 and 1 MiB (1,048,576); any other length makes the reader close the connection.
- **Hello:** type `0x01` in both directions with JSON bodies, where unknown fields are ignored. The client speaks first with `{"protocol":1}`, and the daemon replies `{"session_id":"<guid>","climon2_version":"<v>","protocol":<n>}`.
- **Deadline:** 2 s. The daemon closes a connection that hasn't sent a valid hello within 2 s of accept. A prober treats no full reply within 2 s of starting to connect as not live.

## Considered options

- **Backward-compatible protocol across versions:** rejected. Every future change would carry negotiation and compatibility tests.
- **Refuse sessions from other versions:** rejected. An upgrade would strand live sessions.

## Consequences

- Uninstalling a version that still has live sessions makes them unreachable. The CLI exits with the climon2 error code and names the missing version.
- The executable home is derived from the CLI's own canonicalised path, so version folders must stay siblings in the install layout.
- `climon2 exec` resolves its client through the metadata version from the first release, so this path is tested before attach exists.
