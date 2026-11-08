//! The client library's frozen C ABI (native revision 1): the exported
//! bootstrap and the immutable description it returns.
//!
//! Only `climon2_client_get_interface` is exported; Rust layouts, unwinding and
//! allocators never cross it. The bootstrap and the table entries do no
//! fallible work and allocate nothing. Instance calls are not implemented yet
//! (#85, #93): every table entry fails with `CM1_UNSUPPORTED_OPERATION`.

use climon2_core::native::{
    self, Bytes, Call, Communication, Control, Created, Delivery, Description, Drained, Error,
    Instance, Interface, Metrics, Request, Requirements, RunReport, Startup, Tokens,
};
use std::ptr;

/// The release identity: the version folder name, compiled in.
const RELEASE: &str = env!("CARGO_PKG_VERSION");

const MIB: u64 = 1024 * 1024;

/// Provisional bounded requirements until the measurement job (#121) records
/// qualified ones: the default `monitor.limits` reservations, plus 1 MiB of
/// instance-local allocation.
const REQUIREMENTS: Requirements = Requirements {
    release_bytes: 64 * MIB,
    instance_bytes: MIB,
    owner_stack_bytes: 2 * MIB,
    worker_stack_bytes: 2 * MIB,
    workers: 2,
    instance_descriptors: 8,
    load_descriptors: 4,
    reserved: 0,
};

const UNSUPPORTED_CODE: &str = "unsupported_operation";
const UNSUPPORTED_MESSAGE: &str =
    "this client library implements only the revision 1 bootstrap and description";

const UNSUPPORTED: Call = Call {
    code: native::UNSUPPORTED_OPERATION,
    reserved: 0,
    error: Error {
        origin: native::ORIGIN_LIBRARY,
        reserved: 0,
        code: Bytes {
            data: UNSUPPORTED_CODE.as_ptr(),
            length: UNSUPPORTED_CODE.len() as u64,
        },
        message: Bytes {
            data: UNSUPPORTED_MESSAGE.as_ptr(),
            length: UNSUPPORTED_MESSAGE.len() as u64,
        },
    },
};

struct Shared(Description);

// SAFETY: the description is immutable and points only to static data.
unsafe impl Sync for Shared {}

static DESCRIPTION: Shared = Shared(Description {
    revision: native::REVISION,
    reserved: 0,
    release: Bytes {
        data: RELEASE.as_ptr(),
        length: RELEASE.len() as u64,
    },
    // No instance behaviour is implemented yet, so no capability is advertised.
    capabilities: Tokens {
        data: ptr::null(),
        count: 0,
    },
    requirements: REQUIREMENTS,
    functions: Interface {
        create: Some(create),
        run: Some(run),
        destroy: Some(destroy),
        submit: Some(submit),
        drain: Some(drain),
        release_delivery: Some(release_delivery),
        shutdown: Some(shutdown),
        release_communication: Some(release_communication),
        release_control: Some(release_control),
        notification: Some(notification),
        snapshot: Some(snapshot),
    },
});

/// Returns revision 1's immutable description, which lives until process exit.
///
/// # Safety
///
/// `description` must be null or point to writable storage for one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn climon2_client_get_interface(
    requested_revision: u32,
    description: *mut *const Description,
) -> u32 {
    climon2_core::test_hook_marker!();
    if description.is_null() {
        return native::INVALID_ARGUMENT;
    }
    let (status, value) = if requested_revision == native::REVISION {
        (native::SUCCESS, &DESCRIPTION.0 as *const Description)
    } else {
        (native::UNSUPPORTED_REVISION, ptr::null())
    };
    // SAFETY: the caller supplies writable storage for one pointer.
    unsafe { description.write(value) };
    status
}

/// Failed calls clear their output structs; a null output is never written.
///
/// # Safety
///
/// `output` must be null or point to writable storage for one `T`.
unsafe fn clear<T>(output: *mut T) {
    if !output.is_null() {
        // SAFETY: guaranteed by the caller.
        unsafe { output.write_bytes(0, 1) };
    }
}

unsafe extern "C" fn create(_: *const Startup, created: *mut Created) -> Call {
    // SAFETY: the caller supplies null or writable output storage.
    unsafe { clear(created) };
    UNSUPPORTED
}

unsafe extern "C" fn run(_: *mut Instance, report: *mut RunReport) -> Call {
    // SAFETY: the caller supplies null or writable output storage.
    unsafe { clear(report) };
    UNSUPPORTED
}

unsafe extern "C" fn destroy(_: *mut Instance) -> Call {
    UNSUPPORTED
}

unsafe extern "C" fn submit(_: *mut Communication, _: *const Request) -> Call {
    UNSUPPORTED
}

unsafe extern "C" fn drain(_: *mut Communication, drained: *mut Drained) -> Call {
    // SAFETY: the caller supplies null or writable output storage.
    unsafe { clear(drained) };
    UNSUPPORTED
}

unsafe extern "C" fn release_delivery(_: *mut Delivery) -> Call {
    UNSUPPORTED
}

unsafe extern "C" fn shutdown(_: *mut Control) -> Call {
    UNSUPPORTED
}

unsafe extern "C" fn release_communication(_: *mut Communication) -> Call {
    UNSUPPORTED
}

unsafe extern "C" fn release_control(_: *mut Control) -> Call {
    UNSUPPORTED
}

/// A failed notification lookup writes -1.
unsafe extern "C" fn notification(_: *mut Communication, descriptor: *mut i32) -> Call {
    if !descriptor.is_null() {
        // SAFETY: the caller supplies writable storage for one descriptor.
        unsafe { descriptor.write(-1) };
    }
    UNSUPPORTED
}

unsafe extern "C" fn snapshot(_: *mut Communication, metrics: *mut Metrics) -> Call {
    // SAFETY: the caller supplies null or writable output storage.
    unsafe { clear(metrics) };
    UNSUPPORTED
}
