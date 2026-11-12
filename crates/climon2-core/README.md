# climon2-core

Pure contracts for platform, CLI, daemon and client callers. Core performs no
process/terminal access and contains no platform selection; its only
filesystem access is `config::ConfigSource::read`, one read of `config.jsonc`.
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
- `hello`: optional hello-reply extensions. `Capabilities` (`attach`, `kill`,
  `state-v1`, `client-startup-v1`; attach and kill each also need
  `client-startup-v1`) and `SessionState` (lifecycle `starting`, `attached`,
  `detached` or `terminating`, attachment count, controlled) are parsed
  leniently: absent grants nothing, malformed is reported and never trusted,
  unknown tokens are ignored. `advertise` builds a reply frame with exactly
  those frozen tokens and state. `daemon_wire_v1` is `Valid`
  only when the capabilities include `daemon-wire-v1` and `daemon_wire_v1`
  holds positive u32 `max_request_frame_bytes` and `max_server_frame_bytes`;
  neither is `Absent`, anything else is `Malformed`, and none of these ever
  makes a session stale. `advertise_revision1` encodes the same frozen tokens
  and state plus a revision-1 `Advertisement`: its families after the frozen
  tokens and its `daemon_wire_v1` limits. Neither encoder can omit the frozen
  tokens or state.
- `wire`: the platform-neutral, stateless daemon wire revision 1 codec shared
  by the daemon and the client library (part 1: framing, shared types and
  session messages). Roles, ordering, authority, request-ID sequencing and
  whether a Response's result belongs to its request's operation belong to
  the connection owners: a stateless codec does not know which request an
  ID names.
  - `read_frame(&mut impl Read, FrameLimit)` and
    `write_frame(&mut impl Write, &Frame, FrameLimit)`: `u32be length | u8
    type | body`, the length counting type and body. `FrameLimit` is a
    positive local ceiling; pass the smaller of it and the peer's advertised
    maximum when writing. Oversize is `frame_too_large`, decided from the
    prefix before any body is read; zero is `invalid_frame_length`; closure
    between frames is `Ok(None)` and inside a frame `truncated_frame`;
    transport errors stay `WireError::Io`. A frame the limit refuses is
    never written. Otherwise the frame goes out in one `write_all`, so an I/O
    error can leave part of it written; close the connection on any error.
  - `FrameLimit::frame_length([u8; 4])`: the length a prefix declares, under
    the same rules as `read_frame`, so a nonblocking reader knows how many
    bytes a frame still needs without parsing frames itself. `defines(type)`:
    whether revision 1 defines a frame type in either direction (`0x30`–`0x39`
    and `0x40`–`0x4d`); a daemon still speaking another post-hello protocol
    selects revision 1 by the type of the first frame after hello.
  - `Request::from_frame` (daemon side: Open `0x30`, Leave `0x37`, Kill
    `0x38`) and `ServerMessage::from_frame` (client side: Response `0x40`,
    Initial metadata `0x41`, Title changed `0x42`, Status changed `0x43`,
    Notification `0x44`, Progress changed `0x45`, Command final `0x4c`, Fatal
    error `0x4d`) return a `FatalCode`: `wrong_direction` for a type
    documented for the other direction, `unknown_message_type` otherwise,
    `invalid_json` for broken JSON or UTF-8, `invalid_request_id` for a
    malformed `request_id` (the message type decides: every request and the
    Response carry one), and `invalid_record` for any other schema
    violation: unknown, duplicate or missing members (nullable ones
    included), wrong types, non-object records and inconsistent values.
    `ControlRecord::to_frame` encodes members in their documented order.
  - Shared types: `Counter` (canonical decimal string 1 through
    18446744073709551615), `Guid` (lowercase canonical hyphenated, validated
    exactly like `SessionId`), `Geometry` (positive columns and rows), `Utc`
    (`YYYY-MM-DDTHH:MM:SS.mmmZ` on a real date, seconds 00-59; stricter than
    the frozen `Timestamp`, whose RFC 3339 also admits a leap second and a
    `t` or space separator), `CommandResult` (raw u32 exit or signal number with
    optional name; `final_state`), `Status` (episodes exactly for the two
    attention states), `Progress` and `Percent` (0-100, present as the state
    requires), `Available<T>` (unavailable carries null) and `Error` with a
    nonempty `ErrorCode`. `OperationalCode` and `FatalCode` are the two
    catalogues; an unknown code stays an explicit non-success.
  - Messages: `Open` (`Requested` keeps required and optional `Families`
    disjoint; `Mode` pairs monitor with no geometry and interactive with a
    geometry), `Leave`, `Kill`, `Response` (`Outcome::Ok` holds a `Success`;
    every non-success holds an `Error`: a `Rejection` at most a known final
    command result, and a failed or indeterminate `Failure` at most a
    `KnownResult`, either the operation's known result in its success shape
    or a Kill's `PartialTermination`, never a confirmed termination, which
    `Failure::new` refuses; any other pairing of outcome, result and error
    is `invalid_record`), every documented result variant,
    `InitialMetadata`, `TitleChanged`,
    `StatusChanged` (never a final state), `Notification`,
    `ProgressChanged`, `CommandFinal` (status null or the agreeing final
    status) and `FatalError`. `Family`, `FrameLimits` and `Advertisement`
    describe revision-1 support in hello.
  - Part 2 adds the control and terminal messages as new record types and
    decoder-table entries.
- Protocol-1 messages: client `Hello`, `Attach`, `Input`, `Resize`,
  `TakeControl`, `Ready{period}`, `Detach`, `Kill`, `Error`; server `Hello`,
  `Attached`, `Output`, `Exit{code}`, `Granted{period}`, `RestoreEnd{period}`,
  `Revoked`, `Detached`, `Terminated{code}`, `Error`.
- `protocol::ConnectionState::receive`: per-connection sequencing through
  `AwaitingHello`, `Probing`, `Attached`, `Killing` and `Closed`, poisoned on
  protocol error. A connection chooses `Attach` or `Kill` once; attachment
  requests need `Attached`. This does **not** arbitrate control across
  connections; the daemon decides which attachment controls. Error closes the
  connection.
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
  from a command. Attach and kill take exactly one `selector::Selector`. List
  permits only `--json`; top-level help/version take no arguments.
  `UsageError::exit_code` returns 2; `cli::USAGE` is the usage line.
- `selector::Selector`: 1 to 32 hex digits after removing hyphens and folding
  case; anything else is a `UsageError`. `select` picks exactly one candidate by
  unique prefix or full ID; no match or ambiguity (listing every full ID) is a
  `SelectionError`.
- `config`: `ConfigSource::read` reads `config.jsonc` once (absent means
  defaults, unreadable is an error). `exec()` validates the whole file into
  `ExecConfig` (scrollback `MaxRows`, an integer 0..=100000, default 10000, plus
  client settings); `attach()` validates only the `client` section into
  `AttachConfig`. JSONC comments and trailing commas are accepted; unknown
  fields are ignored; errors name their class.
- `keys`: `ControlKey` is one C0 control byte named in the `Ctrl+` vocabulary.
  `ClientSettings` holds the escape prefix (default `Ctrl+]`) and take-control
  key (default `Ctrl+T`); either may be disabled, take-control may not be
  `Ctrl+[` (Escape), and enabled bindings may not share a byte.
- `startup_envelope::StartupEnvelope`: the schema-1 `CLIMON2_CLIENT_STARTUP`
  JSON, `attach` with exact settings or `kill`, at most 4096 bytes; a present
  envelope is used exactly or rejected.
- `title::{encode,default_title}`: percent-encodes C0, DEL and C1 per UTF-8
  byte; the default title joins the command argv with single spaces.
- `session::SessionEnvironment::for_session`: preserves inherited OS-string
  values except the replaced keys: the two session variables,
  `TERM=xterm-256color` and `COLORTERM=truecolor` are always set, and any
  inherited `CLIMON2_CLIENT_STARTUP` is dropped. Nonempty enclosing session ID
  means nested; malformed enclosing
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
| #29 recovered argv/environment/depth/ordinal rules | `issue29_exec_preserves_literal_os_arguments_and_list_rejects_usage_errors`, `issue29_help_and_version_flags_take_no_arguments`, `issue29_top_level_ignores_stray_depth_and_sets_session_vars_and_terminal_identity`, `projected_environment_replaces_term_and_colorterm_and_drops_startup_envelope`, `issue29_nested_depth_defaults_malformed_enclosing_to_one_and_replaces_empty_term`, `issue29_nested_ordinals_include_teens_centuries_and_explicit_depth_overflow` |
| Continuity messages and connection choice | `continuity_messages_have_exact_wire_codes_and_json`, `continuity_bodies_are_typed_objects_and_each_code_has_one_direction`, `a_connection_chooses_attach_or_kill_once_and_attachment_requests_need_attach` |
| #17 / #20 exit mapping | `issue17_failures_remain_distinct_from_command_status_and_signal_mapping` |
| DOC-01 (Contract case for the part-1 daemon wire pages and the hello reply): every published example decodes to its documented values and re-encodes byte-identically | `published_request_examples_round_trip_byte_identically`, `published_response_and_fatal_error_examples_round_trip_byte_identically`, `published_core_metadata_examples_round_trip_byte_identically`, `published_hello_advertisement_examples_round_trip_byte_identically` |
| COM-01 (codec case): revision 1 needs its token and valid frame limits, otherwise unavailable; known families; unique, disjoint capability arrays; the daemon's frozen advertisement is unchanged; the published hello examples carry these advertisements | `a_revision1_advertisement_needs_its_token_and_valid_frame_limits`, `advertised_families_are_the_known_revision1_tokens`, `revision1_support_is_advertised_beside_the_frozen_tokens_and_state`, `capability_arrays_are_unique_case_sensitive_and_disjoint`, `the_daemon_advertises_the_frozen_tokens_and_a_state_beside_the_mandatory_reply`, `published_hello_advertisement_examples_round_trip_byte_identically` |
| COM-02 (codec case): framing: partial I/O, the local frame ceiling at its limit and one past it, zero and type-only lengths, truncation and transport failures; a prefix alone decides the admitted length; the frame types revision 1 defines | `revision1_frames_survive_partial_io_and_end_cleanly_at_a_frame_boundary`, `the_local_frame_ceiling_admits_its_limit_and_rejects_one_byte_more`, `zero_length_is_invalid_and_a_type_only_frame_has_an_empty_body`, `a_stream_ending_inside_a_frame_is_truncated`, `transport_failures_are_reported_as_io_errors`, `a_prefix_alone_decides_the_admitted_frame_length`, `revision1_defines_the_request_and_server_types_only` |
| LIM-01 (codec case): the local frame ceiling at its limit and one past it, decided from the length prefix before any body is read | `the_local_frame_ceiling_admits_its_limit_and_rejects_one_byte_more`, `a_prefix_alone_decides_the_admitted_frame_length` |
| COM-02 (codec case): types and directions, every member of every part-1 record at every depth (required even when null, unique, documented), JSON and UTF-8, request IDs, capability arrays, integers, geometry, GUIDs, UTC times, episodes, progress and notifications | `requests_are_identified_by_type_and_direction`, `server_messages_are_identified_by_type_and_direction`, `request_ids_are_canonical_decimal_counters`, `only_requests_and_responses_carry_request_ids`, `requests_reject_unknown_duplicate_and_missing_members`, `requests_must_be_complete_utf8_json_objects`, `unsigned_fields_are_plain_integer_tokens`, `open_geometry_is_null_for_monitor_and_positive_for_interactive`, `an_interactive_open_encodes_its_documented_members_in_order`, `every_member_of_every_part1_record_is_required_unique_and_documented`, `success_results_reject_undocumented_shapes`, `connection_ids_are_lowercase_canonical_guids`, `fatal_errors_carry_each_connection_fatal_code`, `utc_times_are_exact_calendar_milliseconds`, `status_episodes_belong_only_to_attention_states`, `progress_percent_matches_its_state`, `notifications_are_normalized_records_from_known_sources`, `capability_arrays_are_unique_case_sensitive_and_disjoint` |
| COM-03 (codec case): every documented result variant; outcome, result and error pairing (a known final result only on a rejection; a failed or indeterminate outcome may carry the operation's known result or a Kill's partial termination, never a confirmed one); catalogue and unknown error codes | `every_documented_success_result_round_trips`, `responses_pair_each_outcome_with_its_result_and_error`, `error_codes_are_nonempty_and_unknown_codes_stay_non_successes` |
| COM-06 (codec case): normal exit differs from signal; a final status agrees with the command result; Status changed never carries the final state; a partial Kill result or a known final result is never a successful confirmation | `command_results_distinguish_normal_exits_from_signals`, `command_final_status_is_terminal_and_agrees_with_the_result`, `initial_metadata_terminal_status_agrees_with_a_known_result`, `status_episodes_belong_only_to_attention_states`, `responses_pair_each_outcome_with_its_result_and_error` |
| CTL-03 (codec case): unavailable, unset and empty values stay distinct | `initial_metadata_reports_availability_explicitly`, `title_changes_distinguish_unset_from_empty` |
| STA-07 (codec case): status records keep exact UTC change times and attention-only episodes | `utc_times_are_exact_calendar_milliseconds`, `status_episodes_belong_only_to_attention_states` |

`tests/config.rs`, `tests/hello.rs`, `tests/keys.rs`, `tests/selector.rs`,
`tests/startup_envelope.rs` and `tests/title.rs` specify those modules one rule
per test.

`tests/wire_framing.rs`, `tests/wire_requests.rs`, `tests/wire_server.rs` and
`tests/wire_examples.rs` specify the revision-1 codec. Their doc comments cite
the acceptance-matrix IDs above as codec cases: pure, platform-neutral evidence
that supplements, and never replaces, the daemon contract and library layers.
`tests/wire_examples.rs` reads each example from its published page.

`adr0005_every_checked_in_golden_is_readable_by_the_current_reader` automatically
discovers every golden. `malformed_metadata_and_control_errors_do_not_echo_external_values`
guards diagnostic privacy.

`tests/goldens/metadata-*.json` is append-only: add fixtures, never rewrite or
remove existing fixtures. Platform file modes, atomic publication, PID reuse,
liveness, TTYs and actual OS deadlines belong to the platform/daemon/client
test suites.
