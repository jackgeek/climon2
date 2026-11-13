//! The client library's frozen C ABI (native revision 1): the exported
//! bootstrap, the immutable description it returns, the function table's
//! instance lifecycle (#85) and passive monitor connections (#93).
//!
//! Only `climon2_client_get_interface` is exported; Rust layouts, unwinding and
//! allocators never cross it. Every entry contains panics and reports them as
//! an explicit `CM1_INTERNAL_ERROR`; the library's own panic hook is silent, so
//! nothing reaches the standard streams.

use climon2_core::native::{
    self, Bytes, Call, Communication, Control, Created, Delivery, Description, Drained, Instance,
    Interface, Metrics, Request, Requirements, RunReport, Startup, Tokens,
};
use failure::{Failure, INVALID_ARGUMENT, PANIC};
use std::{
    panic::{self, AssertUnwindSafe},
    ptr,
    sync::{Arc, Once},
};

mod connection;
mod failure;
mod families;
mod guid;
mod handshake;
#[cfg(feature = "test-hooks")]
mod hooks;
mod instance;
mod link;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[path = "library/linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "library/macos.rs"]
mod platform;
mod records;
mod request;
mod startup;
mod stream;

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

struct Immutable<T>(T);

// SAFETY: the value is immutable and points only to static data.
unsafe impl<T> Sync for Immutable<T> {}

/// The families this release relays, as the description lists them.
static CAPABILITIES: Immutable<[Bytes; 3]> = Immutable([
    failure::bytes(families::RELAYED[0]),
    failure::bytes(families::RELAYED[1]),
    failure::bytes(families::RELAYED[2]),
]);

static DESCRIPTION: Immutable<Description> = Immutable(Description {
    revision: native::REVISION,
    reserved: 0,
    release: failure::bytes(RELEASE),
    capabilities: Tokens {
        data: CAPABILITIES.0.as_ptr(),
        count: CAPABILITIES.0.len() as u64,
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

/// A null handle or output pointer: the one invalid native address the library
/// can detect. Other stale or wrong pointers remain caller misuse.
const NULL: Failure = Failure::new(
    INVALID_ARGUMENT,
    "a required handle or output pointer is null",
);

static SILENT_PANICS: Once = Once::new();

/// The library's panics are contained and reported as results, so its own
/// panic hook (this image's, not the host's) writes nothing.
fn silence_panics() {
    panic::set_hook(Box::new(|_| {}));
}

/// The `test-hooks` panic point at an exported call's boundary.
fn hook(_call: &'static str) {
    #[cfg(feature = "test-hooks")]
    hooks::panic_at(_call);
}

/// An exported call's work with every panic contained: no unwinding leaves.
fn contained(work: impl FnOnce() -> Result<(), Failure>) -> Call {
    failure::call(panic::catch_unwind(AssertUnwindSafe(work)).unwrap_or(Err(PANIC)))
}

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
    SILENT_PANICS.call_once(silence_panics);
    // SAFETY: the caller supplies null or writable storage for one pointer.
    let Some(output) = (unsafe { description.as_mut() }) else {
        return native::INVALID_ARGUMENT;
    };
    *output = ptr::null();
    panic::catch_unwind(|| {
        hook("bootstrap");
        describe(requested_revision)
    })
    .map_or(native::INTERNAL_ERROR, |(status, value)| {
        *output = value;
        status
    })
}

fn describe(requested_revision: u32) -> (u32, *const Description) {
    if requested_revision == native::REVISION {
        (native::SUCCESS, &raw const DESCRIPTION.0)
    } else {
        (native::UNSUPPORTED_REVISION, ptr::null())
    }
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

/// # Safety
///
/// `startup` must be null or a valid startup whose buffers cover their
/// lengths; `created` null or writable storage.
unsafe extern "C" fn create(startup: *const Startup, created: *mut Created) -> Call {
    // SAFETY: the caller supplies null or writable output storage.
    unsafe { clear(created) };
    contained(|| {
        hook("create");
        // SAFETY: the caller supplies null or valid pointers.
        let (Some(startup), Some(created)) =
            (unsafe { startup.as_ref() }, unsafe { created.as_mut() })
        else {
            return Err(NULL);
        };
        // SAFETY: the startup's buffers cover their lengths for this call.
        let startup = unsafe { startup::Startup::parse(startup) }?;
        let effective_limits = startup.limits;
        let (owner, communication, control) = instance::create(startup)?;
        *created = Created {
            instance: Box::into_raw(owner).cast(),
            communication: Box::into_raw(communication).cast(),
            control: Box::into_raw(control).cast(),
            effective_limits,
        };
        Ok(())
    })
}

/// # Safety
///
/// `instance` must be null or this library's live instance; `report` null or
/// writable storage.
unsafe extern "C" fn run(instance: *mut Instance, report: *mut RunReport) -> Call {
    // SAFETY: the caller supplies null or writable output storage.
    unsafe { clear(report) };
    contained(|| {
        // SAFETY: the caller supplies null or valid pointers.
        let (Some(owner), Some(report)) = (
            unsafe { instance.cast::<instance::Owner>().as_mut() },
            unsafe { report.as_mut() },
        ) else {
            return Err(NULL);
        };
        let (result, value) = owner.run()?;
        *report = value;
        result
    })
}

/// # Safety
///
/// `instance` must be null or this library's live instance.
unsafe extern "C" fn destroy(instance: *mut Instance) -> Call {
    contained(|| {
        let owner = instance.cast::<instance::Owner>();
        // SAFETY: the caller supplies null or a live instance.
        let shared = unsafe { owner.as_ref() }.map(|owner| Arc::clone(&owner.shared));
        // SAFETY: as above; only the owner thread passes `destroy`'s check.
        shared
            .ok_or(NULL)?
            .guard("destroy", || unsafe { &*owner }.destroy())?;
        // SAFETY: destroy succeeded, so nothing else uses the instance.
        drop(unsafe { Box::from_raw(owner) });
        Ok(())
    })
}

/// # Safety
///
/// `communication` must be null or this library's live handle; `request`
/// null or a valid request whose active member's buffers cover their lengths.
unsafe extern "C" fn submit(communication: *mut Communication, request: *const Request) -> Call {
    contained(|| {
        // SAFETY: the caller supplies null or valid pointers.
        let (Some(handle), Some(request)) = (
            unsafe { communication.cast::<instance::Communication>().as_ref() },
            unsafe { request.as_ref() },
        ) else {
            return Err(NULL);
        };
        handle
            .shared
            .guard("submit", || handle.shared.submit(request))
    })
}

/// # Safety
///
/// `communication` must be null or this library's live handle; `drained` null
/// or writable storage.
unsafe extern "C" fn drain(communication: *mut Communication, drained: *mut Drained) -> Call {
    // SAFETY: the caller supplies null or writable output storage.
    unsafe { clear(drained) };
    contained(|| {
        // SAFETY: the caller supplies null or valid pointers.
        let (Some(handle), Some(drained)) = (
            unsafe { communication.cast::<instance::Communication>().as_ref() },
            unsafe { drained.as_mut() },
        ) else {
            return Err(NULL);
        };
        *drained = handle.shared.guard("drain", || handle.shared.drain())?;
        Ok(())
    })
}

/// # Safety
///
/// `delivery` must be null or this library's unreleased delivery.
unsafe extern "C" fn release_delivery(delivery: *mut Delivery) -> Call {
    contained(|| {
        let delivery = delivery.cast::<instance::Delivery>();
        // SAFETY: the caller supplies null or an unreleased delivery.
        let shared = unsafe { delivery.as_ref() }.map(|delivery| Arc::clone(&delivery.shared));
        shared.ok_or(NULL)?.guard("release_delivery", || {
            // SAFETY: released exactly once, after its readers finished.
            instance::release(unsafe { Box::from_raw(delivery) });
            Ok(())
        })
    })
}

/// # Safety
///
/// `control` must be null or this library's live handle.
unsafe extern "C" fn shutdown(control: *mut Control) -> Call {
    contained(|| {
        // SAFETY: the caller supplies null or a live handle.
        let handle = unsafe { control.cast::<instance::Control>().as_ref() }.ok_or(NULL)?;
        handle.shared.guard("shutdown", || handle.shared.shutdown())
    })
}

/// # Safety
///
/// `communication` must be null or this library's live handle, no longer in
/// concurrent use.
unsafe extern "C" fn release_communication(communication: *mut Communication) -> Call {
    contained(|| {
        let handle = communication.cast::<instance::Communication>();
        // SAFETY: the caller supplies null or a live handle.
        let shared = unsafe { handle.as_ref() }.map(|handle| Arc::clone(&handle.shared));
        shared.ok_or(NULL)?.guard("release_communication", || {
            // SAFETY: as above.
            unsafe { &*handle }.shared.release_communication()
        })?;
        // SAFETY: released, so the caller makes no later call with it.
        drop(unsafe { Box::from_raw(handle) });
        Ok(())
    })
}

/// # Safety
///
/// `control` must be null or this library's live handle, no longer in
/// concurrent use.
unsafe extern "C" fn release_control(control: *mut Control) -> Call {
    contained(|| {
        let handle = control.cast::<instance::Control>();
        // SAFETY: the caller supplies null or a live handle.
        let shared = unsafe { handle.as_ref() }.map(|handle| Arc::clone(&handle.shared));
        shared.ok_or(NULL)?.guard("release_control", || {
            // SAFETY: as above.
            unsafe { &*handle }.shared.release_control()
        })?;
        // SAFETY: released, so the caller makes no later call with it.
        drop(unsafe { Box::from_raw(handle) });
        Ok(())
    })
}

/// A failed notification lookup writes -1.
///
/// # Safety
///
/// `communication` must be null or this library's live handle; `descriptor`
/// null or writable storage.
unsafe extern "C" fn notification(communication: *mut Communication, descriptor: *mut i32) -> Call {
    // SAFETY: the caller supplies null or writable storage for one descriptor.
    if let Some(descriptor) = unsafe { descriptor.as_mut() } {
        *descriptor = -1;
    }
    contained(|| {
        // SAFETY: the caller supplies null or valid pointers.
        let (Some(handle), Some(descriptor)) = (
            unsafe { communication.cast::<instance::Communication>().as_ref() },
            unsafe { descriptor.as_mut() },
        ) else {
            return Err(NULL);
        };
        *descriptor = handle
            .shared
            .guard("notification", || Ok(handle.shared.descriptor()))?;
        Ok(())
    })
}

/// # Safety
///
/// `communication` must be null or this library's live handle; `metrics` null
/// or writable storage.
unsafe extern "C" fn snapshot(communication: *mut Communication, metrics: *mut Metrics) -> Call {
    // SAFETY: the caller supplies null or writable output storage.
    unsafe { clear(metrics) };
    contained(|| {
        // SAFETY: the caller supplies null or valid pointers.
        let (Some(handle), Some(metrics)) = (
            unsafe { communication.cast::<instance::Communication>().as_ref() },
            unsafe { metrics.as_mut() },
        ) else {
            return Err(NULL);
        };
        *metrics = handle
            .shared
            .guard("snapshot", || Ok(handle.shared.snapshot()))?;
        Ok(())
    })
}
