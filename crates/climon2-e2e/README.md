# climon2-e2e

The default macOS packaged acceptance tier (#38), using the public installer,
the PATH launcher, real sibling binaries and `climon2-test-term` PTYs.
There are no nested Cargo builds, ignored packaged tests, retries, or shortened
production deadlines. Each scenario has an isolated HOME and climon2 home;
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

`bash_profiles` selects macOS bash 3.2 and Homebrew bash 5. Missing profiles fail
CI and are explicitly reported locally. `CLIMON2_TEST_BASH` selects one existing
profile for a matrix job. Vim and less are the actual `/usr/bin` applications.

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

Internal-only fault injections and the explicitly synthetic client-invocation
fixture are debug-sibling scenarios, not applicable to an unmodified no-hooks
archive. All black-box terminal, lifecycle, missing-client, privacy, logging and
raw-socket cases run against the archive. A removed client is restored by
renaming the same archive binary, never by rebuilding/substituting it.

From release two, set `CLIMON2_E2E_PREVIOUS_ARCHIVE` to the actual published
previous release archive as well. The compatibility gate starts its real
client/session before installing the current archive into the same executable
home, then both CLIs list both versions and both clients deliver command exit
statuses. With no prior archive, the first-release gate explicitly reports that
no compatibility evidence was fabricated.

## Specification mapping

Each test also has a specification doc comment in `tests/packaged.rs`.

| Requirement | Named packaged test |
|---|---|
| #16/#21.6/#38 outer/inner controlled bash, arithmetic 42, second PTY list exactly one, outer status 7, empty list and leaks | `installer_built_bash_smoke_lists_one_session_and_preserves_exit_seven` |
| #16/#29 exact literal/empty argv, cwd, whole environment, real initial size/resize/status; #25 TRACE argv/input/environment privacy | `installed_generic_fixture_preserves_runtime_contract_and_log_privacy` |
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

The local development run covers bash 3.2; Homebrew bash 5 was explicitly missing.
The full architecture/bash matrix, exact final universal archive execution and
actual prior-release branch require their CI/release inputs. A successful debug
run is not evidence that those release gates passed. Packaged slow-reader,
unsafe-runtime and empty-readiness scenarios are not newly claimed here; their
preceding public-boundary evidence above remains required.
