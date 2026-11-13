//! Library-layer test host (#75, #85): loads one client library by absolute
//! path through the frozen C ABI and prints what it observed as one JSON
//! object.
//!
//! It uses only `dlopen(RTLD_NOW | RTLD_LOCAL)`, `dlsym` of the bootstrap and
//! the returned function table; never a Rust call into the library. It reports
//! observations, not verdicts: tests decide what is correct. The library is
//! never unloaded, as the embedding contract requires.
//!
//! `climon2-library-host <library>` describes the library.
//! `climon2-library-host <library> <scenario> <json>` drives instances through
//! one scenario (`lifecycle`, `create`, `fault`, `workers`, `independence`,
//! `ambient` or `race`) with the instrumentation every Library-layer test
//! shares: caller startup memory the library can neither keep nor free
//! (`startup`), an ownership ledger and a timeline of every call and its
//! thread (`abi`), and descriptor and process-state probes (`probe`).

#[path = "library/abi.rs"]
mod abi;
#[path = "library/faults.rs"]
mod faults;
#[path = "library/isolation.rs"]
mod isolation;
#[path = "library/probe.rs"]
mod probe;
#[path = "library/scenarios.rs"]
mod scenarios;
#[path = "library/startup.rs"]
mod startup;

use abi::{call, filled, state, text};
use climon2_core::native::{self, Bootstrap, Description, Interface, Tokens};
use serde_json::{Map, Value, json};
use std::{
    ffi::{CStr, CString, c_void},
    mem::MaybeUninit,
    os::unix::ffi::OsStrExt,
    ptr::{self, NonNull},
};

const UNTOUCHED_DESCRIPTOR: i32 = 0x5a5a_5a5a;
const OTHER_REVISIONS: [u32; 3] = [0, 2, u32::MAX];

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let Some(path) = arguments.next() else {
        eprintln!("usage: climon2-library-host <absolute library path> [<scenario> <json>]");
        std::process::exit(2);
    };
    let scenario = arguments.next().map(|scenario| {
        let input = arguments.next().expect("a scenario's JSON input");
        let input = serde_json::from_slice(input.as_bytes()).expect("JSON input");
        (scenario.to_string_lossy().into_owned(), input)
    });
    let report = match (
        load(&CString::new(path.as_bytes()).expect("path without NUL")),
        scenario,
    ) {
        (Ok(library), Some((name, input))) => drive(&library, &name, input),
        (Ok(library), None) => observe(library),
        (Err(error), _) => json!({ "error": error }),
    };
    println!("{report}");
}

/// Bootstraps revision 1, once more if the first bootstrap failed explicitly
/// with `CM1_INTERNAL_ERROR` (a contained panic), then runs one scenario.
fn drive(library: &Library, name: &str, input: Value) -> Value {
    let mut bootstrap = Vec::new();
    let description = loop {
        let (status, description, output) = request(library, native::REVISION);
        bootstrap.push(json!({ "status": status, "output": output }));
        if status != native::INTERNAL_ERROR || bootstrap.len() > 1 {
            break description;
        }
    };
    assert!(!description.is_null(), "bootstrap failed: {bootstrap:?}");
    // SAFETY: success returns an immutable description that lives until exit.
    let description: &'static Description = unsafe { &*description };
    let setup = scenarios::Setup {
        api: abi::Api::new(&description.functions),
        release: abi::text(description.release)
            .as_str()
            .expect("UTF-8 release text")
            .to_owned(),
        input,
    };
    let mut report = match name {
        "lifecycle" => scenarios::lifecycle(&setup),
        "create" => scenarios::creates(&setup),
        "fault" => faults::fault(&setup),
        "workers" => faults::workers(&setup),
        "independence" => isolation::independence(&setup),
        "ambient" => isolation::ambient(&setup),
        "race" => isolation::race(&setup),
        _ => panic!("unknown scenario {name}"),
    };
    if let Some(object) = report.as_object_mut() {
        object.insert("bootstrap".into(), Value::Array(bootstrap));
    } else {
        report = json!({ "cases": report, "bootstrap": bootstrap });
    }
    report
}

struct Library {
    bootstrap: Bootstrap,
}

fn dlerror() -> String {
    // SAFETY: dlerror returns null or a NUL-terminated message owned by the loader.
    let message = unsafe { libc::dlerror() };
    if message.is_null() {
        return "unknown loader error".into();
    }
    // SAFETY: nonnull dlerror results are NUL-terminated strings.
    unsafe { CStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned()
}

fn load(path: &CStr) -> Result<Library, String> {
    // SAFETY: path is NUL-terminated; the handle is deliberately never closed.
    let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if handle.is_null() {
        return Err(dlerror());
    }
    // SAFETY: handle is a live dlopen handle and the name is NUL-terminated.
    let symbol = unsafe { libc::dlsym(handle, native::BOOTSTRAP.as_ptr()) };
    if symbol.is_null() {
        return Err(dlerror());
    }
    // SAFETY: the frozen bootstrap symbol has the `Bootstrap` signature.
    let bootstrap = unsafe { std::mem::transmute::<*mut c_void, Bootstrap>(symbol) };
    Ok(Library { bootstrap })
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
