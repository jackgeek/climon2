//! The function table as the host calls it, instrumented: every call is
//! recorded on the timeline with its thread, and every object the library
//! hands over (instance, communication and control handles, deliveries) is
//! entered in the ownership ledger until a successful release through the same
//! table. The host never frees, reuses or releases twice what it does not own,
//! and reports its observations as JSON; tests decide what is correct.

use climon2_core::native::{
    self, Bytes, Call, Communication, Control, Delivery, Drained, Error, Instance, Interface,
    Metrics, Record, RunReport, Startup, Terminal,
};
use serde_json::{Map, Value, json};
use std::{
    mem::MaybeUninit,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

pub const FILL: u8 = 0xa5;

/// A pointer the library owns, carried between host threads.
pub struct Ptr<T>(pub *mut T);

impl<T> Clone for Ptr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Ptr<T> {}

// SAFETY: the host only passes these pointers back to the library, whose
// handles are thread-safe where the contract says so.
unsafe impl<T> Send for Ptr<T> {}
// SAFETY: as above.
unsafe impl<T> Sync for Ptr<T> {}

#[derive(Clone, Copy)]
pub struct Handles {
    pub instance: Ptr<Instance>,
    pub communication: Ptr<Communication>,
    pub control: Ptr<Control>,
}

/// What one drain returned, with the delivery kept for its release.
pub struct Drain {
    pub kind: u32,
    pub delivery: Option<Ptr<Delivery>>,
    pub observed: Value,
    drained: Retained,
}

/// A drain's output, whose records live until the delivery's release and
/// whose terminal evidence lives until destroy.
struct Retained(Drained);

// SAFETY: the host reads the retained objects only within their lifetimes.
unsafe impl Send for Retained {}

impl Drain {
    /// What the retained output holds now. Only valid before the delivery is
    /// released, and terminal evidence before destroy.
    pub fn observe_again(&self) -> Value {
        observe(&self.drained.0)
    }
}

struct Object {
    kind: &'static str,
    address: usize,
    released_by: Option<&'static str>,
}

/// Every library-owned object the host received, and how it was released.
#[derive(Default)]
pub struct Ledger(Mutex<Vec<Object>>);

impl Ledger {
    fn own(&self, kind: &'static str, address: usize) {
        self.0.lock().unwrap().push(Object {
            kind,
            address,
            released_by: None,
        });
    }

    fn position(objects: &[Object], kind: &str, address: usize) -> Option<usize> {
        objects.iter().position(|object| {
            object.kind == kind && object.address == address && object.released_by.is_none()
        })
    }

    /// Whether `address` is an owned, unreleased object of `kind`; the host
    /// never passes anything else to a release call.
    fn owned(&self, kind: &str, address: usize) -> bool {
        Self::position(&self.0.lock().unwrap(), kind, address).is_some()
    }

    fn release(&self, kind: &str, address: usize, by: &'static str) {
        let mut objects = self.0.lock().unwrap();
        let index = Self::position(&objects, kind, address).expect("released objects are owned");
        objects[index].released_by = Some(by);
    }

    pub fn report(&self) -> Value {
        let objects = self.0.lock().unwrap();
        json!({
            "objects": objects.iter().map(|object| json!({
                "kind": object.kind,
                "released_by": object.released_by,
            })).collect::<Vec<_>>(),
            "outstanding": objects.iter().filter(|object| object.released_by.is_none()).count(),
        })
    }
}

/// Every call in order, with the thread that made it.
#[derive(Default)]
pub struct Timeline {
    sequence: AtomicU64,
    steps: Mutex<Vec<Value>>,
}

impl Timeline {
    pub fn step(&self, call: &str, code: &Value) {
        let thread = std::thread::current();
        let mut steps = self.steps.lock().unwrap();
        steps.push(json!({
            "seq": self.sequence.fetch_add(1, Ordering::SeqCst),
            "thread": thread.name().unwrap_or("unnamed"),
            "call": call,
            "code": code,
        }));
    }

    pub fn report(&self) -> Value {
        Value::Array(self.steps.lock().unwrap().clone())
    }
}

pub fn text(bytes: Bytes) -> Value {
    if bytes.data.is_null() || bytes.length > 65_536 {
        return json!({ "null": bytes.data.is_null(), "length": bytes.length });
    }
    // SAFETY: a nonnull buffer covers `length` readable bytes by contract.
    let slice = unsafe { std::slice::from_raw_parts(bytes.data, bytes.length as usize) };
    json!(String::from_utf8_lossy(slice))
}

pub fn error(error: Error) -> Value {
    json!({
        "origin": error.origin,
        "reserved": error.reserved,
        "code": text(error.code),
        "message": text(error.message),
    })
}

pub fn call(call: Call) -> Map<String, Value> {
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
pub fn filled<T>() -> MaybeUninit<T> {
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

pub fn state<T>(value: &MaybeUninit<T>) -> &'static str {
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

pub fn terminal(terminal: &Terminal) -> Value {
    json!({
        "generation": terminal.generation,
        "client_outcome": terminal.client_outcome,
        "retry": terminal.retry,
        "leave": terminal.leave,
        "attachment_detached": terminal.attachment_detached,
        "connection_closed": terminal.connection_closed,
        "reserved": terminal.reserved,
        "error": error(terminal.error),
    })
}

fn tokens(list: native::Tokens) -> Value {
    if list.data.is_null() || list.count > 64 {
        return json!({ "null": list.data.is_null(), "count": list.count });
    }
    // SAFETY: a nonnull list covers `count` entries until its delivery's release.
    let entries = unsafe { std::slice::from_raw_parts(list.data, list.count as usize) };
    Value::Array(entries.iter().map(|token| text(*token)).collect())
}

fn optional_text(optional: native::OptionalText) -> Value {
    json!({ "present": optional.present, "reserved": optional.reserved, "value": text(optional.value) })
}

fn command(result: &native::CommandResult) -> Value {
    json!({
        "kind": result.kind,
        "value": result.value,
        "signal_name": optional_text(result.signal_name),
    })
}

fn title(title: &native::Title) -> Value {
    json!({ "state": title.state, "reserved": title.reserved, "value": text(title.value) })
}

fn response(response: &native::Response) -> Value {
    let mut value = json!({
        "request_id": response.request_id,
        "outcome": response.outcome,
        "result_tag": response.result_tag,
        "error": error(response.error),
    });
    // SAFETY (each arm): the result tag names the active member.
    value["result"] = match response.result_tag {
        native::OPENED => {
            let opened = unsafe { response.result.opened };
            json!({
                "connection_id": hex(&opened.connection_id.bytes),
                "selected": tokens(opened.selected),
            })
        }
        native::LEFT => {
            json!({ "attachment_detached": unsafe { response.result.attachment_detached } })
        }
        native::KNOWN_FINAL => command(unsafe { &response.result.known_final }),
        _ => Value::Null,
    };
    value
}

fn metadata(metadata: &native::Metadata) -> Value {
    json!({
        "title": title(&metadata.title),
        "status": {
            "available": metadata.status.available,
            "reserved": metadata.status.reserved,
        },
        "progress": {
            "available": metadata.progress.available,
            "reserved": metadata.progress.reserved,
        },
        "command_result": {
            "present": metadata.command_result.present,
            "reserved": metadata.command_result.reserved,
            "value": command(&metadata.command_result.value),
        },
    })
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A record's header and its active member, decoded by its tag; another
/// tag is reported by tag only.
pub fn record(record: &Record) -> Value {
    let mut value = json!({
        "tag": record.tag,
        "reserved": record.reserved,
        "generation": record.generation,
    });
    // SAFETY (each arm): the tag names the active member.
    match record.tag {
        native::DIAGNOSTIC => value["diagnostic"] = error(unsafe { record.data.diagnostic }),
        native::RESPONSE => value["response"] = response(unsafe { &record.data.response }),
        native::INITIAL_METADATA => value["metadata"] = metadata(unsafe { &record.data.metadata }),
        native::TITLE => value["title"] = title(unsafe { &record.data.title }),
        _ => {}
    }
    value
}

pub fn metrics(metrics: &Metrics) -> Value {
    json!({
        "allocation_bytes": metrics.allocation_bytes,
        "peak_allocation_bytes": metrics.peak_allocation_bytes,
        "release_allocation_bytes": metrics.release_allocation_bytes,
        "peak_release_allocation_bytes": metrics.peak_release_allocation_bytes,
        "request_bytes": metrics.request_bytes,
        "peak_request_bytes": metrics.peak_request_bytes,
        "event_bytes": metrics.event_bytes,
        "peak_event_bytes": metrics.peak_event_bytes,
        "input_bytes_accepted": metrics.input_bytes_accepted,
        "display_bytes_delivered": metrics.display_bytes_delivered,
        "records_drained": metrics.records_drained,
        "deliveries_released": metrics.deliveries_released,
        "request_records": metrics.request_records,
        "peak_request_records": metrics.peak_request_records,
        "event_records": metrics.event_records,
        "peak_event_records": metrics.peak_event_records,
        "deliveries": metrics.deliveries,
        "peak_deliveries": metrics.peak_deliveries,
        "operations": metrics.operations,
        "peak_operations": metrics.peak_operations,
        "workers": metrics.workers,
        "peak_workers": metrics.peak_workers,
        "descriptors": metrics.descriptors,
        "peak_descriptors": metrics.peak_descriptors,
    })
}

pub fn limits(limits: &native::Limits) -> Value {
    json!({
        "client_bytes": limits.client_bytes,
        "request_bytes": limits.request_bytes,
        "event_bytes": limits.event_bytes,
        "progress_bytes": limits.progress_bytes,
        "drain_payload_bytes": limits.drain_payload_bytes,
        "request_records": limits.request_records,
        "event_records": limits.event_records,
        "progress_records": limits.progress_records,
        "deliveries": limits.deliveries,
        "operations": limits.operations,
        "chunk_bytes": limits.chunk_bytes,
        "drain_records": limits.drain_records,
        "wire_frame_bytes": limits.wire_frame_bytes,
        "metadata_record_bytes": limits.metadata_record_bytes,
        "title_bytes": limits.title_bytes,
        "notification_title_bytes": limits.notification_title_bytes,
        "notification_body_bytes": limits.notification_body_bytes,
        "max_workers": limits.max_workers,
        "operation_ms": limits.operation_ms,
        "kill_ms": limits.kill_ms,
        "restoration_stall_ms": limits.restoration_stall_ms,
        "restoration_total_ms": limits.restoration_total_ms,
        "delivery_stall_ms": limits.delivery_stall_ms,
        "leave_ms": limits.leave_ms,
        "cleanup_ms": limits.cleanup_ms,
        "start_ms": limits.start_ms,
        "reserved": limits.reserved,
    })
}

/// The table of one loaded library, instrumented.
#[derive(Clone, Copy)]
pub struct Api {
    table: &'static Interface,
    pub ledger: &'static Ledger,
    pub timeline: &'static Timeline,
}

macro_rules! entry {
    ($api:expr, $name:ident) => {
        $api.table
            .$name
            .expect(concat!("the table has ", stringify!($name)))
    };
}

impl Api {
    pub fn new(table: &'static Interface) -> Self {
        Self {
            table,
            ledger: Box::leak(Box::default()),
            timeline: Box::leak(Box::default()),
        }
    }

    /// The raw table, for calls with arguments the wrappers never pass.
    pub fn table(&self) -> &'static Interface {
        self.table
    }

    fn finish(&self, name: &str, result: Call) -> Value {
        let value = Value::Object(call(result));
        self.timeline.step(name, &value["code"]);
        value
    }

    /// Creates on the calling thread, which becomes the instance's owner.
    pub fn create(&self, startup: *const Startup) -> Creation {
        let mut out = filled::<native::Created>();
        // SAFETY: the startup is null or valid; the output is writable.
        let result = unsafe { entry!(self, create)(startup, out.as_mut_ptr()) };
        let output = state(&out);
        let call = self.finish("create", result);
        if result.code != native::SUCCESS {
            return Creation {
                call,
                output,
                handles: None,
                effective_limits: Value::Null,
            };
        }
        // SAFETY: success initialized the output.
        let created = unsafe { out.assume_init() };
        self.ledger.own("instance", created.instance as usize);
        self.ledger
            .own("communication", created.communication as usize);
        self.ledger.own("control", created.control as usize);
        Creation {
            call,
            output,
            handles: Some(Handles {
                instance: Ptr(created.instance),
                communication: Ptr(created.communication),
                control: Ptr(created.control),
            }),
            effective_limits: limits(&created.effective_limits),
        }
    }

    pub fn run(&self, handles: Handles) -> (Value, Value) {
        let mut out = filled::<RunReport>();
        // SAFETY: the handle is live; the output is writable.
        let result = unsafe { entry!(self, run)(handles.instance.0, out.as_mut_ptr()) };
        let output = state(&out);
        let report = if output == "modified" {
            // SAFETY: the library wrote the report.
            let report = unsafe { out.assume_init() };
            json!({
                "output": output,
                "workers_cleanup": report.workers_cleanup,
                "reserved": report.reserved,
                "terminal": terminal(&report.terminal),
            })
        } else {
            json!({ "output": output })
        };
        (self.finish("run", result), report)
    }

    /// Submits a request built in caller memory, which is scrambled and
    /// unmapped as soon as `submit` returns.
    pub fn submit(&self, handles: Handles, request: *const native::Request) -> Value {
        // SAFETY: the handle is live; the request is null or valid.
        let result = unsafe { entry!(self, submit)(handles.communication.0, request) };
        self.finish("submit", result)
    }

    pub fn shutdown(&self, handles: Handles) -> Value {
        // SAFETY: the handle is live.
        let result = unsafe { entry!(self, shutdown)(handles.control.0) };
        self.finish("shutdown", result)
    }

    pub fn drain(&self, handles: Handles) -> (Value, Drain) {
        let mut out = filled::<Drained>();
        // SAFETY: the handle is live; the output is writable.
        let result = unsafe { entry!(self, drain)(handles.communication.0, out.as_mut_ptr()) };
        let output = state(&out);
        let call = self.finish("drain", result);
        if result.code != native::SUCCESS {
            let observed = json!({ "output": output });
            return (
                call,
                Drain {
                    kind: 0,
                    delivery: None,
                    observed,
                    // SAFETY: an all-zero drain output holds only null pointers.
                    drained: Retained(unsafe { std::mem::zeroed() }),
                },
            );
        }
        // SAFETY: success initialized the output.
        let drained = unsafe { out.assume_init() };
        let delivery = (!drained.delivery.is_null()).then_some(Ptr(drained.delivery));
        if let Some(delivery) = delivery {
            self.ledger.own("delivery", delivery.0 as usize);
        }
        let observed = observe(&drained);
        (
            call,
            Drain {
                kind: drained.kind,
                delivery,
                observed,
                drained: Retained(drained),
            },
        )
    }

    pub fn release_delivery(&self, delivery: Ptr<Delivery>) -> Value {
        let address = delivery.0 as usize;
        assert!(
            self.ledger.owned("delivery", address),
            "host bug: not owned"
        );
        // SAFETY: an owned, unreleased delivery.
        let result = unsafe { entry!(self, release_delivery)(delivery.0) };
        if result.code == native::SUCCESS {
            self.ledger.release("delivery", address, "release_delivery");
        }
        self.finish("release_delivery", result)
    }

    pub fn release_communication(&self, handles: Handles) -> Value {
        let address = handles.communication.0 as usize;
        assert!(
            self.ledger.owned("communication", address),
            "host bug: not owned"
        );
        // SAFETY: an owned, unreleased handle.
        let result = unsafe { entry!(self, release_communication)(handles.communication.0) };
        if result.code == native::SUCCESS {
            self.ledger
                .release("communication", address, "release_communication");
        }
        self.finish("release_communication", result)
    }

    pub fn release_control(&self, handles: Handles) -> Value {
        let address = handles.control.0 as usize;
        assert!(self.ledger.owned("control", address), "host bug: not owned");
        // SAFETY: an owned, unreleased handle.
        let result = unsafe { entry!(self, release_control)(handles.control.0) };
        if result.code == native::SUCCESS {
            self.ledger.release("control", address, "release_control");
        }
        self.finish("release_control", result)
    }

    pub fn destroy(&self, handles: Handles) -> Value {
        let address = handles.instance.0 as usize;
        assert!(
            self.ledger.owned("instance", address),
            "host bug: not owned"
        );
        // SAFETY: an owned, undestroyed instance.
        let result = unsafe { entry!(self, destroy)(handles.instance.0) };
        if result.code == native::SUCCESS {
            self.ledger.release("instance", address, "destroy");
        }
        self.finish("destroy", result)
    }

    pub fn notification(&self, handles: Handles) -> (Value, i32) {
        let mut descriptor = i32::from_ne_bytes([FILL; 4]);
        // SAFETY: the handle is live; the output is writable.
        let result =
            unsafe { entry!(self, notification)(handles.communication.0, &mut descriptor) };
        (self.finish("notification", result), descriptor)
    }

    pub fn snapshot(&self, handles: Handles) -> (Value, Value) {
        let mut out = filled::<Metrics>();
        // SAFETY: the handle is live; the output is writable.
        let result = unsafe { entry!(self, snapshot)(handles.communication.0, out.as_mut_ptr()) };
        let output = state(&out);
        let call = self.finish("snapshot", result);
        if result.code != native::SUCCESS {
            return (call, json!({ "output": output }));
        }
        // SAFETY: success initialized the output.
        (call, metrics(&unsafe { out.assume_init() }))
    }
}

/// What `create` returned: its call, its output's state, and on success the
/// handles and effective limits.
pub struct Creation {
    pub call: Value,
    pub output: &'static str,
    pub handles: Option<Handles>,
    pub effective_limits: Value,
}

fn observe(drained: &Drained) -> Value {
    let mut value = json!({
        "kind": drained.kind,
        "reserved": drained.reserved,
        "delivery_null": drained.delivery.is_null(),
        "records_null": drained.records.is_null(),
        "count": drained.count,
        "terminal_null": drained.terminal.is_null(),
    });
    if !drained.records.is_null() && drained.count <= 64 {
        // SAFETY: a delivery's records cover `count` entries until release.
        let records =
            unsafe { std::slice::from_raw_parts(drained.records, drained.count as usize) };
        value["records"] = Value::Array(records.iter().map(record).collect());
    }
    if !drained.terminal.is_null() {
        // SAFETY: terminal evidence lives until destroy.
        value["terminal"] = terminal(unsafe { &*drained.terminal });
    }
    value
}
