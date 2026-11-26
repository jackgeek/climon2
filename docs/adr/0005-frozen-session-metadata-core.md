# Frozen session metadata core

In the current implementation, apart from the [hello handshake](../../GLOSSARY.md#sessions), [session metadata](../../GLOSSARY.md#sessions) is the only data read across climon2 versions (ADR 0001). Any version of `c2 list` must read every version's metadata to show, probe and clean up [sessions](../../GLOSSARY.md#sessions), and the CLI uses it to find the version-matched [client](../../GLOSSARY.md#sessions). So the location and all eleven fields are frozen forever: their names, [JSON](../for-humans/protocol/TERMS.md#json) types and meanings. The format may only grow. Readers ignore unknown fields and accept any `format_version` of 1 or higher. The version number only tells a reader which of the later, optional fields to expect.

## Frozen layout

The agreed [native embedding](../for-humans/protocol/library-embedding.md) and
[daemon wire revision 1](../for-humans/protocol/daemon-client/README.md) extend the
cross-release data boundaries; the client library, the session daemon and,
since [#106](https://github.com/jackgeek/climon2/issues/106), the monitor
implement them. They do not change this frozen write-once core.
Dynamic title, status, episodes and progress come from daemon communication,
not metadata rewrites. Neither interface is implemented by this record.

- **Location:** `<climon2 home>/sessions/<guid>.pty`. The directory is 0700 and the file 0600. Files are written to a temp name and then renamed, and the temp name never matches `<guid>.pty`. Readers only consider files whose stem parses as a [GUID](../for-humans/protocol/TERMS.md#guid) and whose extension is `.pty`.
- **Fields:**
  - `format_version`: u32.
  - `session_id`: lowercase hyphenated GUID, equal to the filename stem.
  - `socket_path`: absolute path.
  - `daemon_pid`: u32.
  - `daemon_start_time`: u64, microseconds since the Unix epoch, from the platform's process-start-time seam.
  - `command`: string, as typed.
  - `args`: array of strings.
  - `cwd`: absolute path.
  - `depth`: u32, at least 1.
  - `created_at`: RFC 3339 [UTC](../for-humans/protocol/TERMS.md#utc) with milliseconds.
  - `climon2_version`: SemVer, equal to the version folder name.
- `command`, `args` and `cwd` are for display only. [Bytes](../for-humans/protocol/TERMS.md#byte) that aren't valid [UTF-8](../for-humans/protocol/TERMS.md#unicode-and-utf-8) are replaced with U+FFFD.
- **Write-once:** only the owning daemon writes or deletes the file, except that `list` may delete stale metadata. State that changes during a session comes from the daemon over the protocol, never from metadata.

## Optional Linux process identity

Linux writers add `daemon_linux_identity` without changing the eleven frozen
fields or requiring a higher `format_version`. It is omitted by macOS writers
and may be absent or null in older metadata. When present, it is an object:

```json
{
  "boot_id": "d56f4c51-46d4-48b3-aeb5-68aa95558389",
  "start_ticks": 12345
}
```

`boot_id` is the canonical lowercase hyphenated boot GUID from
`/proc/sys/kernel/random/boot_id`. `start_ticks` is an unsigned 64-bit integer
from field 22 of `/proc/<daemon_pid>/stat`, in kernel clock ticks since boot.
Both values must match the current non-zombie daemon for its PID to retain
the same identity. Inspection failures are not evidence of death.
Malformed known identity fields make the record unreadable, with the same
warning-and-retention behavior as malformed frozen fields.

Linux still writes `daemon_start_time` in epoch microseconds for compatibility,
but does not use it to authorize stale deletion: `/proc/stat`'s boot epoch can
change after a wall-clock adjustment. If stable identity is missing, Linux
can still probe, display and attach to the session, but refuses stale cleanup,
warns, and retains metadata. Old readers continue to ignore the optional field.
A climon2 home shared across PID namespaces or machines is refused (ADR 0011).

## Optional session origin

Decided in [Keep sessions created in another PID namespace or on another
machine](https://github.com/jackgeek/climon2/issues/133). Writers on every
platform add `daemon_origin`, where the daemon runs, for diagnostics, without
changing the frozen fields or requiring a higher `format_version`. It may be
absent or null in older metadata, and is omitted when the writer cannot read
where it runs. When present, it is an object with two optional members, each
omitted when absent:

```json
{
  "machine_id": "8f2c4d1e9a7b4c3d8e6f5a4b3c2d1e0f",
  "pid_namespace": 4026531836
}
```

- `machine_id` is the machine's stable identity: on Linux the contents of
  systemd's `/etc/machine-id`, absent when there is none (as in many
  container images) or when it is the transient one systemd mounts from
  `/run/machine-id`; on macOS the hardware UUID (`IOPlatformUUID`, from
  `gethostuuid`) as 32 lowercase hex digits; on Windows the `MachineGuid`
  value under `HKLM\SOFTWARE\Microsoft\Cryptography`. Readers accept any
  non-empty string of ASCII letters, digits and hyphens and only compare it.
- `pid_namespace` is Linux only: an unsigned 64-bit integer, the inode of the
  daemon's `/proc/self/ns/pid`. macOS and Windows have no PID namespaces.
- A malformed origin makes the record unreadable, with the same
  warning-and-retention behavior as malformed frozen fields.

**Superseded in part:** sharing one climon2 home across machines or PID
namespaces is refused before any discovery runs ([ADR 0011](0011-one-home-one-host.md)),
so a reader sees records written by its own host, or, after `c2 home release
--force`, by the host it took the home from. A reader uses `daemon_origin`
only to keep the latter: a session whose origin is another host's is uncertain,
created elsewhere, and never cleaned up. A record without an origin is judged
by the identity check above, as before. climon2's own machine ID for a
transient Linux one was dropped with that decision.
## Considered options

- **Readers reject a `format_version` they don't know:** rejected. After an upgrade, `list` would hide live sessions from the other version.
- **A separate metadata directory or file per format version:** rejected. Every reader would have to know every layout, which is the same compatibility burden spread over more places.
- **Freezing only the fields needed for probing:** rejected. Every field already has a reader in some other version (`list` display and `--json`, the stale check, finding the client), so there was nothing to gain.

## Consequences

- A metadata file that can't be read (bad JSON, a frozen field missing or of the wrong type, or a session ID that doesn't match the filename) is hidden from `list`, logged as a warning, and never deleted.
- The stale check compares native process identity, so PID reuse cannot keep
  stale metadata on disk forever. macOS and Windows compare
  `daemon_start_time`; Linux requires the optional stable identity above.
  Only a refused socket connection plus proven dead identity permits cleanup,
  and a non-socket path is never
  unlinked as a socket. This is the tenth platform seam (ADR 0003).
- Golden metadata fixtures in `climon2-core` can only be added to, never edited. The current reader must parse all of them, which is a release gate. From the second release onwards, a packaged end-to-end gate installs the previous release next to the new one and checks that each lists the other's sessions.
