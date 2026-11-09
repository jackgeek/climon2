# climon2-discovery

One [discovery pass](../../GLOSSARY.md#sessions) over a climon2 home. This is
the CLI's own session discovery, moved into a shipped library (#77, planned in
[#59](https://github.com/jackgeek/climon2/issues/59)) so that `climon2 list`,
`attach` and `kill`, and later the monitor, share one implementation. It depends
only on `climon2-core` and `climon2-platform` (ADR 0004) and, like the platform
seams, supports macOS only.

```rust
pub fn discover(home: &Path, options: Options) -> io::Result<Pass>
```

`home` is a resolved, canonical climon2 home, as the CLI computes it. A pass:

1. Lists `sessions/` once. A missing directory is an empty pass. Any other
   listing error is returned, and that and a panicked probe are the only
   errors. Only `.pty` entries are [session metadata](../../GLOSSARY.md#sessions);
   other names are ignored.
2. Reads each entry within the caller's `MetadataLimits`, then applies the
   frozen validation of ADR 0005: a session-ID file name, the frozen fields and a
   matching `session_id`. Entries that fail are `Pass::invalid`, each with its
   path and an `InvalidReason` (`Malformed`, `Unreadable`, `TooLarge` or
   `TooDeep`), in no particular order. They are never deleted.
3. [Probes](../../GLOSSARY.md#sessions) every valid entry concurrently, one
   thread per entry, with the frozen hello `{"protocol":1}` under one absolute
   two-second deadline and nonblocking I/O.
4. Returns `Pass::sessions`, one `Outcome` per valid entry, newest created
   first, with ascending full session ID breaking ties. Before the extraction,
   tied sessions kept the directory's unspecified listing order.

| Outcome | When | Diagnostics |
|---|---|---|
| `Live(LiveSession)` | The daemon answered the hello with this session's ID. | `metadata` and the hello `extensions`; `state()` only when `state-v1` is advertised with a valid snapshot |
| `Uncertain(metadata, Uncertainty)` | Uncertain or unavailable: neither proven live nor proven stale. Never cleaned up. | `Mismatched`, `Unanswered(io::Error)`, `Unreachable(io::Error)`, `DaemonAlive` or `IdentityUnknown(io::Error)` |
| `Stale(metadata, Cleanup)` | Definitively stale: the socket is missing or refuses (`ENOENT`, `ECONNREFUSED`) and the recorded daemon PID and start time no longer match a process. | `Retained`, `Removed` or `Failed(io::Error)` |

`Options::stale` is the only cleanup switch. With `StaleMetadata::Remove`, each
stale session's metadata file and then its socket are deleted, and files that
are already gone count as removed. Only CLI callers choose it. With
`StaleMetadata::Retain`, stale sessions are reported and nothing is deleted;
monitor discovery never deletes metadata. Uncertain sessions and invalid
metadata are never cleaned up.

The library never prints or logs. Callers own scheduling, presentation and
logging. The CLI logs on its own thread exactly what it logged before the
extraction: `invalid session metadata` once per invalid entry, then
`malformed hello extensions` with `session=` for each live session whose
capabilities or state are malformed.

Bounded probe concurrency, incremental passes that report incomplete progress,
and fair continuation come with the monitor's discovery (#106).

## Metadata limits (decision for #77)

The file-size and parse-depth limits are caller parameters, not constants, so
that the CLI stays byte-for-byte unchanged while the monitor bounds its reads:

- The CLI passes `MetadataLimits::UNBOUNDED` (`file_bytes: None, json_depth:
  None`), its behaviour before the extraction: whole files are read, nested as
  deeply as the JSON parser accepts. serde_json skips unknown members at any
  depth, so there is no depth limit in practice.
- The monitor will pass the `monitor.limits` discovery values
  `metadata_file_bytes` (262144 by default) and `metadata_depth` (32).

A bounded read stops one byte past `file_bytes`, so an oversized file is never
read whole. A file of exactly `file_bytes` bytes is accepted; one byte more is
`TooLarge`. `json_depth` counts nested JSON objects and arrays, with the outer
object as 1, and ignores brackets inside strings. Nesting of exactly
`json_depth` is accepted; one level more is `TooDeep`.

## Specification tests

`tests/discovery.rs` drives the public `discover` with real session daemons for
the DSC-06 Contract case, and otherwise with synthetic metadata and raw-socket
hello peers.

| Scenario | Named test |
|---|---|
| DSC-06 Contract case: a nested session and a session controlled elsewhere, in independent resolved homes, are live in their own home's pass only; stale metadata is removed only when the caller opts in | `independent_homes_each_discover_their_own_nested_and_controlled_sessions` |
| DSC-06: CLI `list`, `attach` and `kill` discovery is unchanged after the extraction | the unchanged `climon2-cli` tests in `tests/list.rs`, `tests/attach.rs` and `tests/exec.rs` |
| DSC-04 Contract cases for one pass: mismatched hello IDs, failed probes, uncertain disappearance and malformed or unreadable metadata are structured outcomes and never cleaned up; a failed stale cleanup is reported; no pass prints | `every_outcome_is_structured_and_no_pass_prints` |
| DSC-04 Contract case: oversized and too deeply nested metadata, at and one past each limit | `metadata_limits_accept_their_bound_and_reject_one_past_it` |
| Shared discovery order: newest created first, then ascending full session ID; a home without sessions is an empty pass | `sessions_are_newest_created_first_with_ascending_session_id_ties` |

The monitor's own DSC-01, DSC-03, DSC-04 and DSC-06 cases arrive with #106.
