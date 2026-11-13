//! Explicit failures (native revision 1's errors): each call failure's result
//! code and stable error code live here, once, and so does the shape of every
//! error a record or terminal evidence carries.

use climon2_core::{
    native::{self, Bytes, Call, Error},
    wire,
};
use std::{borrow::Cow, mem};

/// The result of a successful call: code `CM1_SUCCESS`, no origin and empty
/// error buffers, which is exactly the all-zero `cm1_call`.
// SAFETY: every field of `Call` is an integer or a raw pointer, for which zero
// (null) is a valid value.
pub(super) const SUCCESS: Call = unsafe { mem::zeroed() };

/// A `CM1_*` result code and the library's stable error code for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Kind {
    code: u32,
    name: &'static str,
}

pub(super) const INVALID_ARGUMENT: Kind = Kind {
    code: native::INVALID_ARGUMENT,
    name: "invalid_argument",
};
pub(super) const UNSUPPORTED_OPERATION: Kind = Kind {
    code: native::UNSUPPORTED_OPERATION,
    name: "unsupported_operation",
};
/// A capability family this release does not relay: still
/// `CM1_UNSUPPORTED_OPERATION`, with the more specific code.
pub(super) const UNSUPPORTED_CAPABILITY: Kind = Kind {
    code: native::UNSUPPORTED_OPERATION,
    name: "unsupported_capability",
};
pub(super) const INVALID_REQUEST_ID: Kind = Kind {
    code: native::INVALID_REQUEST_ID,
    name: "invalid_request_id",
};
pub(super) const FULL: Kind = Kind {
    code: native::FULL,
    name: "queue_full",
};
pub(super) const CLOSED: Kind = Kind {
    code: native::CLOSED,
    name: "closed",
};
pub(super) const BUSY: Kind = Kind {
    code: native::BUSY,
    name: "busy",
};
pub(super) const WRONG_THREAD: Kind = Kind {
    code: native::WRONG_THREAD,
    name: "wrong_thread",
};
pub(super) const UNSUPPORTED_LIMIT: Kind = Kind {
    code: native::UNSUPPORTED_LIMIT,
    name: "unsupported_limit",
};
pub(super) const RESOURCE_LIMIT: Kind = Kind {
    code: native::RESOURCE_LIMIT,
    name: "resource_limit",
};
pub(super) const INTERNAL_ERROR: Kind = Kind {
    code: native::INTERNAL_ERROR,
    name: "internal_error",
};
pub(super) const TIMEOUT: Kind = Kind {
    code: native::TIMEOUT,
    name: "timeout",
};
pub(super) const RELEASE_MISMATCH: Kind = Kind {
    code: native::RELEASE_MISMATCH,
    name: "release_mismatch",
};

/// One explicit failure: its kind and a static, content-free message, so its
/// text outlives every borrower.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Failure {
    kind: Kind,
    message: &'static str,
}

/// A contained panic: the call fails and its instance stops ordinary work.
pub(super) const PANIC: Failure = Failure::new(
    INTERNAL_ERROR,
    "the client library contained a panic; the instance stopped",
);

impl Failure {
    pub(super) const fn new(kind: Kind, message: &'static str) -> Self {
        Self { kind, message }
    }

    /// The library-origin error this failure reports, in calls, diagnostics
    /// and terminal evidence alike.
    pub(super) fn error(self) -> Error {
        Fault::from(self).error()
    }

    pub(super) fn call(self) -> Call {
        Call {
            code: self.kind.code,
            reserved: 0,
            error: self.error(),
        }
    }
}

/// Refuses with `refusal` unless `holds`: every check that can refuse
/// reads as one condition and the refusal it gives.
pub(super) fn require<E>(holds: bool, refusal: E) -> Result<(), E> {
    if holds { Ok(()) } else { Err(refusal) }
}

/// A call's result: success, or its explicit failure.
pub(super) fn call(result: Result<(), Failure>) -> Call {
    result.map_or_else(Failure::call, |()| SUCCESS)
}

/// A failure that ends an instance's stream, and the retry it calls for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Failed {
    pub(super) fault: Fault,
    pub(super) retry: u32,
}

impl Failed {
    pub(super) const fn new(fault: Fault, retry: u32) -> Self {
        Self { fault, retry }
    }

    /// Recoverable: transport loss, startup failure or a contained panic.
    pub(super) const fn passive(fault: Fault) -> Self {
        Self::new(fault, native::RETRY_PASSIVE)
    }

    /// For good: missing support or invalid configuration.
    pub(super) const fn permanent(fault: Fault) -> Self {
        Self::new(fault, native::RETRY_PERMANENT)
    }
}

/// The text of an error: the library's own static text, or a copy of the
/// daemon's.
type Text = Cow<'static, str>;

/// An error that a record or terminal evidence carries: who reported it, a
/// stable code and a diagnostic message. The library's and the transport's
/// are static text; the daemon's are copies its records own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Fault {
    origin: u32,
    code: Text,
    message: Text,
}

impl Fault {
    /// The library's own judgment: a deadline, a cancellation, a refusal.
    pub(super) const fn library(code: &'static str, message: &'static str) -> Self {
        Self::of(native::ORIGIN_LIBRARY, code, message)
    }

    /// A failure of the daemon connection's bytes: closure, I/O or framing,
    /// or traffic the wire forbids.
    pub(super) const fn transport(code: &'static str, message: &'static str) -> Self {
        Self::of(native::ORIGIN_TRANSPORT, code, message)
    }

    const fn of(origin: u32, code: &'static str, message: &'static str) -> Self {
        Self {
            origin,
            code: Cow::Borrowed(code),
            message: Cow::Borrowed(message),
        }
    }

    /// An error the daemon reported, copied.
    pub(super) fn daemon(error: &wire::Error) -> Self {
        Self {
            origin: native::ORIGIN_DAEMON,
            code: Cow::Owned(error.code.as_str().to_owned()),
            message: Cow::Owned(error.message.clone()),
        }
    }

    pub(super) fn code(&self) -> &str {
        &self.code
    }

    /// The bytes of copied text this fault owns; static text costs nothing.
    pub(super) fn bytes(&self) -> u64 {
        copied(&self.code) + copied(&self.message)
    }

    /// The native error, pointing into this fault's text.
    pub(super) fn error(&self) -> Error {
        Error {
            origin: self.origin,
            reserved: 0,
            code: bytes(&self.code),
            message: bytes(&self.message),
        }
    }
}

impl From<Failure> for Fault {
    fn from(failure: Failure) -> Self {
        Self::library(failure.kind.name, failure.message)
    }
}

fn copied(text: &Text) -> u64 {
    match text {
        Cow::Borrowed(_) => 0,
        Cow::Owned(text) => text.len() as u64,
    }
}

/// A counted view of text that outlives the view; empty text is null and
/// zero, as every empty native buffer is.
pub(super) const fn bytes(text: &str) -> Bytes {
    if text.is_empty() {
        return Bytes {
            data: std::ptr::null(),
            length: 0,
        };
    }
    Bytes {
        data: text.as_ptr(),
        length: text.len() as u64,
    }
}
