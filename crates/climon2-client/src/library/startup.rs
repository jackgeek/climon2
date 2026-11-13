//! `cm1_startup`, validated and copied once, at `create`: what an instance
//! keeps is its own copy, never a caller buffer.

use super::{
    RELEASE, REQUIREMENTS,
    failure::{Failure, INVALID_ARGUMENT, RELEASE_MISMATCH, UNSUPPORTED_LIMIT, require},
    guid,
};
use climon2_core::{
    diagnostics::Filter,
    keys::{ClientSettings, ControlKey},
    native::{self, Bytes},
};
use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf, slice};

/// The validated startup, parsed and copied: nothing here borrows a caller
/// buffer.
pub(super) struct Startup {
    /// The session's GUID in its canonical text, as metadata and hello name it.
    pub(super) session: String,
    pub(super) generation: u64,
    pub(super) paths: Paths,
    pub(super) settings: ClientSettings,
    pub(super) diagnostics: Diagnostics,
    pub(super) limits: native::Limits,
}

/// Copies of the absolute native paths, byte for byte.
pub(super) struct Paths {
    pub(super) metadata: PathBuf,
    pub(super) home: PathBuf,
    pub(super) runtime: PathBuf,
}

pub(super) struct Diagnostics {
    pub(super) log: Option<PathBuf>,
    /// Parsed even when nothing is logged: the `CLIMON2_LOG` grammar.
    pub(super) filter: Filter,
}

impl Startup {
    /// Validates and copies every input before anything is created.
    ///
    /// # Safety
    ///
    /// Every nonnull buffer in `startup` covers its length for this call (the
    /// caller's contract).
    pub(super) unsafe fn parse(startup: &native::Startup) -> Result<Self, Failure> {
        // SAFETY (all calls): guaranteed by the caller.
        let limits = unsafe { supported(startup) }?;
        let (generation, paths, settings) = unsafe { identity(startup) }?;
        let diagnostics = unsafe { diagnostics(startup.diagnostics) }?;
        Ok(Self {
            session: guid::text(startup.session_id.bytes),
            generation,
            paths,
            settings,
            diagnostics,
            limits,
        })
    }
}

impl Paths {
    /// The bytes these copies hold, which their instance's allocation counts.
    pub(super) fn bytes(&self) -> u64 {
        [&self.metadata, &self.home, &self.runtime]
            .into_iter()
            .map(|path| path.as_os_str().len() as u64)
            .sum()
    }
}

fn invalid(message: &'static str) -> Failure {
    Failure::new(INVALID_ARGUMENT, message)
}

/// A required nonempty buffer: nonnull and addressable.
///
/// # Safety
///
/// A nonnull `data` covers `length` readable bytes.
pub(super) unsafe fn buffer<'a>(bytes: Bytes) -> Option<&'a [u8]> {
    isize::try_from(bytes.length)
        .ok()
        .filter(|length| *length > 0 && !bytes.data.is_null())
        // SAFETY: guaranteed by the caller for a nonnull, nonempty buffer.
        .map(|length| unsafe { slice::from_raw_parts(bytes.data, length as usize) })
}

/// Required nonempty UTF-8 text.
///
/// # Safety
///
/// As for [`buffer`].
unsafe fn text<'a>(bytes: Bytes, message: &'static str) -> Result<&'a str, Failure> {
    // SAFETY: guaranteed by the caller.
    unsafe { buffer(bytes) }
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .ok_or(invalid(message))
}

/// The library serves only its own release, the version folder it ships in.
///
/// # Safety
///
/// As for [`buffer`].
unsafe fn release(expected: Bytes) -> Result<(), Failure> {
    // SAFETY: guaranteed by the caller.
    let expected = unsafe { text(expected, "expected_release must be nonempty UTF-8") }?;
    require(
        expected == RELEASE,
        Failure::new(
            RELEASE_MISMATCH,
            "expected_release is not this library's release",
        ),
    )
}

/// What this release supports: its own release, and limits it can honour.
///
/// # Safety
///
/// As for [`buffer`].
unsafe fn supported(startup: &native::Startup) -> Result<native::Limits, Failure> {
    // SAFETY: guaranteed by the caller.
    unsafe { release(startup.expected_release) }?;
    limits(startup.limits)
}

/// The nonzero connection generation, the session's native paths and the
/// client keys.
///
/// # Safety
///
/// As for [`buffer`].
unsafe fn identity(startup: &native::Startup) -> Result<(u64, Paths, ClientSettings), Failure> {
    require(
        startup.generation != 0,
        invalid("generation must be nonzero"),
    )?;
    // SAFETY: guaranteed by the caller.
    let paths = unsafe { Paths::parse(startup) }?;
    Ok((startup.generation, paths, settings(startup.settings)?))
}

impl Paths {
    /// # Safety
    ///
    /// As for [`buffer`].
    unsafe fn parse(startup: &native::Startup) -> Result<Self, Failure> {
        // SAFETY (all calls): guaranteed by the caller.
        Ok(Self {
            metadata: unsafe { path(startup.metadata_path) }?,
            home: unsafe { path(startup.home_path) }?,
            runtime: unsafe { path(startup.runtime_path) }?,
        })
    }
}

/// An absolute native Unix path, copied byte for byte: invalid UTF-8 is
/// preserved; NUL, other encodings and paths the system cannot open
/// (`PATH_MAX` bytes or more) are rejected.
///
/// # Safety
///
/// As for [`buffer`].
unsafe fn path(path: native::Path) -> Result<PathBuf, Failure> {
    (path.encoding == native::NATIVE_UNIX && path.reserved == 0)
        // SAFETY: guaranteed by the caller.
        .then(|| unsafe { buffer(path.bytes) })
        .flatten()
        .filter(|bytes| absolute(bytes))
        .map(|bytes| PathBuf::from(OsString::from_vec(bytes.to_vec())))
        .ok_or(invalid(
            "paths must be absolute native Unix paths without NUL, shorter than PATH_MAX",
        ))
}

fn absolute(bytes: &[u8]) -> bool {
    bytes.len() < libc::PATH_MAX as usize && bytes[0] == b'/' && !bytes.contains(&0)
}

/// # Safety
///
/// As for [`buffer`].
unsafe fn diagnostics(diagnostics: native::Diagnostics) -> Result<Diagnostics, Failure> {
    // SAFETY (both calls): guaranteed by the caller.
    let log = unsafe { optional_path(diagnostics.log_path) }?;
    let filter = unsafe { text(diagnostics.filter, "filter must be nonempty UTF-8") }?;
    let filter =
        Filter::parse(filter).map_err(|_| invalid("filter is not a valid CLIMON2_LOG filter"))?;
    Ok(Diagnostics { log, filter })
}

/// # Safety
///
/// As for [`buffer`].
unsafe fn optional_path(log: native::OptionalPath) -> Result<Option<PathBuf>, Failure> {
    match (log.present, log.reserved) {
        (0, 0) => Ok(None),
        // SAFETY: guaranteed by the caller.
        (1, 0) => unsafe { path(log.value) }.map(Some),
        _ => Err(invalid("log_path presence must be 0 or 1, reserved zero")),
    }
}

fn settings(settings: native::Settings) -> Result<ClientSettings, Failure> {
    let invalid = invalid("settings must hold valid, distinct client keys");
    let escape = key(settings.escape_enabled, settings.escape_byte).ok_or(invalid)?;
    let take_control = key(settings.take_control_enabled, settings.take_control_byte);
    let take_control = take_control.ok_or(invalid)?;
    require(settings.reserved == 0, invalid)?;
    ClientSettings::new(escape, take_control).map_err(|_| invalid)
}

/// An enabled key's control byte; a disabled key's byte must be zero.
/// `None` is an invalid key.
fn key(enabled: u8, byte: u8) -> Option<Option<ControlKey>> {
    match enabled {
        0 => (byte == 0).then_some(None),
        1 => ControlKey::from_byte(byte).map(Some),
        _ => None,
    }
}

fn limits(limits: native::Limits) -> Result<native::Limits, Failure> {
    require(
        limits.reserved == 0,
        invalid("limits.reserved must be zero"),
    )?;
    require(
        honoured(&limits),
        Failure::new(
            UNSUPPORTED_LIMIT,
            "limits must be effective values this release supports",
        ),
    )?;
    Ok(limits)
}

/// Every limit is an effective value, never zero-as-default (only
/// `max_workers` may be zero), and leaves room for this release's own
/// requirements: its instance allocation and its workers. A delivery holds
/// a whole record of `metadata_record_bytes`, so none is ever oversized.
fn honoured(limits: &native::Limits) -> bool {
    let values = [
        limits.client_bytes,
        limits.request_bytes,
        limits.event_bytes,
        limits.progress_bytes,
        limits.drain_payload_bytes,
        limits.request_records.into(),
        limits.event_records.into(),
        limits.progress_records.into(),
        limits.deliveries.into(),
        limits.operations.into(),
        limits.chunk_bytes.into(),
        limits.drain_records.into(),
        limits.wire_frame_bytes.into(),
        limits.metadata_record_bytes.into(),
        limits.title_bytes.into(),
        limits.notification_title_bytes.into(),
        limits.notification_body_bytes.into(),
        limits.operation_ms.into(),
        limits.kill_ms.into(),
        limits.restoration_stall_ms.into(),
        limits.restoration_total_ms.into(),
        limits.delivery_stall_ms.into(),
        limits.leave_ms.into(),
        limits.cleanup_ms.into(),
        limits.start_ms.into(),
    ];
    !values.contains(&0)
        && limits.drain_payload_bytes >= limits.metadata_record_bytes.into()
        && limits.client_bytes >= REQUIREMENTS.instance_bytes
        && limits.max_workers >= REQUIREMENTS.workers
}
