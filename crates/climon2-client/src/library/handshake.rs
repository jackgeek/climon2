//! Before any role takes effect: the session metadata, the frozen hello and
//! the daemon's revision-1 support must all agree with this instance's
//! session, release and Open. Each refusal says whether retrying can help:
//! an unreachable or silent daemon can recover; another session, another
//! release or missing support cannot.

use super::{
    RELEASE,
    failure::{Failed, Fault, require},
    families,
    link::{Link, TIMEOUT, remaining},
    request::Open,
};
use climon2_core::{
    SessionId,
    hello::{Advertised, HelloExtensions},
    metadata::SessionMetadata,
    protocol::{self, ClientMessage, HelloReply, PROTOCOL_VERSION, ServerMessage},
    wire::{Advertisement, Frame, FrameLimit},
};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::Instant,
};

const UNREADABLE: Failed = Failed::passive(Fault::library(
    "io_error",
    "the session metadata cannot be read",
));
const INVALID_METADATA: Failed = Failed::permanent(Fault::library(
    "invalid_configuration",
    "the session metadata is not valid metadata of this session",
));
const ANOTHER_RELEASE: Failed = Failed::permanent(Fault::library(
    "release_mismatch",
    "the session runs another release",
));
const ANOTHER_SESSION: Failed = Failed::permanent(Fault::library(
    "invalid_configuration",
    "the session daemon is another session's",
));
const BAD_HELLO: Failed = Failed::passive(Fault::transport(
    "invalid_record",
    "the session daemon's hello reply is not valid",
));
const UNSUPPORTED_PROTOCOL: Failed = Failed::permanent(Fault::library(
    "unsupported_revision",
    "the session daemon speaks another hello protocol",
));
const NO_REVISION: Failed = Failed::permanent(Fault::library(
    "unsupported_revision",
    "the session daemon does not advertise daemon wire revision 1",
));
const UNSUPPORTED_FAMILY: Failed = Failed::permanent(Fault::library(
    "unsupported_capability",
    "the session daemon does not support a required family",
));
const UNFIT_FRAMES: Failed = Failed::permanent(Fault::library(
    "unsupported_limit",
    "the session daemon's frames can exceed wire_frame_bytes",
));

/// The frozen hello's limit on its frames.
const HELLO_FRAME: u32 = protocol::MAX_FRAME_LENGTH as u32;

/// What an instance checks its session against.
pub(super) struct Expected<'a> {
    pub(super) session: &'a SessionId,
    pub(super) metadata: &'a Path,
    /// At most what the instance may allocate: a larger file is not read.
    pub(super) bytes: u64,
    pub(super) wire_frame_bytes: u32,
}

/// What the daemon advertised: its families and the frame ceilings both
/// sides keep.
pub(super) struct Peer {
    pub(super) advertisement: Advertisement,
    pub(super) receive: FrameLimit,
    pub(super) send: FrameLimit,
}

/// The socket the session's metadata names, once the metadata is this
/// session's, at this release.
pub(super) fn socket(expected: &Expected) -> Result<PathBuf, Failed> {
    let mut bytes = Vec::new();
    File::open(expected.metadata)
        .and_then(|file| file.take(expected.bytes).read_to_end(&mut bytes))
        .map_err(|_| UNREADABLE)?;
    let metadata =
        SessionMetadata::from_json(expected.metadata, &bytes).map_err(|_| INVALID_METADATA)?;
    require(metadata.session_id == *expected.session, INVALID_METADATA)?;
    agree(metadata.climon2_version.as_str(), RELEASE, ANOTHER_RELEASE)?;
    Ok(PathBuf::from(metadata.socket_path.as_str()))
}

fn agree(found: &str, expected: &str, refusal: Failed) -> Result<(), Failed> {
    require(found == expected, refusal)
}

/// Connects and exchanges the frozen hello before `deadline`, then checks
/// the daemon's identity and its support for `open`.
pub(super) fn hello(
    socket: &Path,
    expected: &Expected,
    open: &Open,
    deadline: Instant,
) -> Result<(Link, Peer), Failed> {
    let timeout = remaining(deadline).ok_or(Failed::passive(TIMEOUT))?;
    let mut link = Link::connect(socket, timeout).map_err(Failed::passive)?;
    let reply = exchange(&mut link, deadline)?;
    let peer = peer(&reply, expected, open)?;
    Ok((link, peer))
}

fn exchange(link: &mut Link, deadline: Instant) -> Result<Frame, Failed> {
    let mut hello = Vec::new();
    let frame = ClientMessage::Hello {
        protocol: PROTOCOL_VERSION,
    }
    .to_frame()
    .expect("the hello request is a valid frame");
    protocol::write_frame(&mut hello, &frame).expect("writing to memory succeeds");
    link.send(&hello);
    let limit = FrameLimit::new(HELLO_FRAME).expect("the hello limit is positive");
    link.exchange(limit, deadline).map_err(Failed::passive)
}

fn peer(frame: &Frame, expected: &Expected, open: &Open) -> Result<Peer, Failed> {
    identity(&reply(frame)?, expected.session)?;
    let advertisement = advertisement(frame)?;
    supported(advertisement, open, expected.wire_frame_bytes)
}

/// The frozen reply: a hello frame with its required fields.
fn reply(frame: &Frame) -> Result<HelloReply, Failed> {
    let message = protocol::Frame::new(frame.code(), frame.body().to_vec())
        .and_then(ServerMessage::from_frame);
    match message {
        Ok(ServerMessage::Hello(reply)) => Ok(reply),
        _ => Err(BAD_HELLO),
    }
}

/// This session, the frozen protocol and this release.
fn identity(reply: &HelloReply, session: &SessionId) -> Result<(), Failed> {
    require(reply.session_id == *session, ANOTHER_SESSION)?;
    require(reply.protocol == PROTOCOL_VERSION, UNSUPPORTED_PROTOCOL)?;
    agree(reply.climon2_version.as_str(), RELEASE, ANOTHER_RELEASE)
}

/// Revision 1 is available only with a valid advertisement; the release
/// never implies it.
fn advertisement(frame: &Frame) -> Result<Advertisement, Failed> {
    match HelloExtensions::from_body(frame.body()).daemon_wire_v1 {
        Advertised::Valid(advertisement) => Ok(advertisement),
        _ => Err(NO_REVISION),
    }
}

/// The families the Open needs, and frames this instance can receive.
fn supported(
    advertisement: Advertisement,
    open: &Open,
    wire_frame_bytes: u32,
) -> Result<Peer, Failed> {
    require(
        families::supports(&advertisement, open.mode, &open.requested),
        UNSUPPORTED_FAMILY,
    )?;
    let limits = advertisement.limits();
    let server = limits.max_server_frame_bytes.get();
    require(server <= wire_frame_bytes, UNFIT_FRAMES)?;
    let send = limits.max_request_frame_bytes.get().min(wire_frame_bytes);
    Ok(Peer {
        receive: limits.max_server_frame_bytes,
        send: FrameLimit::new(send).expect("both ceilings are positive"),
        advertisement,
    })
}
