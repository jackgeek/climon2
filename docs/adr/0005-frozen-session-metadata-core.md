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

## Optional session origin

Decided in [Keep sessions created in another PID namespace or on another
machine](https://github.com/jackgeek/climon2/issues/133). A daemon's PID and
process identity mean something only on the machine, and in the PID
namespace, where it was created. A reader in a container without
`--pid=host` that shares the climon2 home, or on another machine sharing it
through a network home directory, can neither reach the session's socket nor
see its daemon, so it used to delete the metadata of live sessions.

Writers on every platform add `daemon_origin` without changing the frozen
fields or requiring a higher `format_version`. It may be absent or null in
older metadata, and is omitted when the writer cannot read where it runs.
When present, it is an object with two optional members, each omitted when
absent:

```json
{
  "machine_id": "8f2c4d1e9a7b4c3d8e6f5a4b3c2d1e0f",
  "pid_namespace": 4026531836
}
```

- `machine_id` is the machine's stable identity: on Linux the contents of
  systemd's `/etc/machine-id`, absent on a machine without one (as in many
  container images); on macOS the hardware UUID (`IOPlatformUUID`, from
  `gethostuuid`) as 32 lowercase hex digits; on Windows the `MachineGuid`
  value under `HKLM\SOFTWARE\Microsoft\Cryptography`. Readers accept any
  non-empty string of ASCII letters, digits and hyphens and only compare it.
- `pid_namespace` is Linux only: an unsigned 64-bit integer, the inode of the
  daemon's `/proc/self/ns/pid`. macOS and Windows have no PID namespaces.
- A malformed origin makes the record unreadable, with the same
  warning-and-retention behavior as malformed frozen fields.

A reader that finds a missing or refusing socket compares the record's origin
with its own, read the same way, before it looks at the daemon's identity:

- **Another origin** (any difference in either member, including one present
  on one side only): the session is uncertain, created elsewhere, and is never
  deleted, whatever its PID and identity look like here.
- **The same origin:** the identity check above decides, as before. A machine
  ID survives reboots, and a host's initial PID namespace keeps its inode
  across them, so a record from this machine before a reboot is still proven
  stale by its boot ID (Linux) or start time (macOS, Windows).
- **No origin in the record** (older writers): the identity check decides, as
  before.
- **A reader that cannot read its own origin:** the identity is unknown; the
  session is retained with a warning.

The boot ID was rejected as the machine identity because it changes on every
reboot, so records from before a reboot would never be cleaned up. Windows
records its origin too: a climon2 home on a network share can be read from
another Windows machine, and a Windows reader would otherwise check a PID
from another machine. The PID namespace inode alone was rejected because every
host's initial namespace has the same inode, so it cannot tell machines apart.

Known limits: two machines without a machine ID, or two clones with the same
one, look like the same machine and behave as before. A container's PID
namespace ends with the container, and one created later gets another, so
metadata left by a container that has gone is kept until someone deletes it.
Nothing proves such a namespace gone.

**Presentation.** A session created elsewhere is uncertain, like any other
session that is neither proven live nor proven stale. `list` shows only live
sessions, so it does not show one, and `attach` and `kill` cannot select it:
they fail with 125 as for any session that does not match. Nothing deletes or
signals it. A session created elsewhere whose socket this reader can reach (a
container that shares `/tmp` with the host) answers the hello and is live as
usual. The monitor treats it as any other uncertain session.

## Considered options

- **Readers reject a `format_version` they don't know:** rejected. After an upgrade, `list` would hide live sessions from the other version.
- **A separate metadata directory or file per format version:** rejected. Every reader would have to know every layout, which is the same compatibility burden spread over more places.
- **Freezing only the fields needed for probing:** rejected. Every field already has a reader in some other version (`list` display and `--json`, the stale check, finding the client), so there was nothing to gain.

## Consequences

- A metadata file that can't be read (bad JSON, a frozen field missing or of the wrong type, or a session ID that doesn't match the filename) is hidden from `list`, logged as a warning, and never deleted.
- The stale check compares native process identity, so PID reuse cannot keep
  stale metadata on disk forever. macOS and Windows compare
  `daemon_start_time`; Linux requires the optional stable identity above.
  Only a refused socket connection plus proven dead identity, checked where
  the daemon was created, permits cleanup, and a non-socket path is never
  unlinked as a socket. This is the tenth platform seam (ADR 0003).
- Golden metadata fixtures in `climon2-core` can only be added to, never edited. The current reader must parse all of them, which is a release gate. From the second release onwards, a packaged end-to-end gate installs the previous release next to the new one and checks that each lists the other's sessions.
