//! `cm1_request`, validated and copied once, at `submit`: what the instance
//! queues is its own copy, never a caller buffer. Only the active argument
//! member is read; inactive storage is ignored.

use super::{
    failure::{
        FULL, Failure, INVALID_ARGUMENT, INVALID_REQUEST_ID, UNSUPPORTED_CAPABILITY,
        UNSUPPORTED_OPERATION, require,
    },
    families,
    startup::buffer,
};
use climon2_core::native::{self, Bytes, Tokens};
use std::{collections::BTreeSet, mem, slice};

/// What an accepted request costs the request queue: the request and every
/// buffer it names, as submitted.
const REQUEST: u64 = mem::size_of::<native::Request>() as u64;
const TOKEN: u64 = mem::size_of::<Bytes>() as u64;

/// One validated request.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct Submitted {
    pub(super) id: u64,
    pub(super) operation: Operation,
    /// What it charges the request queue's bytes.
    pub(super) bytes: u64,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum Operation {
    /// A monitor-mode Open.
    Open(Open),
    /// Leave, naming the connection generation it leaves.
    Leave(u64),
}

/// A monitor-mode Open's families: every required one relayed, and those
/// optional ones this release relays (it never selects others).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Open {
    pub(super) required: Vec<&'static str>,
    pub(super) optional: Vec<&'static str>,
}

fn invalid(message: &'static str) -> Failure {
    Failure::new(INVALID_ARGUMENT, message)
}

const TOO_LARGE: Failure = Failure::new(FULL, "the request does not fit request_bytes");

/// Validates and copies a request. `capacity` is the request queue's byte
/// limit: no token is read or copied for a request that cannot fit it.
///
/// # Safety
///
/// Every nonnull buffer and list in the active member covers its length or
/// count (the caller's contract).
pub(super) unsafe fn parse(request: &native::Request, capacity: u64) -> Result<Submitted, Failure> {
    let id = header(request)?;
    // SAFETY: guaranteed by the caller.
    let (operation, bytes) = unsafe { operation(request, capacity) }?;
    Ok(Submitted {
        id,
        operation,
        bytes,
    })
}

/// A known tag, reserved zero, then a positive ID.
fn header(request: &native::Request) -> Result<u64, Failure> {
    let known = (native::OPEN..=native::FOCUS).contains(&request.tag);
    require(known, invalid("unknown request tag"))?;
    require(request.reserved == 0, invalid("reserved must be zero"))?;
    (request.request_id != 0)
        .then_some(request.request_id)
        .ok_or(Failure::new(
            INVALID_REQUEST_ID,
            "request_id must be positive",
        ))
}

/// # Safety
///
/// As for [`parse`].
unsafe fn operation(request: &native::Request, capacity: u64) -> Result<(Operation, u64), Failure> {
    // SAFETY (both arms): the tag names the active member, whose buffers the
    // caller guarantees.
    match request.tag {
        native::OPEN => unsafe { open(&request.args.open, capacity) },
        native::LEAVE => leave(unsafe { request.args.generation }),
        _ => Err(Failure::new(
            UNSUPPORTED_OPERATION,
            "this client library does not perform this request yet",
        )),
    }
}

fn leave(generation: u64) -> Result<(Operation, u64), Failure> {
    (generation != 0)
        .then_some((Operation::Leave(generation), REQUEST))
        .ok_or(invalid("generation must be nonzero"))
}

/// # Safety
///
/// As for [`parse`].
unsafe fn open(open: &native::Open, capacity: u64) -> Result<(Operation, u64), Failure> {
    monitor(open)?;
    // SAFETY: guaranteed by the caller.
    let lists = unsafe { Lists::read(open, capacity) }?;
    // SAFETY: as above; `read` bounded the lists.
    unsafe { lists.open() }
}

/// Monitor mode only: interactive Open arrives with #94.
fn monitor(open: &native::Open) -> Result<(), Failure> {
    match open.mode {
        native::MONITOR => passive(open),
        native::INTERACTIVE => Err(Failure::new(
            UNSUPPORTED_OPERATION,
            "this client library does not open interactive connections yet",
        )),
        _ => Err(invalid("mode must be monitor or interactive")),
    }
}

fn passive(open: &native::Open) -> Result<(), Failure> {
    let zero = [open.reserved, open.geometry.cols, open.geometry.rows] == [0; 3];
    require(
        zero,
        invalid("monitor Open needs zero geometry and reserved zero"),
    )
}

/// Validated tokens, borrowed from the caller for the call.
type Texts<'a> = Vec<&'a [u8]>;

/// An Open's two token lists, sized before any token is read.
struct Lists<'a> {
    required: &'a [Bytes],
    optional: &'a [Bytes],
    bytes: u64,
}

impl<'a> Lists<'a> {
    /// # Safety
    ///
    /// As for [`parse`].
    unsafe fn read(open: &native::Open, capacity: u64) -> Result<Self, Failure> {
        // SAFETY (both): guaranteed by the caller.
        let required = unsafe { entries(open.required) }?;
        let optional = unsafe { entries(open.optional) }?;
        let bytes = [required, optional]
            .into_iter()
            .flatten()
            .fold(REQUEST, |total, token| {
                total.saturating_add(TOKEN).saturating_add(token.length)
            });
        (bytes <= capacity)
            .then_some(Self {
                required,
                optional,
                bytes,
            })
            .ok_or(TOO_LARGE)
    }

    /// # Safety
    ///
    /// As for [`parse`].
    unsafe fn open(&self) -> Result<(Operation, u64), Failure> {
        // SAFETY: guaranteed by the caller.
        let (required, optional) = unsafe { self.tokens() }?;
        Ok((Operation::Open(relayed(&required, &optional)?), self.bytes))
    }

    /// Every token nonempty UTF-8, each one once across both lists.
    ///
    /// # Safety
    ///
    /// As for [`parse`].
    unsafe fn tokens(&self) -> Result<(Texts<'a>, Texts<'a>), Failure> {
        // SAFETY (both): guaranteed by the caller.
        let required = unsafe { texts(self.required) }?;
        let optional = unsafe { texts(self.optional) }?;
        let distinct: BTreeSet<_> = required.iter().chain(&optional).collect();
        (distinct.len() == required.len() + optional.len())
            .then_some((required, optional))
            .ok_or(invalid(
                "tokens must be unique, and required and optional disjoint",
            ))
    }
}

/// A list's entries: null and zero, or a nonnull pointer and a count.
/// Nothing but the entries' lengths is read before the request's size is
/// known to fit.
///
/// # Safety
///
/// As for [`parse`].
unsafe fn entries<'a>(list: Tokens) -> Result<&'a [Bytes], Failure> {
    match (list.data.is_null(), list.count) {
        (true, 0) => Ok(&[]),
        // SAFETY: guaranteed by the caller.
        (false, 1..) => unsafe { listed(list) },
        _ => Err(invalid(
            "a token list is null and zero, or a pointer and a count",
        )),
    }
}

/// # Safety
///
/// As for [`parse`].
unsafe fn listed<'a>(list: Tokens) -> Result<&'a [Bytes], Failure> {
    let size = list
        .count
        .checked_mul(TOKEN)
        .and_then(|bytes| isize::try_from(bytes).ok());
    // SAFETY: a nonnull list covers its count (the caller's contract), and
    // the entries' size fits `isize`.
    size.map(|_| unsafe { slice::from_raw_parts(list.data, list.count as usize) })
        .ok_or(invalid("a token list's count overflows"))
}

/// # Safety
///
/// As for [`parse`].
unsafe fn texts<'a>(list: &[Bytes]) -> Result<Texts<'a>, Failure> {
    list.iter()
        // SAFETY: guaranteed by the caller.
        .map(|token| unsafe { buffer(*token) })
        .map(|token| token.filter(|token| std::str::from_utf8(token).is_ok()))
        .collect::<Option<_>>()
        .ok_or(invalid("tokens must be nonempty UTF-8"))
}

/// The relayed families: a required family this release does not relay is
/// unsupported; an optional one is never selected.
fn relayed(required: &[&[u8]], optional: &[&[u8]]) -> Result<Open, Failure> {
    let required = required
        .iter()
        .map(|token| families::relayed(token))
        .collect::<Option<_>>()
        .ok_or(Failure::new(
            UNSUPPORTED_CAPABILITY,
            "this client library does not relay a required family",
        ))?;
    let optional = optional
        .iter()
        .filter_map(|token| families::relayed(token))
        .collect();
    Ok(Open { required, optional })
}
