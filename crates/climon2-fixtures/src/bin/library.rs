//! Library-layer test host (#75, #84): loads one version's client library
//! from a climon2 executable home through the shared validated loader
//! (`climon2_platform::library`) and prints what it observed as one JSON
//! object: the loader's refusal, or what the library shows through the frozen
//! C ABI, its bootstrap and function table, and whether a second load of the
//! version shared the first; then the client library files the process has
//! loaded, and its environment.
//!
//! It never makes a Rust call into the library. It reports observations, not
//! verdicts: tests decide what is correct. Nothing is unloaded, as the
//! embedding contract requires.

use climon2_core::{
    VersionName,
    layout::{ExecutableHome, client_library_name},
    native::{self, Bootstrap, Bytes, Call, Description, Interface, Tokens},
};
use climon2_platform::library::Loader;
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    ffi::{CStr, OsString, c_void},
    mem::{MaybeUninit, size_of},
    path::PathBuf,
    ptr::{self, NonNull},
};

const FILL: u8 = 0xa5;
const UNTOUCHED_DESCRIPTOR: i32 = 0x5a5a_5a5a;
const OTHER_REVISIONS: [u32; 3] = [0, 2, u32::MAX];

fn main() {
    let (home, version) = arguments().unwrap_or_else(|| {
        eprintln!("usage: climon2-library-host <absolute executable home> <version>");
        std::process::exit(2);
    });
    let mut report = load(home, &version);
    report["images"] = json!(images());
    report["environment"] = json!(
        std::env::vars_os()
            .map(|(name, value)| (lossy(name), lossy(value)))
            .collect::<BTreeMap<_, _>>()
    );
    println!("{report}");
}

fn arguments() -> Option<(ExecutableHome, VersionName)> {
    let mut arguments = std::env::args_os().skip(1);
    let home = ExecutableHome::new(&PathBuf::from(arguments.next()?)).ok()?;
    let version = arguments.next()?.to_str()?.parse().ok()?;
    arguments.next().is_none().then_some((home, version))
}

fn lossy(text: OsString) -> String {
    text.to_string_lossy().into_owned()
}

struct Library {
    bootstrap: Bootstrap,
}

/// What the loader answered for `version`, twice.
fn load(home: ExecutableHome, version: &VersionName) -> Value {
    let loader = Loader::new(home);
    match loader.load(version) {
        Ok(library) => {
            let mut report = observe(Library {
                bootstrap: library.bootstrap(),
            });
            let again = loader.load(version);
            report["shared"] = json!(again.is_ok_and(|again| ptr::eq(again, library)));
            report
        }
        Err(error) => json!({
            "error": { "failure": format!("{:?}", error.failure()), "message": error.to_string() },
        }),
    }
}

/// Every loaded image named like a client library, by the path it was
/// loaded from.
fn images() -> Vec<String> {
    let name = format!("/{}", client_library_name());
    loaded_images()
        .into_iter()
        .filter(|image| image.ends_with(&name))
        .collect()
}

#[cfg(target_os = "linux")]
fn loaded_images() -> Vec<String> {
    unsafe extern "C" fn collect(
        info: *mut libc::dl_phdr_info,
        _: usize,
        images: *mut c_void,
    ) -> libc::c_int {
        // SAFETY: dl_iterate_phdr passes valid information and our Vec.
        let (name, images) = unsafe { ((*info).dlpi_name, &mut *images.cast::<Vec<String>>()) };
        if !name.is_null() {
            // SAFETY: a nonnull object name is NUL-terminated.
            images.push(
                unsafe { CStr::from_ptr(name) }
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        0
    }
    let mut images = Vec::<String>::new();
    // SAFETY: the callback only reads the information and appends to `images`.
    unsafe { libc::dl_iterate_phdr(Some(collect), (&raw mut images).cast()) };
    images
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    /// `<mach-o/dyld.h>`: how many images dyld has loaded.
    fn _dyld_image_count() -> u32;
    /// `<mach-o/dyld.h>`: the path loaded image `index` was loaded from, or
    /// null once it is gone.
    fn _dyld_get_image_name(index: u32) -> *const std::ffi::c_char;
}

#[cfg(target_os = "macos")]
fn loaded_images() -> Vec<String> {
    // SAFETY: dyld's image list, read by index.
    (0..unsafe { _dyld_image_count() })
        .map(|index| unsafe { _dyld_get_image_name(index) })
        .filter(|name| !name.is_null())
        // SAFETY: a nonnull image name is NUL-terminated.
        .map(|name| {
            unsafe { CStr::from_ptr(name) }
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

/// The file of the loaded image that contains `address`.
fn image(address: *const c_void) -> Value {
    let mut info = MaybeUninit::<libc::Dl_info>::zeroed();
    // SAFETY: dladdr only writes the supplied Dl_info.
    if unsafe { libc::dladdr(address, info.as_mut_ptr()) } == 0 {
        return Value::Null;
    }
    // SAFETY: dladdr succeeded, so dli_fname is a NUL-terminated path.
    let name = unsafe { CStr::from_ptr(info.assume_init().dli_fname) };
    json!(name.to_string_lossy())
}

fn text(bytes: Bytes) -> Value {
    if bytes.data.is_null() || bytes.length > 65_536 {
        return json!({ "null": bytes.data.is_null(), "length": bytes.length });
    }
    // SAFETY: a nonnull buffer covers `length` readable bytes by contract.
    let slice = unsafe { std::slice::from_raw_parts(bytes.data, bytes.length as usize) };
    json!(String::from_utf8_lossy(slice))
}

fn tokens(list: Tokens) -> Value {
    let mut values = Vec::new();
    if !list.data.is_null() && list.count <= 64 {
        for index in 0..list.count as usize {
            // SAFETY: a nonnull list covers `count` readable entries by contract.
            values.push(text(unsafe { *list.data.add(index) }));
        }
    }
    json!({ "null": list.data.is_null(), "count": list.count, "tokens": values })
}

/// What the bootstrap wrote into a sentinel output for `revision`.
fn request(library: &Library, revision: u32) -> (u32, *const Description, Value) {
    let sentinel = NonNull::<Description>::dangling().as_ptr().cast_const();
    let mut description = sentinel;
    // SAFETY: the output is writable storage for one pointer.
    let status = unsafe { (library.bootstrap)(revision, &mut description) };
    let output = if description.is_null() {
        "null"
    } else if description == sentinel {
        "untouched"
    } else {
        "set"
    };
    (status, description, json!(output))
}

fn observe(library: Library) -> Value {
    let (status, description, output) = request(&library, native::REVISION);
    let mut report = json!({
        "image": image(library.bootstrap as *const c_void),
        "status": status,
        "output": output,
        "other_revisions": OTHER_REVISIONS.map(|revision| {
            let (status, _, output) = request(&library, revision);
            json!({ "revision": revision, "status": status, "output": output })
        }),
        // SAFETY: a null output pointer is the documented invalid-argument case.
        "null_output_status": unsafe { (library.bootstrap)(native::REVISION, ptr::null_mut()) },
    });
    if status == native::SUCCESS && !description.is_null() {
        report["stable"] = json!(request(&library, native::REVISION).1 == description);
        report["description_image"] = image(description.cast());
        // SAFETY: success returns an immutable description that lives until exit.
        report["description"] = describe(unsafe { &*description });
    }
    report
}

fn describe(description: &Description) -> Value {
    let requirements = description.requirements;
    json!({
        "revision": description.revision,
        "reserved": description.reserved,
        "release": text(description.release),
        "capabilities": tokens(description.capabilities),
        "requirements": {
            "release_bytes": requirements.release_bytes,
            "instance_bytes": requirements.instance_bytes,
            "owner_stack_bytes": requirements.owner_stack_bytes,
            "worker_stack_bytes": requirements.worker_stack_bytes,
            "workers": requirements.workers,
            "instance_descriptors": requirements.instance_descriptors,
            "load_descriptors": requirements.load_descriptors,
            "reserved": requirements.reserved,
        },
        "functions": functions(&description.functions),
    })
}

fn call(call: Call) -> Map<String, Value> {
    let error = call.error;
    let mut result = Map::new();
    result.insert("code".into(), json!(call.code));
    result.insert("reserved".into(), json!(call.reserved));
    result.insert("origin".into(), json!(error.origin));
    result.insert("error_reserved".into(), json!(error.reserved));
    result.insert("error_code".into(), text(error.code));
    result.insert("message".into(), text(error.message));
    result
}

/// Output storage filled with a pattern, to see whether a call clears it.
fn filled<T>() -> MaybeUninit<T> {
    let mut value = MaybeUninit::<T>::uninit();
    // SAFETY: the pointer covers exactly one T.
    unsafe {
        value
            .as_mut_ptr()
            .cast::<u8>()
            .write_bytes(FILL, size_of::<T>())
    };
    value
}

fn state<T>(value: &MaybeUninit<T>) -> &'static str {
    // SAFETY: every byte was initialized by `filled` or by the library.
    let bytes = unsafe { std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), size_of::<T>()) };
    if bytes.iter().all(|byte| *byte == 0) {
        "zeroed"
    } else if bytes.iter().all(|byte| *byte == FILL) {
        "untouched"
    } else {
        "modified"
    }
}

fn with_output<T>(mut result: Map<String, Value>, output: &MaybeUninit<T>) -> Value {
    result.insert("output".into(), json!(state(output)));
    Value::Object(result)
}

/// Calls every entry once without an instance, as an embedder would before
/// creating one; null handles are never dereferenced by unsupported calls.
fn functions(table: &Interface) -> Value {
    let startup = MaybeUninit::<native::Startup>::zeroed();
    let request = MaybeUninit::<native::Request>::zeroed();
    let mut entries = Map::new();
    let mut null_outputs = Map::new();
    let mut entry = |name: &str, value: Option<Value>| {
        entries.insert(name.into(), value.unwrap_or(Value::Null));
    };
    // SAFETY (all calls below): every pointer is null or valid caller storage
    // of the declared type, and the table entries come from the description.
    entry(
        "create",
        table.create.map(|create| {
            let mut out = filled::<native::Created>();
            let result = unsafe { create(startup.as_ptr(), out.as_mut_ptr()) };
            null_outputs.insert(
                "create".into(),
                json!(unsafe { create(startup.as_ptr(), ptr::null_mut()) }.code),
            );
            with_output(call(result), &out)
        }),
    );
    entry(
        "run",
        table.run.map(|run| {
            let mut out = filled::<native::RunReport>();
            let result = unsafe { run(ptr::null_mut(), out.as_mut_ptr()) };
            null_outputs.insert(
                "run".into(),
                json!(unsafe { run(ptr::null_mut(), ptr::null_mut()) }.code),
            );
            with_output(call(result), &out)
        }),
    );
    entry(
        "destroy",
        table
            .destroy
            .map(|destroy| Value::Object(call(unsafe { destroy(ptr::null_mut()) }))),
    );
    entry(
        "submit",
        table.submit.map(|submit| {
            Value::Object(call(unsafe { submit(ptr::null_mut(), request.as_ptr()) }))
        }),
    );
    entry(
        "drain",
        table.drain.map(|drain| {
            let mut out = filled::<native::Drained>();
            let result = unsafe { drain(ptr::null_mut(), out.as_mut_ptr()) };
            null_outputs.insert(
                "drain".into(),
                json!(unsafe { drain(ptr::null_mut(), ptr::null_mut()) }.code),
            );
            with_output(call(result), &out)
        }),
    );
    entry(
        "release_delivery",
        table
            .release_delivery
            .map(|release| Value::Object(call(unsafe { release(ptr::null_mut()) }))),
    );
    entry(
        "shutdown",
        table
            .shutdown
            .map(|shutdown| Value::Object(call(unsafe { shutdown(ptr::null_mut()) }))),
    );
    entry(
        "release_communication",
        table
            .release_communication
            .map(|release| Value::Object(call(unsafe { release(ptr::null_mut()) }))),
    );
    entry(
        "release_control",
        table
            .release_control
            .map(|release| Value::Object(call(unsafe { release(ptr::null_mut()) }))),
    );
    entry(
        "notification",
        table.notification.map(|notification| {
            let mut descriptor = UNTOUCHED_DESCRIPTOR;
            let mut result = call(unsafe { notification(ptr::null_mut(), &mut descriptor) });
            null_outputs.insert(
                "notification".into(),
                json!(unsafe { notification(ptr::null_mut(), ptr::null_mut()) }.code),
            );
            result.insert("descriptor".into(), json!(descriptor));
            Value::Object(result)
        }),
    );
    entry(
        "snapshot",
        table.snapshot.map(|snapshot| {
            let mut out = filled::<native::Metrics>();
            let result = unsafe { snapshot(ptr::null_mut(), out.as_mut_ptr()) };
            null_outputs.insert(
                "snapshot".into(),
                json!(unsafe { snapshot(ptr::null_mut(), ptr::null_mut()) }.code),
            );
            with_output(call(result), &out)
        }),
    );
    json!({ "entries": entries, "null_outputs": null_outputs })
}
