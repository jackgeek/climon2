# Version-matched client and frozen hello handshake

A session daemon is only ever reached by the client from its own version. Session metadata records the daemon's version folder, and the CLI runs `<climon2 executable home>/<version>/climon2-client`, even when a different version is current. This keeps the protocol after hello version-specific, with no backward compatibility to maintain, and lets upgrades install alongside running sessions without breaking them. For the same reason, the hello handshake (framing and fields) is frozen forever: it is the one exchange that crosses versions, so any version of `climon2 list` can tell whether any daemon is live.

## Considered options

- **Backward-compatible protocol across versions:** rejected. Every future change would carry negotiation and compatibility tests.
- **Refuse sessions from other versions:** rejected. An upgrade would strand live sessions.

## Consequences

- Uninstalling a version that still has live sessions makes them unreachable. The CLI exits with the climon2 error code and names the missing version.
- The executable home is derived from the CLI's own canonicalised path, so version folders must stay siblings in the install layout.
- `climon2 exec` resolves its client through the metadata version from the first release, so this path is tested before attach exists.
