/// The connection-fatal catalogue. Each code ends the connection; delivering
/// it to the peer is best effort.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FatalCode {
    InvalidFrameLength,
    FrameTooLarge,
    TruncatedFrame,
    InvalidJson,
    InvalidRecord,
    InvalidRequestId,
    UnknownMessageType,
    WrongDirection,
    InternalError,
}
