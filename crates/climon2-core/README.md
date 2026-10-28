# climon2-core

Pure contracts for platform, CLI, daemon and client callers. Core performs no
filesystem/process/terminal access and contains no platform selection.
`test_hook_marker!` is preserved.

## Public boundaries

- `SessionId`, `VersionName`, `Timestamp`: `FromStr` and validated Serde strings;
  canonical lowercase hyphenated GUID, SemVer path component, and RFC 3339 UTC
  with exactly three fractional digits. `ContractError` reports invalid input.
- `protocol::Frame::{new,type_code,body}`, `read_frame(&mut impl Read)`,
  `write_frame(&mut impl Write, &Frame)`: bounded exact byte-stream framing.
  Length includes the type byte: 1–1,048,576 bytes; Input/Output frame lengths
  are at most 65,536 bytes (65,535 payload bytes).
- `protocol::{ClientMessage,ServerMessage}::{to_frame,from_frame}`: directional
  protocol-1 messages, object-only JSON controls and uninterpreted binary data.
  Unknown JSON fields are ignored; unknown codes/directions are errors.
  Parse errors do not echo untrusted values into diagnostics.
  `HelloReply::{is_live_session,validate_client}` separates cross-version probes
  from version-matched client validation.
- `protocol::ConnectionState::receive`: per-connection hello/probe/attach/input
  sequencing, poisoned on protocol error. This does **not** arbitrate session
  ownership across connections; the daemon separately rejects a second client.
  Error closes the connection; no Detach, Snapshot or protocol-2 fields exist.
- `protocol::TerminalSize::{initial,resize}` handles fallback and ignored sizes.
  `hello_time_remaining(elapsed)` is an absolute two-second budget, not a
  per-read timeout. Callers must enforce it on their transport, starting before
  connect or at accept. Constants also describe hello, startup, hang-up grace
  and PTY drain deadlines.
- `metadata::SessionMetadata::{from_json,to_json}`: eleven typed frozen fields.
  `NonZeroU32` expresses format version/depth; future versions are accepted.
  `from_json` validates the `.pty` filename and matching GUID.
  `metadata_session_id` filters candidate filenames.
  `metadata::MetadataPath` accepts absolute Unix, drive-rooted Windows and UNC
  display paths independently of the reader OS.
  `metadata::DisplayCommand::from_runtime` is a **one-way** lossy UTF-8
  projection; never reconstruct runtime argv/cwd from metadata.
- `layout::HomeLayout::resolve`: only an absent override defaults to
  `<user-home>/.climon2`. Explicit empty/relative overrides fail.
  `root`, `sessions`, `metadata`, `session_logs`, `general_logs` return paths.
- `layout::ExecutableLayout::from_cli_path`: accepts a caller-canonicalized
  absolute CLI path with a validated SemVer parent. `root`, `version`, `binary`
  and `client_invocation` locate version-matched companions. The latter returns
  `Invocation { program, args }`, exactly `attach --metadata <absolute-path>`;
  other client configuration belongs in environment. `.exe` naming is inferred
  from the explicit input, never from a target cfg.
  `VersionBinary` enumerates CLI/daemon/client; `parse_current` validates the
  current file, permitting one LF or CRLF terminator, not arbitrary trimming.
- `layout::home_hash8` hashes exact canonical-home encoded bytes, not lossy
  text. `RuntimeLayout::{new,directory,socket_path}` supplies naming and a
  caller-selected socket byte limit. The platform supplies the runtime root,
  canonicalization, UID, ownership/mode checks and socket operations.
- `cli::CliArgs::parse`: arguments **exclude** argv[0]. Exec retains
  `CommandArgs { command: OsString, args: Vec<OsString> }`, including empty
  trailing arguments and option-looking tokens. Optional `--` separates exec
  from a command. List permits only `--json`; top-level help/version are modeled.
  `UsageError::exit_code` returns 2.
- `session::SessionEnvironment::for_session`: preserves inherited OS-string
  values except the two overwritten session variables and absent-only TERM
  default. Nonempty enclosing session ID means nested; malformed enclosing
  depth means 1. `depth` is nonzero; an increment beyond u32 returns
  `DepthOverflow`. `nested_warning` contains exact stderr text without newline.
  `ordinal` handles English teen/century exceptions.
- `exit::{ExitStatus,LaunchFailure}::code`: neutral platform-exit input,
  preserving raw command u32 codes, checked `128+signal`, and classifications
  for own failure 125, usage 2, launch 127/126 and terminal gone 129.

## Specification tests

Integration tests call only these public boundaries:

| Rules | Named tests |
|---|---|
| #20 / ADR 0001 partial I/O, raw bytes and bounds | `issue20_frozen_frame_handles_partial_io_and_raw_bytes`, `issue20_frame_bounds_include_type_and_reject_truncation_before_allocation` |
| #20 exact codes, control types, directions, sizes, deadlines and states | `issue20_protocol1_exact_wire_codes_and_json_have_no_feature2_reservations`, `issue20_server_protocol1_exact_wire_codes_and_json`, `issue20_control_json_is_an_object_with_required_typed_fields_and_direction`, `issue20_probe_is_not_attached_and_protocol_errors_are_terminal`, `issue20_errors_close_from_every_state_and_prehello_input_is_rejected`, `issue20_size_rules_and_absolute_deadline_do_not_restart_for_partial_io` |
| ADR 0001 / #27 unknown hello fields and version checks | `adr0001_hello_ignores_future_fields_for_probe_and_version_matched_client` |
| ADR 0005 GUID/SemVer/time validation | `adr0005_guid_and_version_are_validated_without_normalizing_untrusted_paths`, `adr0005_created_at_requires_real_utc_calendar_time_with_exact_milliseconds` |
| ADR 0005 / #26 frozen fields, future999 and one-way display | `adr0005_append_only_goldens_include_future999_and_unknown_fields`, `adr0005_frozen_fields_reject_missing_wrong_types_invalid_values_and_filename_mismatch`, `adr0005_metadata_requires_a_json_object_not_a_positional_array`, `adr0005_metadata_paths_are_absolute_cross_os_display_strings_not_native_paths`, `adr0005_display_is_lossy_only_and_never_used_as_runtime_argv` |
| #17/#18/#28 home, layout, hash and frozen invocation | `issue17_home_defaults_only_when_unset_and_never_accepts_empty_or_relative`, `adr0001_version_matched_client_has_exact_frozen_invocation`, `issue18_runtime_hash_is_sha256_of_exact_canonical_home_and_socket_is_bounded` |
| #29 recovered argv/environment/depth/ordinal rules | `issue29_exec_preserves_literal_os_arguments_and_list_rejects_usage_errors`, `issue29_top_level_ignores_stray_depth_and_sets_only_session_vars_and_absent_term`, `issue29_nested_depth_defaults_malformed_enclosing_to_one_and_preserves_empty_term`, `issue29_nested_ordinals_include_teens_centuries_and_explicit_depth_overflow` |
| #17 / #20 exit mapping | `issue17_failures_remain_distinct_from_command_status_and_signal_mapping` |

`adr0005_every_checked_in_golden_is_readable_by_the_current_reader` automatically
discovers every golden. `malformed_metadata_and_control_errors_do_not_echo_external_values`
guards diagnostic privacy.

`tests/goldens/metadata-*.json` is append-only: add fixtures, never rewrite or
remove existing fixtures. Platform file modes, atomic publication, PID reuse,
liveness, TTYs and actual OS deadlines belong to the platform/daemon/client
test suites.
