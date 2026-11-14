# climon2-discovery

One [discovery pass](../../GLOSSARY.md#sessions) over a climon2 home. This is
the CLI's own session discovery, moved into a shipped library (#77, planned in
[#59](https://github.com/jackgeek/climon2/issues/59)) so that `climon2 list`,
`attach` and `kill`, and later the monitor, share one implementation. It depends
only on `climon2-core` and `climon2-platform` (ADR 0004) and, like the platform
seams, supports macOS and x86-64 GNU/Linux; it has no OS-specific code of its
own.

```rust
pub fn discover(home: &HomeLayout, options: Options) -> io::Result<Pass>
```

`home` is the `climon2-core` layout of a resolved, canonical climon2 home, as
the CLI computes it; the layout names its sessions directory and each session's
metadata file. A pass:

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
| `Stale(metadata, Cleanup)` | Definitively stale: the socket is missing or refuses (`ENOENT`, `ECONNREFUSED`) and the recorded daemon identity provably no longer runs (ADR 0005): on Linux the boot ID and start ticks of `daemon_linux_identity`, never the epoch start time, and metadata without that identity is `IdentityUnknown`; on macOS the PID and start time. | `Retained`, `Removed` or `Failed(io::Error)` |

`Options::stale` is the only cleanup switch. With `StaleMetadata::Remove`, each
stale session's metadata file and then its socket are deleted, and files that
are already gone count as removed. A socket path that exists but is not a
socket is never unlinked as one: the cleanup fails before anything is deleted,
and the session is `Stale(metadata, Cleanup::Failed(_))`. Only CLI callers
choose `Remove`. With
`StaleMetadata::Retain`, stale sessions are reported and nothing is deleted;
monitor discovery never deletes metadata. Uncertain sessions and invalid
metadata are never cleaned up.

The library never prints or logs: its probe threads emit nothing, and every
diagnostic is returned as data. Callers own scheduling, presentation and
logging. The CLI logs, on its own thread and under its own target, exactly the
events it logged before the extraction, mapped from these outcomes and invalid
entries ([CLI README](../climon2-cli/README.md)).

## Incremental passes (#106)

The monitor runs bounded, incremental and fair passes instead:

```rust
pub fn Continuation::pass(&mut self, home: &HomeLayout, scan: &Scan, report: impl FnMut(Batch)) -> Progress
```

- A `Continuation` remembers where the next pass starts. A pass reaches the
  `.pty` names after that point in byte order, then wraps to the names up to
  it, so an entry added behind the pass's position waits for the next pass,
  and an entry removed before it is reached is simply not reported.
- It works one batch at a time: each lists the directory again, keeping only
  the next `Scan::batch_records` names in a bounded heap, so no pass holds the
  whole directory. It reads them in order until the next file would take the
  batch past `Scan::batch_bytes` (its first entry is always read), applies
  `Scan::limits`, and probes the valid entries with at most `Scan::probes`
  frozen hello probes in flight, each under the two-second deadline.
- `report` receives each `Batch` (`sessions` newest created first, then
  ascending session ID, and `invalid`) as soon as it is probed.
- `Scan::pass_time` runs from the pass's start. Once it has passed, no probe
  and no further batch starts, as soon as the pass has made progress
  (reported an entry or started a probe), so every pass makes progress, even
  over invalid metadata alone. The pass then ends
  `Progress::Incomplete(Incomplete::Deadline)` and the next continues after
  the last entry it reported. A sessions directory that cannot be listed ends
  it `Progress::Incomplete(Incomplete::Inaccessible(error))`. Otherwise it is
  `Progress::Complete`, possibly with no batch: a completed empty pass.
- It always retains stale metadata: monitor discovery never deletes anything.

`discovery.stall_ms` is not applied yet: probes are bounded by their own
deadline, and a stalled listing or read cannot be interrupted.

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
| DSC-06 Contract case: a nested session and a session controlled elsewhere, in independent resolved homes, are live in their own home's pass only, with their hello capabilities, state and depth; stale metadata is removed only when the caller opts in | `independent_homes_each_discover_their_own_nested_and_controlled_sessions` |
| DSC-06 Contract case (structured pass outcomes, no presentation) with DSC-04 Contract cases for one pass: mismatched hello IDs, failed probes, uncertain disappearance and malformed or unreadable metadata are structured outcomes and never cleaned up; a failed stale cleanup is reported; no pass prints | `every_outcome_is_structured_and_no_pass_prints` |
| DSC-06 (CLI part): `list`, `attach` and `kill` discovery, cleanup and logging are unchanged after the extraction | the `climon2-cli` tests that cite it ([CLI README](../climon2-cli/README.md#specification-mapping)): in `tests/list.rs` `malformed_metadata_is_hidden_warned_and_never_deleted`, `stale_cleanup_requires_connection_refusal_and_proven_dead_identity`, `uncertain_linux_daemon_identity_is_warned_and_metadata_is_retained` (Linux), `already_missing_stale_socket_does_not_report_cleanup_failure`, `gone_pid_is_stale_but_wrong_or_malformed_hello_never_is`, `list_args_fit_the_real_stdout_terminal_width`, `live_daemon_list_preserves_the_controlling_session_and_json_arguments`, `failed_hello_retains_artifacts_under_a_single_two_second_deadline`, `stalled_partial_hello_does_not_block_past_the_discovery_deadline`, `stale_metadata_cannot_unlink_a_regular_file_as_a_socket`, `frozen_probes_are_concurrent_and_sort_newest_first`, `malformed_hello_extensions_are_logged_and_the_session_stays_live` and `every_probe_diagnostic_is_logged_once_with_its_errno`; in `tests/attach.rs` `a_selector_must_match_exactly_one_live_session`, `a_session_without_attach_capabilities_is_refused`, `a_session_cannot_attach_to_itself`, `attach_needs_a_terminal`, `a_session_of_an_uninstalled_version_names_that_version`, `kill_selects_exactly_one_live_session`, `kill_refuses_an_older_session_without_capabilities`, `kill_of_an_uninstalled_version_names_that_version_and_ignores_config`, `attach_validates_only_the_client_configuration`; in `tests/exec.rs` `packaged_client_can_detach_and_reattach_to_a_live_session`, `kill_terminates_a_detached_session_silently_after_cleanup`, `kill_while_attached_gives_the_client_the_signal_status`, `attach_overwrites_an_inherited_kill_envelope`, `an_unconfirmed_kill_fails_at_the_deadline_and_reaps_only_its_helper` |
| DSC-04 Contract case: oversized and too deeply nested metadata, at and one past each limit | `metadata_limits_accept_their_bound_and_reject_one_past_it` |
| DSC-04 Contract case: a silent daemon is uncertain only after the frozen two-second hello deadline, and kept even when cleanup is requested | `a_silent_daemon_is_uncertain_after_the_two_second_hello_deadline` |
| DSC-04 Contract case (uncertain disappearance): a regular file at a stale session's socket path is never deleted, and neither is its metadata; Linux reports the failed cleanup, macOS the unreachable path | `a_regular_file_at_the_socket_path_is_never_deleted` |
| DSC-04 Contract case (uncertain disappearance), Linux: only the recorded boot ID and start ticks prove a session stale; a live identity with any start time, and a missing identity, are uncertain and kept even when cleanup is requested | `linux_daemon_identity_alone_proves_a_session_stale` |
| DSC-04 Contract case (inaccessible discovery): a sessions directory that cannot be listed fails the pass with the listing error instead of reporting an empty home, and nothing is deleted | `a_sessions_directory_that_cannot_be_listed_fails_the_pass` |
| DSC-01 Contract case (discovery order): newest created first, then ascending full session ID; a home without sessions is an empty pass | `sessions_are_newest_created_first_with_ascending_session_id_ties` |

| DSC-03 and DSC-04 Contract cases (incremental passes): every entry once, in batches bounded by records and by bytes at and one byte short of the bound; complete versus completed empty passes; nothing deleted | `incremental_passes_report_every_entry_once_in_bounded_batches` |
| DSC-03 Contract case (fair continuation while the directory changes) | `a_pass_continues_through_directory_changes_and_the_next_pass_reaches_what_it_missed` |
| DSC-04 Contract and Fault cases (slow and dead probes, incomplete discovery) with DSC-03 fair continuation | `a_pass_deadline_reports_incomplete_progress_and_the_next_pass_continues_after_it` |
| DSC-04 Contract case (bounded passes) with DSC-03 fair continuation: the deadline binds a pass that finds only invalid metadata | `a_pass_deadline_bounds_a_pass_that_finds_only_invalid_metadata` |
| DSC-04 Contract case (bounded probes) | `probes_are_bounded_by_the_scans_concurrency`, `probes_run_concurrently_up_to_the_bound` |
| DSC-04 Contract case (inaccessible discovery, incremental) | `an_unlistable_sessions_directory_makes_the_pass_incomplete` |

The monitor's own DSC-01, DSC-03, DSC-04 and DSC-06 cases are mapped in the
[monitor README](../climon2-monitor/README.md#specification-mapping).
