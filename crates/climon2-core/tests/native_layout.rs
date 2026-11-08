//! DOC-01 layout case: the Rust `climon2_core::native` declarations match the
//! published revision-1 header on this host's macOS architecture.
//!
//! The header is the oracle. The test turns every Rust size, alignment, field
//! offset, field size and constant into a C11 static assertion and lets clang
//! check them against `client-v1.h` for this host's target. CI runs it on both
//! arm64 and x86_64 macOS runners. Name sets are compared in both directions so
//! that a declaration missing from either side fails too.
#![cfg(target_os = "macos")]

use climon2_core::native::*;
use std::{
    collections::BTreeSet,
    io::Write,
    mem::{align_of, offset_of, size_of},
    path::PathBuf,
    process::{Command, Stdio},
};

fn header_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/protocol/monitor-client")
}

fn header() -> String {
    std::fs::read_to_string(header_directory().join("client-v1.h")).expect("read client-v1.h")
}

fn field_size<T, F>(_: fn(&T) -> &F) -> usize {
    size_of::<F>()
}

/// The documented mechanical mapping: `OptionalText` declares `cm1_optional_text`.
fn c_name(rust: &str) -> String {
    let mut name = String::from("cm1");
    for character in rust.chars() {
        if character.is_ascii_uppercase() {
            name.push('_');
        }
        name.push(character.to_ascii_lowercase());
    }
    name
}

/// C11 static assertions generated from the Rust declarations.
#[derive(Default)]
struct Assertions {
    source: String,
    types: BTreeSet<String>,
    constants: BTreeSet<String>,
}

impl Assertions {
    fn assert(&mut self, expression: String, message: String) {
        self.source
            .push_str(&format!("_Static_assert({expression}, \"{message}\");\n"));
    }

    fn record<T>(&mut self, c_type: &str) {
        self.types.insert(c_type.to_owned());
        self.assert(
            format!("sizeof({c_type}) == {}", size_of::<T>()),
            format!("{c_type} size"),
        );
        self.assert(
            format!("_Alignof({c_type}) == {}", align_of::<T>()),
            format!("{c_type} alignment"),
        );
    }

    fn field(&mut self, c_type: &str, member: &str, offset: usize, size: usize) {
        let member = member.replace("r#", "");
        self.assert(
            format!("offsetof({c_type}, {member}) == {offset}"),
            format!("{c_type}.{member} offset"),
        );
        self.assert(
            format!("sizeof((({c_type} *)0)->{member}) == {size}"),
            format!("{c_type}.{member} size"),
        );
    }

    fn opaque<T>(&mut self, c_type: &str, _: std::marker::PhantomData<T>) {
        self.types.insert(c_type.to_owned());
    }

    fn constant(&mut self, name: &str, value: u32) {
        let name = format!("CM1_{name}");
        self.assert(format!("{name} == {value}u"), name.clone());
        self.constants.insert(name);
    }
}

/// Lists a struct's fields; the exhaustive pattern fails to compile if a Rust
/// field is missing from the list.
macro_rules! record {
    ($checks:ident, $rust:ident, [$($field:ident),+ $(,)?]) => {{
        let _exhaustive = |value: $rust| {
            let $rust { $($field: _),+ } = value;
        };
        let c_type = c_name(stringify!($rust));
        $checks.record::<$rust>(&c_type);
        $(
            $checks.field(
                &c_type,
                stringify!($field),
                offset_of!($rust, $field),
                field_size(|value: &$rust| &value.$field),
            );
        )+
    }};
}

/// Lists an anonymous C union member's variants, located through its record.
macro_rules! union_member {
    ($checks:ident, $rust:ident . $member:ident, $union:ident, [$($field:ident),+ $(,)?]) => {{
        let c_type = c_name(stringify!($rust));
        $checks.field(
            &c_type,
            stringify!($member),
            offset_of!($rust, $member),
            size_of::<$union>(),
        );
        $checks.assert(
            format!(
                "_Alignof(__typeof__((({} *)0)->{})) == {}",
                c_type,
                stringify!($member),
                align_of::<$union>()
            ),
            format!("{}.{} alignment", c_type, stringify!($member)),
        );
        $(
            $checks.field(
                &c_type,
                &format!("{}.{}", stringify!($member), stringify!($field)),
                offset_of!($rust, $member) + offset_of!($union, $field),
                // SAFETY: only the field's type is inspected; nothing is read.
                field_size(|value: &$union| unsafe { &value.$field }),
            );
        )+
    }};
}

/// Opaque handles are incomplete C types: only their names can be compared.
macro_rules! opaque {
    ($checks:ident, $($rust:ident),+) => {
        $( $checks.opaque(&c_name(stringify!($rust)), std::marker::PhantomData::<$rust>); )+
    };
}

macro_rules! constants {
    ($checks:ident, $($name:ident),+ $(,)?) => {
        $( $checks.constant(stringify!($name), $name); )+
    };
}

fn assertions() -> Assertions {
    let mut checks = Assertions::default();
    opaque!(checks, Instance, Communication, Control, Delivery);
    record!(checks, Guid, [bytes]);
    record!(checks, Utc, [bytes]);
    record!(checks, Bytes, [data, length]);
    record!(checks, Tokens, [data, count]);
    record!(checks, OptionalText, [present, reserved, value]);
    record!(checks, Path, [encoding, reserved, bytes]);
    record!(checks, OptionalPath, [present, reserved, value]);
    record!(checks, Geometry, [cols, rows]);
    record!(checks, Authority, [generation, period, restoration_id]);
    record!(checks, Error, [origin, reserved, code, message]);
    record!(checks, Call, [code, reserved, error]);
    record!(checks, Title, [state, reserved, value]);
    record!(
        checks,
        Status,
        [state, episode_present, episode_id, changed_at]
    );
    record!(checks, AvailableStatus, [available, reserved, value]);
    record!(
        checks,
        Progress,
        [state, percent_present, percent, reserved]
    );
    record!(checks, AvailableProgress, [available, reserved, value]);
    record!(checks, CommandResult, [kind, value, signal_name]);
    record!(checks, OptionalCommand, [present, reserved, value]);
    record!(checks, Metadata, [title, status, progress, command_result]);
    record!(
        checks,
        Settings,
        [
            escape_enabled,
            escape_byte,
            take_control_enabled,
            take_control_byte,
            reserved
        ]
    );
    record!(checks, Diagnostics, [log_path, filter]);
    record!(
        checks,
        Limits,
        [
            client_bytes,
            request_bytes,
            event_bytes,
            progress_bytes,
            drain_payload_bytes,
            request_records,
            event_records,
            progress_records,
            deliveries,
            operations,
            chunk_bytes,
            drain_records,
            wire_frame_bytes,
            metadata_record_bytes,
            title_bytes,
            notification_title_bytes,
            notification_body_bytes,
            max_workers,
            operation_ms,
            kill_ms,
            restoration_stall_ms,
            restoration_total_ms,
            delivery_stall_ms,
            leave_ms,
            cleanup_ms,
            start_ms,
            reserved
        ]
    );
    record!(
        checks,
        Requirements,
        [
            release_bytes,
            instance_bytes,
            owner_stack_bytes,
            worker_stack_bytes,
            workers,
            instance_descriptors,
            load_descriptors,
            reserved
        ]
    );
    record!(
        checks,
        Startup,
        [
            session_id,
            generation,
            expected_release,
            metadata_path,
            home_path,
            runtime_path,
            settings,
            diagnostics,
            limits
        ]
    );
    record!(
        checks,
        Created,
        [instance, communication, control, effective_limits]
    );
    record!(checks, Open, [mode, reserved, geometry, required, optional]);
    record!(checks, Input, [authority, bytes]);
    record!(checks, Resize, [authority, geometry]);
    record!(checks, Flag, [authority, value, reserved]);
    record!(checks, Request, [tag, reserved, request_id, args]);
    union_member!(
        checks,
        Request.args,
        RequestArgs,
        [open, input, resize, generation, authority, flag, required]
    );
    record!(checks, Opened, [connection_id, selected]);
    record!(
        checks,
        AuthorityResult,
        [changed, reserved, period, restoration_id]
    );
    record!(
        checks,
        FlagResult,
        [changed, value, reserved, accepted_bytes]
    );
    record!(checks, Termination, [command_result, cleanup, reserved]);
    record!(
        checks,
        Response,
        [request_id, outcome, result_tag, error, result]
    );
    union_member!(
        checks,
        Response.result,
        ResponseResult,
        [
            opened,
            accepted_bytes,
            authority,
            flag,
            changed,
            attachment_detached,
            termination,
            known_final
        ]
    );
    record!(
        checks,
        Granted,
        [period, restoration_id, geometry, reason, reserved]
    );
    record!(checks, Revoked, [period, restoration_id, reason, reserved]);
    record!(checks, Restoration, [period, restoration_id, geometry]);
    record!(checks, Barrier, [period, restoration_id]);
    record!(checks, Display, [authority, bytes]);
    record!(checks, GeometryEvent, [period, geometry]);
    record!(
        checks,
        Notification,
        [source, episode_present, episode_id, source_id, title, body]
    );
    record!(checks, Final, [command_result, status]);
    record!(checks, Record, [tag, reserved, generation, data]);
    union_member!(
        checks,
        Record.data,
        RecordData,
        [
            response,
            metadata,
            title,
            status,
            notification,
            progress,
            granted,
            revoked,
            restoration,
            display,
            barrier,
            geometry,
            r#final,
            diagnostic
        ]
    );
    record!(
        checks,
        Terminal,
        [
            generation,
            client_outcome,
            retry,
            leave,
            attachment_detached,
            connection_closed,
            reserved,
            error
        ]
    );
    record!(checks, RunReport, [workers_cleanup, reserved, terminal]);
    record!(
        checks,
        Drained,
        [kind, reserved, delivery, records, count, terminal]
    );
    record!(
        checks,
        Metrics,
        [
            allocation_bytes,
            peak_allocation_bytes,
            release_allocation_bytes,
            peak_release_allocation_bytes,
            request_bytes,
            peak_request_bytes,
            event_bytes,
            peak_event_bytes,
            input_bytes_accepted,
            display_bytes_delivered,
            records_drained,
            deliveries_released,
            request_records,
            peak_request_records,
            event_records,
            peak_event_records,
            deliveries,
            peak_deliveries,
            operations,
            peak_operations,
            workers,
            peak_workers,
            descriptors,
            peak_descriptors
        ]
    );
    record!(
        checks,
        Interface,
        [
            create,
            run,
            destroy,
            submit,
            drain,
            release_delivery,
            shutdown,
            release_communication,
            release_control,
            notification,
            snapshot
        ]
    );
    record!(
        checks,
        Description,
        [
            revision,
            reserved,
            release,
            capabilities,
            requirements,
            functions
        ]
    );
    constants!(
        checks,
        REVISION,
        SUCCESS,
        UNSUPPORTED_REVISION,
        INVALID_ARGUMENT,
        UNSUPPORTED_OPERATION,
        INVALID_REQUEST_ID,
        FULL,
        CLOSED,
        BUSY,
        WRONG_THREAD,
        UNSUPPORTED_LIMIT,
        RESOURCE_LIMIT,
        INTERNAL_ERROR,
        CLEANUP_FAILED,
        TIMEOUT,
        RELEASE_MISMATCH,
        ORIGIN_NONE,
        ORIGIN_LIBRARY,
        ORIGIN_DAEMON,
        ORIGIN_TRANSPORT,
        OK,
        REJECTED,
        FAILED,
        INDETERMINATE,
        MONITOR,
        INTERACTIVE,
        NATIVE_UNIX,
        UNAVAILABLE,
        UNSET,
        VALUE,
        IDLE,
        RUNNING,
        NEEDS_ATTENTION,
        ACKNOWLEDGED,
        COMPLETED,
        STATUS_FAILED,
        PROGRESS_NONE,
        PROGRESS_NORMAL,
        PROGRESS_INDETERMINATE,
        PROGRESS_ERROR,
        PROGRESS_PAUSED,
        EXIT,
        SIGNAL,
        CLEANUP_COMPLETE,
        CLEANUP_INCOMPLETE,
        CLEANUP_UNKNOWN,
        OPEN,
        INPUT,
        RESIZE,
        ACQUIRE,
        READY,
        VIEWING,
        ACKNOWLEDGE,
        LEAVE,
        KILL,
        FOCUS,
        RESULT_NONE,
        OPENED,
        INPUT_ACCEPTED,
        RESIZE_ACCEPTED,
        ACQUIRED,
        READY_RESULT,
        VIEWING_RESULT,
        ACKNOWLEDGEMENT,
        LEFT,
        TERMINATION,
        FOCUS_RESULT,
        KNOWN_FINAL,
        RESPONSE,
        INITIAL_METADATA,
        TITLE,
        STATUS,
        NOTIFICATION,
        PROGRESS,
        GRANTED,
        REVOKED,
        RESTORATION_BEGIN,
        DISPLAY,
        RESTORATION_COMPLETE,
        FINAL,
        GEOMETRY,
        DIAGNOSTIC,
        ATTACH,
        TAKE_CONTROL,
        HAND_BACK,
        SUCCESSOR,
        TAKEOVER,
        SESSION_ENDING,
        OSC9,
        OSC99,
        OSC777,
        DELIVERY,
        EMPTY,
        TERMINAL,
        CLIENT_STOPPED,
        CLIENT_FAILED,
        RETRY_NONE,
        RETRY_PASSIVE,
        RETRY_PERMANENT,
        LEAVE_NOT_REQUIRED,
        LEAVE_CONFIRMED,
        LEAVE_REJECTED,
        LEAVE_FAILED,
        LEAVE_INDETERMINATE,
    );
    checks
}

fn host_target() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64-apple-macos11",
        "x86_64" => "x86_64-apple-macos11",
        other => panic!("revision 1 supports only macOS arm64 and x86_64, not {other}"),
    }
}

/// Compiles the assertions against the header; returns clang's diagnostics.
fn compile(source: &str) -> Result<(), String> {
    let mut clang = Command::new("clang")
        .args(["-target", host_target(), "-std=c11", "-Wall", "-Wextra"])
        .args(["-Werror", "-fsyntax-only", "-ferror-limit=0", "-I"])
        .arg(header_directory())
        .args(["-x", "c", "-"])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run clang");
    clang
        .stdin
        .take()
        .unwrap()
        .write_all(format!("#include \"client-v1.h\"\n#include <stddef.h>\n{source}").as_bytes())
        .unwrap();
    let output = clang.wait_with_output().unwrap();
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

/// Names declared by `typedef ... cm1_x;` in the header.
fn header_types(header: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for statement in header.split(';') {
        let statement = statement.trim_end();
        let is_typedef = statement.contains("typedef struct");
        let name: String = statement
            .chars()
            .rev()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if name.starts_with("cm1_") && (is_typedef || statement.ends_with(&format!("}} {name}"))) {
            names.insert(name);
        }
    }
    names
}

/// Names of every `#define CM1_* UINT32_C(n)` in the header.
fn header_constants(header: &str) -> BTreeSet<String> {
    header
        .lines()
        .filter_map(|line| line.strip_prefix("#define CM1_"))
        .filter(|definition| definition.contains("UINT32_C("))
        .map(|definition| format!("CM1_{}", definition.split_whitespace().next().unwrap()))
        .collect()
}

/// DOC-01 layout case: sizes, alignments, field offsets and sizes, numeric tags
/// and result codes of every Rust declaration equal the header's on this host.
#[test]
fn rust_native_declarations_match_the_published_header() {
    let checks = assertions();
    if let Err(diagnostics) = compile(&checks.source) {
        panic!("Rust declarations differ from client-v1.h:\n{diagnostics}");
    }
    let header = header();
    assert_eq!(checks.types, header_types(&header), "declared types");
    assert_eq!(
        checks.constants,
        header_constants(&header),
        "declared constants"
    );
}

/// The comparison is not vacuous: clang reports a single wrong value by name.
#[test]
fn a_layout_difference_is_reported_against_the_header() {
    let diagnostics = compile("_Static_assert(sizeof(cm1_bytes) == 24, \"cm1_bytes size\");")
        .expect_err("a wrong size must not compile");
    assert!(diagnostics.contains("cm1_bytes size"), "{diagnostics}");
}
