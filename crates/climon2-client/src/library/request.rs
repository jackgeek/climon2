//! `cm1_request`, validated and copied once, at `submit`: what the instance
//! queues is its own copy, never a caller buffer. Only the active argument
//! member is read; inactive storage is ignored. Authority and geometry are
//! parsed into core's wire types here, once (#94).

use super::{
    failure::{
        FULL, Failure, INVALID_ARGUMENT, INVALID_REQUEST_ID, UNSUPPORTED_CAPABILITY,
        UNSUPPORTED_OPERATION, require,
    },
    families,
    startup::buffer,
};
use climon2_core::{
    native::{self, Bytes, Tokens},
    wire::{AuthorityIds, Counter, Families, Geometry, Mode, Requested},
};
use std::{mem, num::NonZeroU32, slice};

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
    Open(Open),
    /// Leave, naming the connection generation it leaves.
    Leave(u64),
    /// A request on the opened role (#94), naming its generation.
    Command(Command),
}

/// An Open's mode and families, parsed once: unique, required and optional
/// disjoint, every required one relayed, and only those optional ones this
/// release relays (it never selects others).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Open {
    pub(super) mode: Mode,
    pub(super) requested: Requested,
}

/// A request on the opened role: the generation it names, and what it asks.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct Command {
    pub(super) generation: u64,
    pub(super) act: Act,
}

/// What a command asks of the daemon, under the period and restoration it
/// names, if any. Input owns its exact bytes.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Act {
    Acquire,
    Input(AuthorityIds, Box<[u8]>),
    Resize(AuthorityIds, Geometry),
    Ready(AuthorityIds),
    Focus(AuthorityIds, bool),
}

fn invalid(message: &'static str) -> Failure {
    Failure::new(INVALID_ARGUMENT, message)
}

const TOO_LARGE: Failure = Failure::new(FULL, "the request does not fit request_bytes");

/// What a request may cost and carry: the request queue's byte limit, and
/// the most bytes one Input carries.
#[derive(Clone, Copy)]
pub(super) struct Capacity {
    pub(super) request_bytes: u64,
    pub(super) chunk_bytes: u64,
}

/// Validates and copies a request. No token or byte is read or copied for
/// a request that cannot fit `capacity`.
///
/// # Safety
///
/// Every nonnull buffer and list in the active member covers its length or
/// count (the caller's contract).
pub(super) unsafe fn parse(
    request: &native::Request,
    capacity: Capacity,
) -> Result<Submitted, Failure> {
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

/// One request tag's parser: its operation and what it charges.
///
/// # Safety
///
/// As for [`parse`], for the member the tag names.
type Parser = unsafe fn(&native::Request, Capacity) -> Result<(Operation, u64), Failure>;

/// Every request this release performs, by tag.
const PARSERS: [(u32, Parser); 7] = [
    (native::OPEN, open_request),
    (native::LEAVE, leave),
    (native::ACQUIRE, acquire),
    (native::INPUT, input),
    (native::RESIZE, resize),
    (native::READY, ready),
    (native::FOCUS, focus),
];

/// # Safety
///
/// As for [`parse`].
unsafe fn operation(
    request: &native::Request,
    capacity: Capacity,
) -> Result<(Operation, u64), Failure> {
    let parser = PARSERS
        .iter()
        .find(|(tag, _)| *tag == request.tag)
        .map(|(_, parser)| *parser)
        .ok_or(Failure::new(
            UNSUPPORTED_OPERATION,
            "this client library does not perform this request yet",
        ))?;
    // SAFETY: the parser reads the member the tag names, whose buffers the
    // caller guarantees.
    unsafe { parser(request, capacity) }
}

fn nonzero(generation: u64) -> Result<u64, Failure> {
    (generation != 0)
        .then_some(generation)
        .ok_or(invalid("generation must be nonzero"))
}

/// A command on the role, charged `bytes`.
fn command(generation: u64, act: Act, bytes: u64) -> (Operation, u64) {
    (Operation::Command(Command { generation, act }), bytes)
}

/// # Safety
///
/// The tag names `generation`.
unsafe fn leave(request: &native::Request, _: Capacity) -> Result<(Operation, u64), Failure> {
    // SAFETY: guaranteed by the caller.
    let generation = nonzero(unsafe { request.args.generation })?;
    Ok((Operation::Leave(generation), REQUEST))
}

/// # Safety
///
/// The tag names `generation`.
unsafe fn acquire(request: &native::Request, _: Capacity) -> Result<(Operation, u64), Failure> {
    // SAFETY: guaranteed by the caller.
    let generation = nonzero(unsafe { request.args.generation })?;
    Ok(command(generation, Act::Acquire, REQUEST))
}

/// A nonzero generation, period and restoration ID, never substituted for
/// one another.
fn authority(authority: native::Authority) -> Result<(u64, AuthorityIds), Failure> {
    let ids = Counter::new(authority.period)
        .zip(Counter::new(authority.restoration_id))
        .map(|(period, restoration_id)| AuthorityIds {
            period,
            restoration_id,
        });
    ids.filter(|_| authority.generation != 0)
        .map(|ids| (authority.generation, ids))
        .ok_or(invalid("authority must be nonzero"))
}

/// # Safety
///
/// The tag names `authority`.
unsafe fn ready(request: &native::Request, _: Capacity) -> Result<(Operation, u64), Failure> {
    // SAFETY: guaranteed by the caller.
    let (generation, ids) = authority(unsafe { request.args.authority })?;
    Ok(command(generation, Act::Ready(ids), REQUEST))
}

/// Positive columns and rows.
fn geometry(geometry: native::Geometry) -> Option<Geometry> {
    NonZeroU32::new(geometry.cols)
        .zip(NonZeroU32::new(geometry.rows))
        .map(|(cols, rows)| Geometry { cols, rows })
}

/// # Safety
///
/// The tag names `resize`.
unsafe fn resize(request: &native::Request, _: Capacity) -> Result<(Operation, u64), Failure> {
    // SAFETY: guaranteed by the caller.
    let resize = unsafe { request.args.resize };
    let (generation, ids) = authority(resize.authority)?;
    let geometry = geometry(resize.geometry).ok_or(invalid("geometry must be positive"))?;
    Ok(command(generation, Act::Resize(ids, geometry), REQUEST))
}

/// Only 0 and 1, with every reserved byte zero.
///
/// # Safety
///
/// The tag names `flag`.
unsafe fn focus(request: &native::Request, _: Capacity) -> Result<(Operation, u64), Failure> {
    // SAFETY: guaranteed by the caller.
    let flag = unsafe { request.args.flag };
    let (generation, ids) = authority(flag.authority)?;
    require(
        flag.value <= 1 && flag.reserved == [0; 7],
        invalid("focus is 0 or 1, with reserved bytes zero"),
    )?;
    Ok(command(
        generation,
        Act::Focus(ids, flag.value == 1),
        REQUEST,
    ))
}

/// # Safety
///
/// The tag names `input`, whose buffer covers its length.
unsafe fn input(
    request: &native::Request,
    capacity: Capacity,
) -> Result<(Operation, u64), Failure> {
    // SAFETY: guaranteed by the caller.
    let input = unsafe { request.args.input };
    let (generation, ids) = authority(input.authority)?;
    // SAFETY: as above.
    let (payload, bytes) = unsafe { payload(input.bytes, capacity) }?;
    Ok(command(generation, Act::Input(ids, payload), bytes))
}

/// Nonempty exact bytes of at most `chunk_bytes`, which with the request
/// must fit `request_bytes`: sized before any byte is copied.
///
/// # Safety
///
/// A nonnull buffer covers its length.
unsafe fn payload(bytes: Bytes, capacity: Capacity) -> Result<(Box<[u8]>, u64), Failure> {
    require(
        bytes.length <= capacity.chunk_bytes,
        invalid("input exceeds chunk_bytes"),
    )?;
    let charge = REQUEST + bytes.length;
    require(charge <= capacity.request_bytes, TOO_LARGE)?;
    // SAFETY: guaranteed by the caller.
    unsafe { buffer(bytes) }
        .map(|payload| (Box::from(payload), charge))
        .ok_or(invalid("input must be nonempty"))
}

/// # Safety
///
/// The tag names `open`, whose lists cover their counts.
unsafe fn open_request(
    request: &native::Request,
    capacity: Capacity,
) -> Result<(Operation, u64), Failure> {
    // SAFETY: guaranteed by the caller.
    unsafe { open(&request.args.open, capacity.request_bytes) }
}

/// # Safety
///
/// As for [`parse`].
unsafe fn open(open: &native::Open, capacity: u64) -> Result<(Operation, u64), Failure> {
    let mode = mode(open)?;
    // SAFETY: guaranteed by the caller.
    let lists = unsafe { Lists::read(open, capacity) }?;
    // SAFETY: as above; `read` bounded the lists.
    unsafe { lists.open(mode) }
}

/// Monitor mode at zero geometry, or interactive at a positive one;
/// reserved zero either way.
fn mode(open: &native::Open) -> Result<Mode, Failure> {
    require(open.reserved == 0, invalid("reserved must be zero"))?;
    match open.mode {
        native::MONITOR => monitor(open.geometry),
        native::INTERACTIVE => geometry(open.geometry)
            .map(Mode::Interactive)
            .ok_or(invalid("interactive Open needs a positive geometry")),
        _ => Err(invalid("mode must be monitor or interactive")),
    }
}

fn monitor(geometry: native::Geometry) -> Result<Mode, Failure> {
    let zero = [geometry.cols, geometry.rows] == [0; 2];
    zero.then_some(Mode::Monitor)
        .ok_or(invalid("monitor Open needs zero geometry"))
}

/// Validated tokens, copied from the caller.
type Texts = Vec<String>;

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
    unsafe fn open(&self, mode: Mode) -> Result<(Operation, u64), Failure> {
        // SAFETY: guaranteed by the caller.
        let requested = unsafe { self.tokens() }?;
        Ok((Operation::Open(relayed(mode, &requested)?), self.bytes))
    }

    /// Every token nonempty UTF-8, each one once across both lists.
    ///
    /// # Safety
    ///
    /// As for [`parse`].
    unsafe fn tokens(&self) -> Result<Requested, Failure> {
        // SAFETY (both): guaranteed by the caller.
        let required = unsafe { texts(self.required) }?;
        let optional = unsafe { texts(self.optional) }?;
        Families::new(required)
            .zip(Families::new(optional))
            .and_then(|(required, optional)| Requested::new(required, optional))
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
unsafe fn texts(list: &[Bytes]) -> Result<Texts, Failure> {
    list.iter()
        // SAFETY: guaranteed by the caller.
        .map(|token| unsafe { buffer(*token) })
        .map(|token| token.and_then(|token| std::str::from_utf8(token).ok()))
        .map(|token| token.map(str::to_owned))
        .collect::<Option<_>>()
        .ok_or(invalid("tokens must be nonempty UTF-8"))
}

/// The relayed families: a required family this release does not relay is
/// unsupported; an optional one is never requested.
fn relayed(mode: Mode, requested: &Requested) -> Result<Open, Failure> {
    let relayed = |token: &str| families::relayed(token).is_some();
    require(
        requested
            .required()
            .tokens()
            .iter()
            .all(|token| relayed(token)),
        Failure::new(
            UNSUPPORTED_CAPABILITY,
            "this client library does not relay a required family",
        ),
    )?;
    Ok(Open {
        mode,
        requested: requested.retain_optional(relayed),
    })
}
