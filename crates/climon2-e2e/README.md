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
fixture, which uses only `dlopen` (`RTLD_NOW | RTLD_LOCAL`), `dlsym` of
`climon2_client_get_interface` and the returned function table, never Rust
calls, static linking or a mock table. The host prints its observations as
JSON; the tests decide what is correct. Each host run is bounded; one that
outlives its deadline is killed, reaped and failed. In archive mode the same
tests load the exact archive's library, so the CI archive jobs exercise both
macOS architectures' slices and the Linux library. The exported symbol set is
a release check (`xtask`), not a Library-layer test.

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

### Library layer (`tests/library.rs`, #75)

| Scenario | Named test |
|---|---|
| LIB-01 Library: absolute exact-release load; release text is the version folder name; one stable description; bounded requirements; no unimplemented capability | `installed_library_describes_revision_one_with_its_version_identity` |
| LIB-01 Library: with copies named by `current`, in the working directory and on ambient search paths (`LD_LIBRARY_PATH`, `DYLD_FALLBACK_LIBRARY_PATH`), the absolute path maps the requested file (the system loader's lookup, which no library change can alter); the library answers from its own image with its own folder's release, not `current`'s, and a misplaced copy reports its compiled-in release | `installed_library_loads_by_absolute_version_path_whatever_current_cwd_and_search_paths` |
| LIB-02 Library: unsupported revisions rejected with a null description; null output storage | `installed_library_rejects_other_revisions_without_a_description` |
| DOC-01: all eleven table entries report unsupported operation by the frozen call rules | `installed_library_table_reports_every_instance_call_unsupported` |

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

| Scenario | Named packaged test (`tests/monitor.rs`) |
|---|---|
| ENT-01 `c2 monitor`, `c2 mon`, `climon2 monitor` and `climon2 mon` open the same installed monitor through the launcher: top row, empty state, exact host modes, prefix+x exits 0 with the prefix read alone | `installed_monitor_and_mon_open_the_same_monitor` |
| INP-01 configured prefix hint and exit 0, null take-control key accepted; once read, Ctrl+C and the default prefix+x are not exits (SIGTERM then gives 143) | `installed_monitor_hints_and_exits_with_the_configured_prefix` |
| INP-01 null escape prefix and invalid keys refused before terminal changes | `installed_monitor_refuses_a_null_escape_prefix_and_invalid_keys` |
| CFG-01 unreadable and malformed JSONC, null, wrong type, duplicate, overflow, out-of-range and inconsistent settings: one diagnostic, 125, no terminal change | `installed_monitor_refuses_every_invalid_configuration_before_the_terminal_changes` |
| CFG-01 `monitor.show_key_hints` false hides the empty state's hint, prefix+x still exits; unknown and unrelated settings ignored | `installed_monitor_without_key_hints_shows_none_and_still_exits` |
| CFG-01 no live reload: after an edit, a repaint keeps the started settings and the edited prefix, once read, does not exit (SIGTERM then gives 143) | `editing_the_configuration_while_the_monitor_runs_changes_nothing` |
| CFG-01 every effective setting in the monitor's general log | `installed_monitor_logs_its_effective_configuration` |
| INP-03 (empty monitor) pastes never exit, never complete a pending prefix and leave none pending | `pastes_never_exit_or_leave_a_pending_prefix` |
| Derived scenario: monitor argument refusal, usage and 2 | `installed_monitor_arguments_are_a_usage_error` |
| Derived scenario: monitor non-TTY stdin or stdout refusal | `installed_monitor_refuses_a_non_terminal_stdin_or_stdout` |
| Spec #73 entry: missing monitor fails with 125 naming the version | `missing_installed_monitor_fails_naming_its_version` |
| Spec #73 startup: no executable home outside a version folder | `a_monitor_outside_a_version_folder_refuses_to_start` |
| EXT-03 (Monitor, empty monitor) SIGINT/SIGTERM/SIGHUP give 130/143/129 | `signals_release_the_host_with_signal_style_status` |
| EXT-03 (Monitor, empty monitor) hung-up host input releases the host before its one diagnostic, logged, 125 | `a_hung_up_host_input_releases_the_host_before_its_diagnostic` |
| EXT-03 (Fault, empty monitor) render failure or failed wait for host input releases the host before its one diagnostic, 125 | `debug_hook_render_failure_restores_the_host_before_its_diagnostic`, `debug_hook_input_wait_failure_restores_the_host_before_its_diagnostic` |
| Spec #73 host terminal; TRM-04 (empty monitor): SIGWINCH repaints exactly at the new size without erasing or showing the cursor | `resizing_the_host_redraws_the_monitor_at_the_new_size` |
| Spec #73 exit: content-free monitor general log | `installed_monitor_logs_content_free_records_to_its_general_log` |

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
