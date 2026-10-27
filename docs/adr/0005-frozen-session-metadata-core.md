# Frozen session metadata core

Apart from the hello handshake, session metadata is the only data read across climon2 versions (ADR 0001). Any version of `climon2 list` must read every version's metadata to show, probe and clean up sessions, and the CLI uses it to find the version-matched client. So the location and all eleven fields are frozen forever: their names, JSON types and meanings. The format may only grow. Readers ignore unknown fields and accept any `format_version` of 1 or higher. The version number only tells a reader which of the later, optional fields to expect.

## Frozen layout

- **Location:** `<climon2 home>/sessions/<guid>.pty`. The directory is 0700 and the file 0600. Files are written to a temp name and then renamed, and the temp name never matches `<guid>.pty`. Readers only consider files whose stem parses as a GUID and whose extension is `.pty`.
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
  - `created_at`: RFC 3339 UTC with milliseconds.
  - `climon2_version`: SemVer, equal to the version folder name.
- `command`, `args` and `cwd` are for display only. Bytes that aren't valid UTF-8 are replaced with U+FFFD.
- **Write-once:** only the owning daemon writes or deletes the file, except that `list` may delete stale metadata. State that changes during a session comes from the daemon over the protocol, never from metadata.

## Considered options

- **Readers reject a `format_version` they don't know:** rejected. After an upgrade, `list` would hide live sessions from the other version.
- **A separate metadata directory or file per format version:** rejected. Every reader would have to know every layout, which is the same compatibility burden spread over more places.
- **Freezing only the fields needed for probing:** rejected. Every field already has a reader in some other version (`list` display and `--json`, the stale check, finding the client), so there was nothing to gain.

## Consequences

- A metadata file that can't be read (bad JSON, a frozen field missing or of the wrong type, or a session ID that doesn't match the filename) is hidden from `list`, logged as a warning, and never deleted.
- The stale check compares `daemon_start_time` with the start time of the live process, so a reused PID doesn't keep stale metadata on disk forever. This adds a tenth platform seam (ADR 0003).
- Golden metadata fixtures in `climon2-core` can only be added to, never edited. The current reader must parse all of them, which is a release gate. From the second release onwards, a packaged end-to-end gate installs the previous release next to the new one and checks that each lists the other's sessions.
