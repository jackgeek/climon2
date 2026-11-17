# climon2-e2e

The default macOS and GNU/Linux packaged acceptance tier (#38), using the public installer,
the PATH launcher, real sibling binaries and `climon2-test-term` PTYs.
There are no nested Cargo builds, ignored packaged tests, test retries, or shortened
production deadlines. A client discards keys typed during a restoration, which every
grant starts (a new attachment, take control or hand-back), and nothing on the screen
marks its end, so typed input is ordered by observed output:
before input that must reach the session, `type_until` retypes a harmless command
until its output (or echo) appears, and the input itself is then sent exactly once.
That orders the input; it does not retry an assertion. Where typing would only end
the command, the command instead waits for an acknowledgement file the test creates.
A command that cannot answer a harmless key, because it records every byte or any key
ends it, answers an acknowledgement file with output instead: created only once the
client shows restored output, the file's answer is live output, which a client writes
only after its restoration.
Each scenario has an isolated HOME and climon2 home;
normal completion checks owned processes, session metadata and runtime sockets.
Logs intentionally survive until the isolated home is dropped.

```sh
cargo test --workspace --locked
```

Cargo builds every workspace binary through its integration target. Tests find
siblings using `bin_path`; a missing binary fails rather than invoking Cargo.
Ordinary scenarios share only an immutable installer-built executable layout;
data homes and mutable installation scenarios never share state. Cleared child
environments preserve only the harness controls and inherited
`LLVM_PROFILE_FILE`, including for the installed instrumented executables.

Interactive shells are GNU bash 5.x only. `controlled_bash` uses
`CLIMON2_TEST_BASH`, else `/opt/homebrew/bin/bash`, else `/usr/local/bin/bash`,
or Linux `/bin/bash`, and never macOS `/bin/bash`; anything other than bash 5.x is rejected. A missing
bash 5 fails CI and is explicitly reported as a local skip. `/bin/bash` remains
on macOS only for non-interactive `-c` helpers and fixture scripts; `install.sh`
compatibility with `/bin/bash` 3.2 is tested in `xtask/tests/install_script.rs`.
Vim and less are the actual `/usr/bin` applications.

## Exact final archive

```sh
CLIMON2_E2E_ARCHIVE="$PWD/target/dist/climon2-0.1.0-macos-universal.tar.gz" \
  cargo test --workspace --locked
```

The value is an exact `.tar.gz` file, not a directory or download URL. Tests
validate its exact versioned payload, extract it, and execute **that archive's
`install --dir …`**. No archive binary is rebuilt or replaced with a Cargo
sibling. Fixtures remain test-only sibling binaries, not shipped programs.
The smoke scenario leaves `CLIMON2_LOG` unset and checks the actual installed
release's logs contain no DEBUG or TRACE records.

On Linux use `climon2-0.1.0-linux-x86_64-gnu.tar.gz` instead. Archive payload
validation requires the host platform's suffix; a Darwin archive cannot stand
in for Linux runtime qualification. See [distribution](../../docs/distribution.md).

Internal-only fault injections (including the CLI's `CLIMON2_CLI_HOLD`, the
daemon's `pty-write` and `pty-write-partial`, and the monitor's
`CLIMON2_MONITOR_FAULT=draw` and `poll`) and the explicitly synthetic client-invocation
fixture are debug-sibling scenarios, not applicable to an unmodified no-hooks
archive. All black-box terminal, lifecycle, missing-client, privacy, logging and
raw-socket cases run against the archive. A removed client is restored by
renaming the same archive binary, never by rebuilding/substituting it.

From release two, set `CLIMON2_E2E_PREVIOUS_ARCHIVE` to the actual published
previous release archive for that platform as well. The compatibility gate starts its real
client/session before installing the current archive into the same executable
home, then both CLIs list both versions and both clients deliver command exit
statuses. With no prior archive, the first-release gate explicitly reports that
no compatibility evidence was fabricated.

## Library layer

`tests/library.rs` loads the installed client library,
`<executable home>/<version>/libclimon2_client.dylib` on macOS or
`libclimon2_client.so` on Linux, through the test-only `climon2-library-host`
fixture. Given an executable home and a version, the host loads through the
shared validated loader (`climon2_platform::library`, #84), which opens the
library with `dlopen` (`RTLD_NOW | RTLD_LOCAL`), and then calls only the
bootstrap and the returned function table, never Rust calls, static linking or
a mock table. The host prints its observations as JSON, including the loader's
refusal, whether a second load shared the first, every loaded file named like
the client library, and its environment; the tests decide what is correct.
Each host run is bounded; one that outlives its deadline is killed, reaped and
failed. In archive mode the same tests load the exact archive's library, so
the CI archive jobs exercise both macOS architectures' slices and the Linux
library. The exported symbol set is a release check (`xtask`), not a
Library-layer test; the loader's Fault-layer cases are `climon2-platform`'s,
except the macOS decoy found through `DYLD_LIBRARY_PATH` without the bootstrap.

`tests/instance.rs` (#85) drives client instances through the same host's
scenarios (`climon2-library-host <home> <version> <scenario> <json>`), from the
threads the contract names: an owner thread creates, runs and destroys each
instance while the test's main thread shuts it down, drains and releases. The
host is the Library layer's instrumented host: caller startup memory is one
mapping it scrambles and unmaps as soon as `create` returns, an ownership
ledger shows how every handle and delivery was released, a timeline records
each call's thread, and probes compare the process's environment, working
directory, signals and standard streams and the notification descriptor.
On x86-64 GNU/Linux the host also counts the library's heap (#180): it
defines glibc's `malloc`, `calloc`, `realloc`, `free` and `posix_memalign`,
exported dynamically, so the loaded library binds to them. Each allocating
entry is a naked function that passes on its return address, and the host
records each block the library image's own code allocated until it is freed;
glibc's internal blocks are never the library's. macOS binds a library to its
system allocator directly, so there the tests check exact self-consistency
instead: each instance's own changes return exactly to their earlier values.
Expected values come from the native header and contract. Fault cases select
the hooked library's `CLIMON2_LIBRARY_FAULT` and return early in archive
mode, whose release library has no hooks; every other case runs against the
archive's library.

`tests/passive.rs`, `tests/passive_faults.rs` and `tests/passive_admission.rs`
(#93) drive passive monitor connections through the host's `script`
scenario, step by step from the threads the contract names: create, submit,
run, drain, observe, release, snapshot, shutdown and finish, each instance on
its own owner thread. Requests live in caller memory the host scrambles and
unmaps as soon as `submit` returns. `passive.rs` registers with real session
daemons started through the packaged CLI (`climon2 exec` of the generic
fixture, held at points by its `await-file` action) and reads their hello
`state` to show that a passive connection never acquires control. The Fault
cases use the host's clearly labelled fake daemon peer, an adversarial
fixture that serves scripted hello and daemon wire revision 1 traffic
(malformed, slow, bytewise, coalesced, oversized or truncated frames, lost or
late answers, a peer that closes mid-conversation) and records every frame
the library sent, so a replay or an early Open shows. Ordering uses trigger
files and observable completion, never sleeps; every peer wait is bounded and
an expired wait is recorded.

`tests/control.rs` and `tests/control_faults.rs` (#94) drive control and
display the same way: interactive Open, Acquire, Input, Resize, Ready and
Focus under the authority the host last drained, against real sessions
whose application (the fixture's `raw` and `echo-until` actions) writes
back every input byte unchanged, and against the fake peer for holds,
losses, mismatches and records out of authority.

`tests/status.rs` and `tests/status_faults.rs` (#95) drive status, Set
viewing and Acknowledge the same way. `status.rs` starts its real sessions
with the daemon's controllable status clock (`CLIMON2_DAEMON_CLOCK`, a test
hook): the host's `clock` step sets it and waits for the daemon to
acknowledge the turn that read it, so each status change happens at an
exact, hand-computed time. Release archives have no hooks, so those cases
return early in archive mode. The passive suites leave `status-v1` out of
their Opens, because a real daemon reports status on its own clock.

`tests/mixed.rs` (#97) runs two releases in one host process. Each test
installs, in its own executable home and with the real installer, the
[second-release fixture](../../GLOSSARY.md#testing) as release 0.0.97 in a
sibling version folder: this build's other executables with the test-only
`climon2-second-release` crate's library and session daemon. The library is
the client library source built with its own release identity and without
test hooks. The daemon is the daemon library behind a thin main that serves
only 0.0.97 and gives its session's command `CLIMON2_SECOND_RELEASE_DAEMON`;
each test session turns that into its title, so a library relays over
`title-v1` which release's daemon it is connected to. Each release's CLI
starts headless sessions served by its own folder's daemon. The host loads
both releases' libraries through the shared loader (`script` input
`releases`; each `create` step names its `release`) and runs instances of
both at the same time. Releases ship no private dependencies, so
release-relative private-dependency resolution (LIB-01) is not exercised.
The Fault case returns early in archive mode.

## Session continuity scenarios

Detach and reattach, takeover and hand-back, the placeholder, kill, listing
state, restoration and release are packaged scenarios driven through controlled
bash 5 outer terminals (section 6.1 of
`docs/agents/session-continuity-plan.reviewed.gao.reviewed.md`). Each asserts
the actual exit statuses and ends with `assert_clean()`. Ordering uses
acknowledgement files and observable completion, never sleeps.

## Specification mapping

Each test also has a specification doc comment in `tests/packaged.rs`, for
session continuity (plan section 6.1) in `tests/continuity.rs`, or for headless
sessions in `tests/headless.rs`.

| Requirement | Named packaged test |
|---|---|
| #16/#21.6/#38 outer/inner controlled bash, arithmetic 42, second PTY list exactly one, outer status 7, empty list and leaks | `installer_built_bash_smoke_lists_one_session_and_preserves_exit_seven` |
| #16/#29 exact literal/empty argv, cwd, whole environment, real initial size/resize/status; #25 TRACE argv/input/environment privacy; continuity 6.1.15 `TERM`/`COLORTERM` overrides, otherwise inherited environment | `installed_generic_fixture_preserves_runtime_contract_and_log_privacy` |
| #16/#29 real nested bash, both GUIDs and depths, independent exit statuses | `installed_nested_bash_sessions_are_both_listed_and_route_exit_status` |
| #16/#20 alternate screen, cursor query replies, SGR mouse, bracketed paste, resize redraw, fixture's final screen survives | `installed_tui_roundtrips_queries_mouse_paste_and_resize_without_reset` |
| #16/#21.10 real vim edits/saves | `installed_real_vim_edits_and_saves` |
| #16/#21.10 real less navigation/resize | `installed_real_less_navigates_and_resizes` |
| #19/#21.7 owned client SIGKILL, real five-second daemon grace/group kill | `killed_installed_client_uses_daemon_leave_policy_and_leaves_nothing` |
| #19/#20 owned command SIGKILL/status 137/descendants | `killed_installed_command_returns_137_and_cleans_descendants` |
| #19/#25/#26 owned daemon SIGKILL, lost-connection/log suffix, real list stale cleanup | `killed_installed_daemon_reports_logs_and_list_reclaims_stale_session` |
| #19/#21.7 named pre-command panic/never-ready, real five-second startup deadline | `debug_hook_startup_panic_and_never_ready_report_and_clean_up` |
| #25/#28 headers, general/session linking, CLI/client PID continuity, daemon parent PID, TRACE/WARN propagation | `installed_log_headers_links_and_filters_survive_exec_handoffs` |
| #20 unrelated malformed/oversized raw peers and pending hello do not starve installed client/list | `installed_session_survives_raw_socket_abuse_and_pending_hello` |
| ADR 0001/#19/#25 missing version-matched client after Ready, version/log diagnostic, startup owner cleanup | `missing_installed_client_after_ready_is_diagnostic_and_cleans_command` |
| #21.17–20 crash mid-install/recovery while real client remains usable | `debug_hook_mid_install_crash_recovers_without_stranding_live_client` |
| ADRs 0001/0002/#28 controlled side-by-side versions, exact frozen client argv, home/filter environment, current handoff | `controlled_versions_route_their_own_frozen_client_across_current_handoff` |
| ADRs 0001/0002/0005/#26/#39 actual previous/current archive compatibility, no fabricated previous release | `actual_previous_release_remains_live_across_current_archive_install` |
| #19/#25 client panic after exec, diagnostic/log suffix, actual daemon leave policy | `debug_hook_client_panic_after_exec_logs_and_uses_daemon_leave_policy` |
| #25/#39 actual release default INFO-or-above | smoke's `assert_archive_default_levels` in archive mode |

### Library layer (`tests/library.rs`, #75, #84)

| Scenario | Named test |
|---|---|
| LIB-01 Library: absolute exact-release load; release text is the version folder name; one stable description; requirements present and within the default `monitor.limits` reservations; exactly the relayed families, every revision-1 family since #98; a second load of the version shares the first, and only that file is loaded | `installed_library_describes_revision_one_with_its_version_identity` |
| LIB-01 Library: with copies named by `current`, in the working directory and on ambient search paths (`LD_LIBRARY_PATH`, `DYLD_FALLBACK_LIBRARY_PATH`), the absolute path maps the requested file and only it (the system loader's lookup, which no library change can alter); the library answers from its own image with its own folder's release, not `current`'s | `installed_library_loads_by_absolute_version_path_whatever_current_cwd_and_search_paths` |
| LIB-02 Library: a version folder holding a copy that describes another release is refused as the wrong release, naming the compiled-in release; the copy stays loaded and the release it describes is not substituted; a folder without a library is a missing library and nothing loads | `installed_library_of_another_release_or_none_is_refused_without_substitution` |
| LIB-01 Library, Linux: with `LD_PRELOAD`, `LD_AUDIT`, `LD_LIBRARY_PATH` and `GLIBC_TUNABLES` set, the library loads and all four stay as they were | `installed_library_loads_under_the_users_loader_environment` |
| LIB-01 Library, macOS (#84 owner decision): a copy found through `DYLD_LIBRARY_PATH` is refused as a substituted image, naming both files, and stays loaded | `installed_library_substituted_through_dyld_library_path_is_refused` |
| LIB-01 Fault, macOS (#84 owner decision): an adversarial decoy without the bootstrap is refused, listing every loaded image of the library's file name in load order, the main executable included | `substitute_without_a_bootstrap_is_refused_naming_every_loaded_image_of_its_name` |
| LIB-02 Library: unsupported revisions rejected with a null description; null output storage | `installed_library_rejects_other_revisions_without_a_description` |
| DOC-01: all eleven table entries reject null handles and outputs by the frozen call rules, `submit`'s null request included | `installed_library_table_rejects_null_arguments` |

### Library layer: client instances (`tests/instance.rs`, #85)

| Scenario | Named test |
|---|---|
| LIB-03, LIB-04 and LIB-05 Library: create, run, shutdown, drain to terminal evidence, release, destroy and join; wrong-thread run and destroy, repeated run, busy destroy and release; terminal evidence apart from the cleanup report; every object released through the library | `installed_library_instance_runs_create_run_shutdown_drain_release_destroy_join` |
| LIB-03 Library: failed creates return no handles and leave no descriptor: null arguments, mismatched release, generation, native paths (non-UTF-8 kept, `PATH_MAX` bound) | `installed_library_create_rejects_invalid_identity_and_paths_without_handles` |
| LIB-03 Library: client keys, log path and `CLIMON2_LOG` filter | `installed_library_create_rejects_invalid_settings_and_diagnostics_without_handles` |
| LIB-03 Library: limits it cannot honour, each at its bound (`drain_payload_bytes` must hold a `metadata_record_bytes` record and a `chunk_bytes` Display), never raised or lowered; no descriptor left for the notification and waker descriptors or for the log file (`CM1_RESOURCE_LIMIT`, no log file), one more is enough | `installed_library_create_rejects_limits_it_cannot_honour_and_exhausted_descriptors` |
| LIB-03 Library and Fault: copies survive unmapped caller memory; lossless native paths; two instances' homes, settings and logs stay their own | `installed_library_instances_keep_their_own_copies_paths_settings_and_logs` |
| LIB-03 Library and Fault: environment, working directory, signals and standard streams unchanged | `installed_library_leaves_environment_cwd_signals_and_streams_unchanged` |
| LIB-04 Library and Fault, Linux `eventfd`: borrowed descriptor, counted once; a scheduled race in which polling sees the stream end while the handle is used before shutdown and after the end, every call succeeding with the same descriptor | `installed_library_notification_descriptor_is_borrowed_and_safe_to_poll_racing_the_end` |
| LIB-04 Fault: allocator pairing, a delivery charged until released, the release returning exactly what the drain charged | `installed_library_charges_a_delivery_until_it_is_released` |
| LIM-02 and LIB-03 Library, x86-64 GNU/Linux: `snapshot`'s current and peak allocation equal the host's own count of the library's heap, after create and once ended, one call at a time after a warm-up lifecycle on the same threads, with logging on (its dispatcher included) and off, and with a `joined-worker` whose `chunk_bytes` block sets the peak (the peaks differ by more than nothing and at most one thread handle, which the standard library allocates outside every allocator) | `installed_library_snapshot_counts_every_allocation_its_instance_holds` |
| LIM-02 and LIB-03 Library, x86-64 GNU/Linux: an instance that opens a passive monitor connection to a fake daemon peer, shuts down after its Response and drains to terminal evidence keeping its deliveries: current allocation equals the host's count with both records unreleased, and the peak the connection set equals the host's, with logging on and off | `installed_library_snapshot_counts_a_passive_connections_allocation` |
| LIB-03 Library: while `a` lives, a whole lifecycle of `b` leaves `a`'s current and peak allocation exactly unchanged | `installed_library_instances_never_change_each_others_allocation` |
| LIB-03 Library: successive instances on one owner thread whose startups differ only in the home's length differ by exactly that many bytes, current and peak | `installed_library_charges_successive_instances_only_for_their_own_differences` |
| LIB-04 Fault: the failure diagnostic only within `event_bytes`, at and one under the bound | `installed_library_queues_the_failure_diagnostic_only_within_event_bytes` |
| LIB-05 Fault: injected panic at the bootstrap and `create` | `installed_library_contains_a_panic_at_the_bootstrap_or_create` |
| LIB-05, LIB-04 and REC-01 Fault: injected panic at `submit`, `notification`, `snapshot`, `drain`, `shutdown` and `run` stops the instance, with no retry from `shutdown` on; a delivery held past terminal evidence alone keeps communication and destroy busy | `installed_library_contains_a_panic_in_a_running_instance_and_stops_it` |
| LIB-04 and LIB-05 Fault: injected panic at `release_delivery`, `release_communication`, `release_control` and `destroy` keeps the object | `installed_library_contains_a_panic_in_a_release_and_keeps_the_object` |
| LIB-05 Fault (`worker` hook): worker completion apart from terminal evidence | `installed_library_reports_worker_completion_apart_from_terminal_evidence` |
| LIB-06 Fault (`stall-cleanup` hook): stalled worker cleanup incomplete, never complete or destroyed | `installed_library_reports_stalled_worker_cleanup_as_incomplete_and_never_destroys` |

LIB-06 Library is not covered yet: without the fault hook this release starts
no worker that could stall. Open
([#93](https://github.com/jackgeek/climon2/issues/93)) brings the first.

### Library layer: passive monitor connections (`tests/passive*.rs`, #93)

| Scenario | Named test |
|---|---|
| CTL-01 Library (controlled elsewhere: hello `state` unchanged throughout), CTL-03 Library (Response, atomic Initial metadata, gap-free titles by value, empty and unset distinct), COM-01 Library (optional families intersect), COM-03 Library (copied before return, charged until executed), LIB-04 Library (kept deliveries unchanged until released) and EXT-01 Library (shutdown with live deliveries: one confirmed Leave, closure, outcome, cleanup and handle release reported apart) | `installed_library_registers_passively_while_controlled_elsewhere_and_relays_titles` (`passive.rs`) |
| CTL-01 and CTL-03 Library, detached session: never acquired, nothing attached | `installed_library_registration_never_acquires_a_detached_session` (`passive.rs`) |
| COM-03 and EXT-01 Library: explicit Leave answered `left`, second Open `invalid_state`, stale generation `stale_generation`, submit closed afterwards, no further Leave | `installed_library_explicit_leave_departs_once_and_is_answered` (`passive.rs`) |
| EXT-02 Library: shutdown's Leave to a stopped real daemon is never answered: `timeout` after `leave_ms`, Leave indeterminate, failed, no retry | `installed_library_reports_a_lost_leave_to_a_stopped_daemon_as_unconfirmed` (`passive.rs`) |
| COM-01 Library: daemon frames one byte over `wire_frame_bytes` rejected before any role, for good | `installed_library_rejects_a_daemon_whose_frames_exceed_wire_frame_bytes` (`passive.rs`) |
| COM-01 and COM-02 Library: an Open naming no family registers passively and selects none, an empty selection being null and zero | `installed_library_opens_naming_no_family_and_selects_none` (`passive.rs`) |
| LIB-05 Library and Fault: an instance stalled by its fake daemon never stops another registering with a real daemon; `submit`, `drain`, `snapshot` and `release_delivery` return at once | `installed_library_instance_stalled_by_its_daemon_never_stops_another` (`passive.rs`) |
| COM-01 and REC-01 Fault: missing support (frozen tokens only, no `monitor-v1`, malformed limits, an unadvertised required family), another session, release or protocol in hello or metadata, an unserved or unreadable socket or metadata, a reply that is not a hello, or none within `start_ms`: rejected before any Open is sent, with retry permanent or passive | `installed_library_rejects_an_open_the_session_cannot_honour_before_any_role` (`passive_faults.rs`) |
| COM-01 Fault: optional families intersect three ways (`progress-v1` offered, unadvertised `title-v1` and unrequested `focus-v1` not); a selection the request does not allow fails | `installed_library_selects_only_families_both_sides_support` (`passive_faults.rs`) |
| COM-02 Fault: invalid JSON, wrong direction, unknown type, zero, oversized and truncated frames, uncorrelated or mismatched responses, out-of-order or unselected records, a title or record one byte over its limit, and a daemon Fatal each fail the connection with their origin and code; nothing after the failing frame is delivered, and nothing stays counted once run returns | `installed_library_fails_the_connection_on_traffic_revision_one_forbids` (`passive_faults.rs`) |
| COM-03 and REC-01 Fault: deadlines answer `timeout` (indeterminate); lost answers fabricate nothing and leave unanswered IDs to the caller's ledger; failed and indeterminate Leaves; refused and failed Opens with their retry (transport loss passive, missing support permanent); no retry once the caller's Leave is sent; a close after an Initial metadata that knew the final result a clean stop (stopped, no error, retry none); best-effort diagnostics; nothing replayed; nothing outstanding once run returns | `installed_library_reports_lost_and_late_answers_truthfully_without_replay` (`passive_faults.rs`) |
| COM-01 and COM-03 Fault: a known final result in a rejection, charged its copies, with retry none | `installed_library_reports_a_known_final_result_and_never_retries` (`passive_faults.rs`) |
| EXT-01 and EXT-02 Fault: shutdown's Leave lost, late or rejected; a rejected explicit Leave keeps the registration; shutdown during an Open, or during hello (the Open `cancelled`, never sent); Leave before any Open or while another is unanswered | `installed_library_shutdown_leaves_once_and_reports_unconfirmed_departures` (`passive_faults.rs`) |
| LIB-04 and LIB-05 Fault: a panic inside a running connection leaves no descriptor or operation counted | `installed_library_panic_in_a_connection_leaves_nothing_counted` (`passive_faults.rs`) |
| LIB-05 Fault: the library never raises `SIGPIPE` in an embedder that left it at its default | `installed_library_never_raises_sigpipe_in_its_embedder` (`passive_faults.rs`) |
| COM-02 Fault: titles at `title_bytes` and records at `metadata_record_bytes` accepted whole; a signal command result mapped | `installed_library_accepts_records_exactly_at_their_limits` (`passive_faults.rs`) |
| COM-02 Library (unknown tags, nonzero reserved fields, inactive storage ignored) and COM-03 Library (invalid and closed rejections synchronous, no ID consumed, no buffer retained, a Kill's required list checked as an Open's, `cancelled` for work shutdown never ran) | `installed_library_admits_only_valid_requests_and_rejections_cost_nothing` (`passive_admission.rs`) |
| COM-03 Library: `request_records`, `request_bytes`, `operations`, `event_records` and `event_bytes` at the limit and one past; full rejections consume no ID | `installed_library_bounds_requests_by_every_queue_limit` (`passive_admission.rs`) |
| LIB-04 Library: one delivery allowed, `queue_full` while it is outstanding, its records unchanged and charged until released | `installed_library_holds_each_delivery_until_released` (`passive_admission.rs`) |
| LIB-04 Fault: deliveries stop before `drain_payload_bytes`, which always holds a whole record | `installed_library_batches_records_within_drain_payload_bytes` (`passive_admission.rs`) |
| COM-02 and LIB-04 Fault: frames split bytewise or coalesced; daemon records wait while the event queue is full, never dropped or overrunning it | `installed_library_holds_daemon_records_while_the_event_queue_is_full` (`passive_admission.rs`) |
| LIB-04 Fault: each record charged its header twice (the `cm1_record` and the library's copy) and the bytes it owns once | `installed_library_charges_each_record_its_header_and_the_bytes_it_owns` (`passive_admission.rs`) |

### Library layer: control and display (`tests/control*.rs`, #94)

`control.rs` runs real session daemons through the packaged CLI;
`control_faults.rs` the host's fake daemon peer. A script names the
authority the host drained last (`$period`, `$restoration`) or the one
before it.

| Scenario | Named test |
|---|---|
| COM-04 Library (Input `00 ff 41`, NUL and invalid UTF-8 reach the raw application, whose echo returns as exact Display bytes), COM-05 Library (Response, Initial metadata, attach grant, Restoration begin, Display, complete under one authority), COM-02/COM-03 Library acceptance of interactive Open, Ready and Input, EXT-01 Library (attached shutdown: release Display, Leave confirmed, attachment detached); with `chunk_bytes` 16, every Display at most 16 bytes and the stream healthy | `installed_library_attaches_and_relays_binary_input_and_display_unchanged` (`control.rs`) |
| CTL-05 Library: superseded restoration (`stale_restoration`), revoked (`not_controller`) and stale period (`stale_period`), new restorations and periods `not_ready` until Ready, no rejected byte reaching the application; Resize accepted, Acquire by an attachment, `stale_generation` never sent | `installed_library_stale_authority_never_re_enables_input` (`control.rs`) |
| FOC-01 Library: no duplicate focus, `not_ready` before readiness, stale period rejected, the new period resynchronized after Ready | `installed_library_focus_has_no_duplicates_and_resynchronizes_on_new_readiness` (`control.rs`) |
| CTL-02 Library: Acquire, no-change re-acquisition, switching between monitors (takeover under the exact held authority), the last acquisition winning, background control kept until another client's takeover | `installed_library_acquisition_switches_and_takeover_keep_the_last_accepted` (`control.rs`) |
| COM-05 and COM-06 Library: a command that exits while an attached instance controls: `CM1_FINAL` (exit 0, status unavailable), then the release Display and Control revoked `session_ending`, then the close: a clean stop, no error, retry none, Leave not required | `installed_library_relays_the_command_final_before_the_release_and_session_ending` (`control.rs`) |
| CTL-04 and EXT-01 Library (Leave racing a promotion, real daemon): the promoted registration's one Leave delivers the successor grant, its restoration and the release, then Leave confirmed with the attachment detached; the session stays detached | `installed_library_leave_racing_a_promotion_leaves_as_the_attachment` (`control.rs`) |
| CTL-04 and COM-05 Library: the newest registration promoted with a background successor grant and canonical restoration, Ready without selection, the next promoted after it; fresh periods per grant | `installed_library_promotes_the_newest_registration_with_a_canonical_restoration` (`control.rs`) |
| COM-02/COM-03 Library control cases, local rejection: malformed control requests `invalid_argument` (generation zero inside an authority, Input over `chunk_bytes` included), Input at `chunk_bytes`, at and one past `request_bytes` | `installed_library_rejects_malformed_control_requests_locally` (`control_faults.rs`) |
| COM-02/COM-03 Library control cases, `cancelled`: every control request queued behind an Open is cancelled by shutdown, never sent | `installed_library_cancels_queued_control_requests_on_shutdown` (`control_faults.rs`) |
| COM-02/COM-03 Library control cases, stale generation and state: `stale_generation`, `unsupported_capability` without `focus-v1`, `invalid_state` while leaving, `unsupported_limit` over the request frame ceiling, none sent; interactive Open without `monitor-v1` | `installed_library_rejects_stale_and_unsupported_control_requests_locally` (`control_faults.rs`) |
| COM-02/COM-03 Library control cases, `timeout` and lost responses for every command and interactive Open, never replayed | `installed_library_times_out_or_loses_control_requests_without_replay` (`control_faults.rs`) |
| COM-02 Library control cases, answers: each success's own result, daemon rejections, mismatched, partial and known-final answers `invalid_record` | `installed_library_maps_control_answers_and_refuses_mismatched_results` (`control_faults.rs`) |
| COM-03 and CTL-05 Library and Fault: empty, unheld and late Display, early or second grant, foreign, repeated or older begin or Geometry, unheld complete or revocation `invalid_record`; a Display at `chunk_bytes` one record, one past it two (`chunk_bytes` and one byte), both streams healthy | `installed_library_refuses_presentation_records_out_of_authority` (`control_faults.rs`) |
| CTL-04 Library and Fault (Leave racing a promotion), COM-05 Library and Fault (Command final first, Geometry, `session_ending` revocation, then the close as a clean stop with retry none, a fresh instance seeing only its own records) | `installed_library_resolves_a_promotion_racing_leave_and_fresh_attempts_stand_alone` (`control_faults.rs`) |
| COM-03 Library and Fault, Command final (minimal, #94): `CM1_FINAL` exit or signal with its name, status unavailable, the close a clean stop with retry none (inside a frame still `truncated_frame`; meeting a Leave, indeterminate); second, after a known result (agreeing or contradicting), early or with a status `invalid_record` | `installed_library_relays_one_command_final_after_the_initial_metadata` (`control_faults.rs`) |
| COM-03 Library and Fault, backpressure: a Display waiting for `event_bytes` room stops `operation_ms`; `delivery_stall_ms` fails the connection `delivery_stalled`, retry passive, the held-back Ready unanswered | `installed_library_fails_a_stalled_delivery_without_timing_out_its_commands` (`control_faults.rs`) |
| COM-03 Library and Fault: once waiting records are queued no stall deadline remains, and a Ready times out at `operation_ms` | `installed_library_times_commands_out_again_once_waiting_records_are_queued` (`control_faults.rs`) |
| LIB-04 Fault, control and screen records: each charged its header twice and only a Display's bytes beside it; Control revoked (`session_ending`) delivered with its reserved field zero | `installed_library_charges_presentation_records_only_the_display_bytes_they_own` (`control_faults.rs`) |

### Library layer: termination (`tests/termination*.rs`, #96)

| Scenario | Named test |
|---|---|
| COM-06 Library (exit zero, exit 143 and signal 15 from real daemons: one Command final each, of its own kind, status unavailable) and REC-01 Library (the daemon's close after it a clean stop, retry none) | `installed_library_relays_each_real_command_final_apart_from_signals` (`termination.rs`) |
| COM-03 Kill and DEL-02 Library (a Kill alongside a passive Open and a Kill-only instance, each on its own connection, answered OK with SIGHUP and complete cleanup only after the hang-up grace, the metadata and socket gone; a second Kill while the first is pending `invalid_state`, never sent), COM-06 Library (exactly one Command final of the same result) and REC-01 Library (retry none) | `installed_library_kills_a_real_session_only_after_termination_and_cleanup` (`termination.rs`) |
| COM-03 Kill, DEL-02 and REC-01 Library: on an opened role, a Kill whose hello a stopped daemon never answers is rejected `timeout` within `kill_ms`, unsent, with no diagnostic; the role stays open and its Leave is confirmed after the daemon resumes; the session is never terminated | `installed_library_keeps_a_real_role_when_its_kill_cannot_reach_the_daemon` (`termination.rs`) |
| REC-01 Library: transport loss (the real daemon killed) fails the role with `io_error`, retry passive; startup naming another session than its real metadata is `invalid_configuration`, permanent; a contained panic (`panic:receive`, hooked library only) is `internal_error`, retry passive | `installed_library_classifies_real_failures_for_retry` (`termination.rs`) |
| REC-01 Library: a stopped daemon fails an Open with `timeout`, retry passive; destroy busy while handles are held; a fresh instance and generation open only after destroy and join | `installed_library_recovers_only_through_a_fresh_instance_after_join` (`termination.rs`) |
| COM-06 and REC-01 Fault (beside #94's Command final cases in `control_faults.rs`): exit 0 and exit 143 as Command final records of their raw values, then a clean stop; a contradictory second Command final `invalid_record` (retry none); a close without final evidence (failed, passive) or meeting the caller's Leave (indeterminate) | `installed_library_relays_each_command_final_once_and_closure_invents_nothing` (`termination_faults.rs`) |
| COM-03 Kill, DEL-02, COM-06 and REC-01 Fault: a Kill-only instance's Kill sent once (wire ID 1, revision 1), `monitor-v1` not needed; confirmed, known-final, partial (known and unknown result), revisionless, lost, late (`kill_ms`), mismatched, uncorrelated, non-answer and Fatal answers with their retry; refused unsent for an unadvertised family, an oversized frame, a lost or silent hello; a second pending Kill `invalid_state`; `cancelled` before execution or after a hello shutdown overtook; a Kill under way answered after shutdown; never replayed, nothing counted once run returns | `installed_library_reports_each_kill_outcome_truthfully_and_never_replays_it` (`termination_faults.rs`) |
| COM-06 and COM-03 Fault: a partial termination on a failed Open, Leave or Acquire `invalid_record`, the request unanswered (the Leave indeterminate, retry none; the others retry passive) | `installed_library_refuses_a_partial_termination_on_any_other_request` (`termination_faults.rs`) |
| COM-03 Kill, DEL-02, COM-06 and REC-01 Fault, Kill on an opened role (the fake daemon's second connection): lost (`io_error`) and late (`timeout`) answers indeterminate, a partial termination (failed) and a known final result (rejected) carried, an unadvertised family and a lost hello rejected unsent, a second pending Kill `invalid_state`; never a diagnostic, the role reading on to its own Command final and clean stop, or to a close without final evidence (retry passive, or none once the partial termination knew the result); the auxiliary connection carries one Kill at most | `installed_library_keeps_an_opened_role_through_each_kill_failure` (`termination_faults.rs`) |

### Library layer: status, viewing and acknowledgement (`tests/status*.rs`, #95)

| Scenario | Named test |
|---|---|
| STA-07 Library (idle at the daemon's start in every Initial metadata, running at the first sample, needs_attention episode 1 ten seconds later; a fresh instance's Initial metadata keeps that episode and time; the final status only with Command final, completed, no Status after it), CTL-03 Library status case (the same status reaches a passive registration, an attachment revoked by takeover and a background controller) and STA-08 Library (viewing false from a background controller before Ready) | `installed_library_relays_status_to_every_role_and_keeps_its_identity_across_instances` (`status.rs`) |
| STA-08 Library: viewing true and Acknowledge `not_ready` before Ready, viewing false accepted during restoration; `not_viewing` until viewing true; viewing changed then unchanged; explicit no-change Acknowledge; Status acknowledged (same episode and time) before the changed success, then no change; viewing false leaves the status; `not_controller` after takeover; a new period starts not viewing; accepted Input clears acknowledged attention, its Status first. COM-03 Library: `stale_generation` locally | `installed_library_sets_viewing_and_acknowledges_attention_through_a_real_daemon` (`status.rs`) |
| COM-03 Library and Fault (viewing and acknowledgement cases): malformed requests `invalid_argument`, queued requests `cancelled` by shutdown; STA-08 Fault: `unsupported_capability` without `status-v1` and `stale_generation`, none sent | `installed_library_rejects_viewing_and_acknowledgement_it_cannot_send` (`status_faults.rs`) |
| STA-08 Library and Fault and COM-03 Library and Fault: wire bodies sent once; viewing and acknowledgement results mapped, no-change explicit, Status before a changed acknowledgement; daemon rejections (`stale_period`, `stale_restoration`, `not_ready`, `not_viewing`) kept; contradicting or mismatched results `invalid_record`; held answers `timeout` (indeterminate), closed connections no fabricated Response, nothing replayed | `installed_library_maps_viewing_and_acknowledgement_answers_without_replay` (`status_faults.rs`) |
| STA-07 Library and Fault, CTL-03 Fault: every state, episode and exact time mapped, the final status in `CM1_FINAL` (completed, failed) and in a knowing Initial metadata, a status record charged its record alone; early, unselected or post-final Status, availability against the selection, final status without the result or working status with it, Command final without status, final state or stray episode `invalid_record` | `installed_library_relays_status_records_by_the_reference_rules` (`status_faults.rs`) |

### Library layer: notifications and progress (`tests/notifications.rs`, #98)

| Scenario | Named test |
|---|---|
| STA-06 Library (a real daemon's OSC 9, OSC 99 and OSC 777 notifications as `CM1_NOTIFICATION` with source, identifier, title, body and episode, after the needs_attention Status; OSC 9;4 progress as `CM1_PROGRESS`, normal, indeterminate, then paused with the cached percentage, never requesting attention; progress in Initial metadata, none at first and retained for a later instance) and CTL-03 Library (notification and progress cases: the same records in the same order reach a passive registration, an attachment revoked by takeover and a background controller) | `installed_library_relays_notifications_and_progress_to_every_role` |
| STA-06 Library: exact native mapping, notification text exactly at `notification_title_bytes` and `notification_body_bytes` with multi-byte characters, a body alone, no episode; progress states and percentages, an unknown percentage absent; a queued notification charged its record and text, progress its record alone | `installed_library_maps_notifications_and_progress_exactly_at_their_bounds` |
| STA-06 Fault and LIM-01 Library: notification title or body one byte past its bound `resource_limit`, never truncated; a notification with neither title nor body, a notification or progress before the Initial metadata, progress without `progress-v1`, Initial metadata progress against the selection `invalid_record` | `installed_library_fails_the_connection_on_notification_and_progress_faults` |

### Library layer: mixed releases (`tests/mixed.rs`, #97)

| Scenario | Named test |
|---|---|
| LIB-01 Library (exact-version lookup and one shared load; private-dependency resolution not exercised, releases ship none): both releases installed; with `current`, the working directory, `LD_LIBRARY_PATH` and `DYLD_FALLBACK_LIBRARY_PATH` naming the other release, one process loads each by its exact version folder, in both orders; two instances of each register passively with a session of their own release at the same time, relay the title only their own release's daemon produces, and leave; a second load of each release shares the first, and exactly the two files are loaded once each | `mixed_releases_load_by_exact_version_and_run_side_by_side_whatever_current_cwd_and_search_paths` |
| LIB-01 Library (exact-release pairing): the second-release fixture's daemon, with its release compiled in, starts no session for a CLI of another release (folder 0.0.98) | `second_release_daemon_serves_only_its_compiled_in_release` |
| LIB-03 Library, identity separation: beside registered instances of both releases, a create expecting the other release fails `CM1_RELEASE_MISMATCH` without handles, and an Open of the other release's session, served by the other release's daemon, is refused `release_mismatch`, permanently, before any role | `mixed_release_instances_keep_their_own_release_identity_side_by_side` |
| LIB-03 Fault, fault containment: `panic:receive`, once per hooked image, cannot reach the second-release fixture's instance, which opens and receives its Initial metadata before the hooked release's instance is submitted; that instance then fails (`internal_error`) and the second-release fixture's instance leaves | `a_fault_in_one_release_never_reaches_the_other_releases_instances` |

### Session continuity (plan section 6.1)

| Scenario | Named packaged test (`tests/continuity.rs` unless noted) |
|---|---|
| 6.1.1 detach, list `detached`, reattach on an occupied host, prior host content kept, termios restored | `installed_session_detaches_and_reattaches_on_an_occupied_host` |
| 6.1.2 alternate-screen restoration, quitting reveals normal content | `installed_alternate_screen_is_reattached_and_quitting_shows_normal_content` |
| 6.1.3 shared session, one controller, take-control, controller size drives the PTY | `installed_clients_share_a_session_with_exactly_one_controller` |
| 6.1.4 client SIGKILL uses the daemon leave policy | `killed_installed_client_uses_daemon_leave_policy_and_leaves_nothing` (`tests/packaged.rs`) |
| 6.1.5 kill of detached and attached sessions | `installed_kill_ends_detached_and_attached_sessions` |
| 6.1.6 self-attach refused without disturbing the session | `installed_self_attach_is_refused_without_disturbing_the_session` |
| 6.1.7 invalid configuration refused before a session exists; rebound keys | `installed_invalid_config_is_refused_before_a_session_exists`, `installed_config_rebinds_the_client_keys` |
| 6.1.8 `scrollback.max_rows` bounds restored history with a notice; CFG-01 (exec case) the monitor's `history.max_rows` never changes it | `installed_scrollback_max_rows_bounds_restored_history` |
| 6.1.9 detached queries answered by the session, never forwarded to the host | `installed_session_answers_queries_without_the_host` |
| 6.1.10 encoded titles, OSC 8 links as plain text | `installed_titles_are_encoded_and_links_are_plain_text` |
| 6.1.11 detached session absorbs an 8 MiB write | `installed_detached_session_absorbs_a_large_write` |
| 6.1.11 host released (modes, termios) on detach, takeover, exit and kill | `installed_client_releases_the_host_on_every_ending` |
| 6.1.12 interrupted paste closed once on takeover, socket loss and readiness change | `installed_interrupted_paste_is_closed_once_without_stale_payload` |
| 6.1.13 large restoration complete, live output once and in order | `installed_large_restoration_is_complete_and_ordered` |
| 6.1.14 wide and tall hosts present the session rectangle | `installed_wide_host_presents_the_session_rectangle`, `installed_tall_host_scrolls_the_session_rectangle` |
| 6.1.15 `TERM`/`COLORTERM` overrides | `installed_generic_fixture_preserves_runtime_contract_and_log_privacy` (`tests/packaged.rs`) |
| 6.1.17 previous-release compatibility | `actual_previous_release_remains_live_across_current_archive_install` (`tests/packaged.rs`) |

### Monitor (#74, spec #73)

`tests/monitor.rs` drives the installed `c2 monitor`/`mon` (and the `climon2`
spellings) on macOS and Linux, from outer shells that title the host and record
termios around it. Each case checks the documented baseline, exact termios,
the untouched title, no OSC output and ordinary shell input afterwards. Cases
that depend on how keystrokes or pastes are read stop the monitor (SIGSTOP):
`wait_for_unread_input` shows when every byte sent has reached the terminal,
then, after SIGCONT, when the monitor has read them all in one read of their
own. The hang-up case completes its hang-up while the monitor is stopped.
A resize after such a read shows that the monitor acted on every byte, because
it handles each read before the next signal. Cases that do not test the key
introduction first save **Don't show again** in their climon2 home, as an
earlier run would, so they open on the empty state.

| Scenario | Named packaged test (`tests/monitor.rs`) |
|---|---|
| ENT-01 `c2 monitor`, `c2 mon`, `climon2 monitor` and `climon2 mon` open the same installed monitor through the launcher: top row, empty state, exact host modes, prefix+x exits 0 with the prefix read alone | `installed_monitor_and_mon_open_the_same_monitor` |
| INP-01 configured prefix hint and exit 0, null take-control key accepted; once read, Ctrl+C and the default prefix+x are not exits (SIGTERM then gives 143) | `installed_monitor_hints_and_exits_with_the_configured_prefix` |
| INP-01 null escape prefix and invalid keys refused before terminal changes | `installed_monitor_refuses_a_null_escape_prefix_and_invalid_keys` |
| CFG-01 unreadable and malformed JSONC, null, wrong type, duplicate, overflow, out-of-range and inconsistent settings: one diagnostic, 125, no terminal change | `installed_monitor_refuses_every_invalid_configuration_before_the_terminal_changes` |
| CFG-01 limits one byte short of the monitor's parser and one 500x200 session's documented current model and recent page (total), of the model and page (client budget) or recent page (history total, then per-session history budget): one diagnostic naming that limit, 125, no terminal change; at those limits the monitor opens and prefix+x exits 0 (#167) | `installed_monitor_refuses_limits_that_cannot_hold_the_largest_session_model` |
| CFG-01 `monitor.show_key_hints` false hides the empty state's hint, prefix+x still exits; unknown and unrelated settings ignored | `installed_monitor_without_key_hints_shows_none_and_still_exits` |
| CFG-01 no live reload: after an edit, a repaint keeps the started settings and the edited prefix, once read, does not exit (SIGTERM then gives 143) | `editing_the_configuration_while_the_monitor_runs_changes_nothing` |
| CFG-01 every effective setting in the monitor's general log | `installed_monitor_logs_its_effective_configuration` |
| INP-03 (empty monitor) pastes never exit, never complete a pending prefix and leave none pending | `pastes_never_exit_or_leave_a_pending_prefix` |
| Derived scenario: monitor argument refusal, usage and 2 | `installed_monitor_arguments_are_a_usage_error` |
| Derived scenario: monitor non-TTY stdin or stdout refusal | `installed_monitor_refuses_a_non_terminal_stdin_or_stdout` |
| Spec #73 entry: missing monitor fails with 125 naming the version | `missing_installed_monitor_fails_naming_its_version` |
| Spec #73 startup: no executable home outside a version folder | `a_monitor_outside_a_version_folder_refuses_to_start` |
| EXT-03 (Monitor, empty monitor) SIGINT/SIGTERM/SIGHUP give 130/143/129 | `signals_release_the_host_with_signal_style_status` |
| EXT-03 (Monitor, empty monitor) hung-up host input, read as its end or as EIO, releases the host before its one diagnostic, logged, 125 | `a_hung_up_host_input_releases_the_host_before_its_diagnostic`, `host_input_failing_with_eio_releases_the_host_before_its_diagnostic` |
| EXT-03 (Fault, empty monitor) render failure or failed wait for host input releases the host before its one diagnostic, 125 | `debug_hook_render_failure_restores_the_host_before_its_diagnostic`, `debug_hook_input_wait_failure_restores_the_host_before_its_diagnostic` |
| Spec #73 host terminal; TRM-04 (empty monitor): SIGWINCH repaints exactly at the new size without erasing or showing the cursor | `resizing_the_host_redraws_the_monitor_at_the_new_size` |
| Spec #73 exit: content-free monitor general log | `installed_monitor_logs_content_free_records_to_its_general_log` |
| HLP-01 every start with hints on and nothing saved opens the key introduction: the actual prefix with x, s and ?, an unchecked box; Enter continues, closing saves nothing, prefix+x exits from it | `the_key_introduction_opens_at_every_start_until_dont_show_again_is_saved` |
| HLP-01 hints off or the choice saved keep it closed; prefix+? opens it regardless, showing the saved choice, and is a no-op while open; a later version's preference file is honoured | `the_key_introduction_opens_on_prefix_question_mark_whatever_hints_and_the_saved_choice_say` |
| HLP-01 Space saves and reverses the choice per climon2 home: exact version 1 bytes, 0600 under umask 000, by rename, nothing left behind, a file already under the temporary name replaced; restarts and another home | `dont_show_again_is_saved_privately_and_atomically_per_home_and_reversible` |
| HLP-01 (derived scenario: a save interrupted before its rename) saving first removes the temporary files of processes no longer running, keeping those of running processes and any other file | `saving_removes_temporary_files_that_ended_processes_left_behind` |
| HLP-01 (derived scenario: unreadable or malformed preference file) not suppressed, reason under the box and logged, never rewritten implicitly | `an_unreadable_or_malformed_preference_file_shows_why_beside_the_introduction` |
| HLP-01 (Fault; derived scenario: unreadable or malformed preference file, key hints off) the introduction stays closed, the empty state shows the reason where the hint would be; prefix+? shows it under the box; a failing save shows its failure in both places, a successful one clears them | `without_key_hints_the_empty_state_shows_problems_with_the_saved_choice` |
| HLP-01 (Fault) a save that cannot create its temporary file or replace the preference file is reported and logged, saves nothing, leaves nothing | `a_failing_save_is_reported_in_the_key_introduction_and_saves_nothing` |
| HLP-02 (empty monitor) the key introduction owns typed keys, prefixed keys, reports and pastes, none acts; Enter closes, prefix+? reopens, prefix+x exits | `the_key_introduction_is_modal_and_prefix_x_still_exits_from_it` |
| HLP-02 (empty monitor) with Ctrl+[ as the prefix Escape cannot close the key introduction, prefix+x still exits | `with_escape_as_the_prefix_escape_cannot_close_the_key_introduction_and_prefix_x_still_exits` |
| HLP-02 (empty monitor) a doubled prefix reaches the key introduction as one literal prefix: with Ctrl+M as the prefix, Enter Enter continues, the prefix and Space reach nothing, prefix+x still exits | `with_enter_as_the_prefix_a_doubled_enter_continues_from_the_key_introduction` |

Not packaged, with lower-boundary evidence: resize before `Ready` (no hook
holds a packaged client between `Granted` and `Ready`; daemon
`resize_before_ready_replaces_the_restoration_period`)
and the ambiguous selector (random GUIDs; CLI
`a_selector_must_match_exactly_one_live_session`). Daemon raw-socket (6.2) and
client bridge (6.3) scenarios are mapped in those crates' test doc comments.

### Headless sessions (spec #73, acceptance matrix HDL-01)

Monitor-layer evidence through the packaged CLI on macOS and Linux, in
`tests/headless.rs` unless noted. The Contract-layer cases are mapped in the
daemon README. Scripted runs have stdin from `/dev/null` and pipes for
output, so no terminal is involved; a CLI still running at its deadline is
killed and reaped and its test fails. Scripted creation uses `c2`; the TTY
case and every kill use `climon2`, the same installed launcher. Every
headless session command ends itself after two minutes, so even a test
killed before cleaning up cannot leave a session running forever.

| Scenario | Named packaged test |
|---|---|
| Confirmed scenario: from a script with no terminal, exactly the full lowercase GUID and a newline, status 0 only once the session is listed `detached` with no client, 80x24 start; HDL-01 command completion with zero clients | `installed_headless_exec_from_a_script_prints_only_the_guid_and_starts_at_80x24` |
| Confirmed scenario: a terminal on stdin sets the initial size, with stdout captured as in `id=$(climon2 exec --headless …)`; HDL-01 existing `climon2 kill` ends a headless session | `installed_headless_exec_from_a_terminal_starts_at_its_size_and_kill_ends_it` |
| HDL-01 failed creation: 127, 126 and 125 with exec's diagnostics, nothing on stdout, nothing left | `installed_headless_creation_failures_keep_exec_statuses_and_diagnostics` |
| HDL-01 failed creation and CFG-01 (exec case): an invalid `monitor` section, a wrong type or an out-of-range limit, refused with 125 before any session side effect: nothing on stdout, only the general log written, nothing left | `installed_headless_exec_rejects_an_invalid_monitor_section_before_any_session_side_effect` |
| HDL-01 creation with an unwritable stdout (owner decision of 2026-11-13): status 0, nothing on stderr, the session keeps running, listed `detached`, until its command completes | `installed_headless_exec_that_cannot_print_the_guid_leaves_the_session_running` |
| HDL-01 zero clients past the initial attach window (ordered, not slept: the next step follows the Kill of a reference session whose command ignores hang-up, which is confirmed only after the five-second hang-up grace), after a killed client and after a deliberate detach, then Kill | `installed_headless_session_survives_zero_clients_a_killed_client_and_a_detach` |
| HDL-01 ordinary leave policy unchanged | `killed_installed_client_uses_daemon_leave_policy_and_leaves_nothing` (`tests/packaged.rs`) |

The HDL-01 cases that need a monitor discovering sessions wait for it: headless
rows appearing in the selector ([#106](https://github.com/jackgeek/climon2/issues/106))
and `c2 kill` leaving an independently running monitor's row in place
([#116](https://github.com/jackgeek/climon2/issues/116)).

### Deliberately reused preceding boundary evidence

This crate closes cross-component gaps rather than duplicating every existing
boundary test. The complete core/platform/daemon/client/CLI/installer mappings
remain in those crates' READMEs and test doc comments. In particular:

- Unsafe runtime directory refusal:
  `issue18_private_directory_refuses_symlink_file_and_permissive_mode_without_repair`
  in platform `tests/runtime.rs`.
- Empty readiness pipe and absolute deadlines:
  `issue19_empty_readiness_kills_daemon_and_cleans_metadata_before_socket` and
  `issue19_readiness_uses_real_five_second_absolute_deadline` in platform
  `tests/daemon.rs`. There is no public packaged empty-readiness hook.
- Actual stalled-producer/lossless 8 MiB backpressure:
  `saturation_backpressures_pty_without_blocking_probes_or_losing_bytes` in
  daemon `tests/lifecycle.rs`, and
  `bounded_output_backpressure_drains_every_byte_before_exit` /
  `input_backpressure_is_bounded_and_lossless` in client `tests/bridge.rs`.
  The public packaged PTY driver always drains, so this crate does not claim
  an additional end-to-end slow-reader proof.
- Exact signal/EOF/termios restoration and negative reset-sequence assertions
  remain the client's real-PTY boundary tests; the packaged TUI case additionally
  checks the fixture's final screen was not reset.

**Allowed ownership review rule (#27.4/#27.7c):** preserve the daemon's single
`Session::pump_output` → `Pty::read_output` call graph; no cloned PTY readers,
master transfer, descriptor passing, or second output pump. A textual search
would not prove ownership, so no vacuous architecture test is claimed.

## Validation and remaining release evidence

Local development runs use bash 5 (`CLIMON2_TEST_BASH`, Homebrew or
`/usr/local`, or Linux `/bin/bash`). The full architecture matrix, exact final native archive
execution and actual prior-release branch require their CI/release inputs. A successful debug
run is not evidence that those release gates passed. Packaged slow-reader,
unsafe-runtime and empty-readiness scenarios are not newly claimed here; their
preceding public-boundary evidence above remains required.
