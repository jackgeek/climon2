//! Explicit failures of the function-table calls (native revision 1's errors):
//! each kind's result code and stable error code live here, once.

use climon2_core::native::{self, Bytes, Call, Error};
use std::mem;

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
    pub(super) const fn error(self) -> Error {
        Error {
            origin: native::ORIGIN_LIBRARY,
            reserved: 0,
            code: text(self.kind.name),
            message: text(self.message),
        }
    }

    pub(super) const fn call(self) -> Call {
        Call {
            code: self.kind.code,
            reserved: 0,
            error: self.error(),
        }
    }
}

const fn text(value: &'static str) -> Bytes {
    Bytes {
        data: value.as_ptr(),
        length: value.len() as u64,
    }
}

/// A call's result: success, or its explicit failure.
pub(super) fn call(result: Result<(), Failure>) -> Call {
    result.map_or_else(Failure::call, |()| SUCCESS)
}

/// Succeeds when `holds`; otherwise fails with `failure`.
pub(super) fn require(holds: bool, failure: Failure) -> Result<(), Failure> {
    holds.then_some(()).ok_or(failure)
}
