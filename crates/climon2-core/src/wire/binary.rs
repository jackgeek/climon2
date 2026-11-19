//! Binary records: Input `0x31` and Display `0x49`. A header of nonzero
//! big-endian u64 counters comes first; the payload after it is exact bytes,
//! never text: a NUL does not end it and its UTF-8 is never checked.
use super::{AuthorityIds, Counter, FatalCode, Frame, record::correlated};
use std::array;

/// Input `0x31`: user bytes for the application under the current control
/// period and restoration. An empty payload is still identifiable, so the
/// session daemon rejects it with an operational error instead of failing
/// the connection.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Input {
    pub request_id: Counter,
    pub authority: AuthorityIds,
    pub payload: Vec<u8>,
}

impl Input {
    pub const CODE: u8 = 0x31;

    pub fn to_frame(&self) -> Frame {
        let AuthorityIds {
            period,
            restoration_id,
        } = self.authority;
        frame(
            Self::CODE,
            [self.request_id, period, restoration_id],
            &self.payload,
        )
    }

    pub(crate) fn decode(body: &[u8]) -> Result<Self, FatalCode> {
        let ([request_id, period, restoration_id], payload) = split(Self::CODE, body)?;
        Ok(Self {
            request_id,
            authority: AuthorityIds {
                period,
                restoration_id,
            },
            payload: payload.to_vec(),
        })
    }
}

/// Display `0x49`: terminal bytes for the controller under its control
/// period and restoration. An empty payload is an invalid operation for the
/// client to judge, not a malformed record.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Display {
    pub authority: AuthorityIds,
    pub payload: Vec<u8>,
}

impl Display {
    pub const CODE: u8 = 0x49;

    pub fn to_frame(&self) -> Frame {
        let AuthorityIds {
            period,
            restoration_id,
        } = self.authority;
        frame(Self::CODE, [period, restoration_id], &self.payload)
    }

    pub(crate) fn decode(body: &[u8]) -> Result<Self, FatalCode> {
        let ([period, restoration_id], payload) = split(Self::CODE, body)?;
        Ok(Self {
            authority: AuthorityIds {
                period,
                restoration_id,
            },
            payload: payload.to_vec(),
        })
    }
}

const COUNTER_BYTES: usize = 8;

fn frame<const N: usize>(code: u8, header: [Counter; N], payload: &[u8]) -> Frame {
    let mut body: Vec<u8> = header
        .into_iter()
        .flat_map(|counter| counter.get().to_be_bytes())
        .collect();
    body.extend_from_slice(payload);
    Frame::new(code, body)
}

/// A body shorter than its header is `invalid_record`. A request's header
/// starts with its request ID, so the message type decides which zero is
/// `invalid_request_id`, as it does for JSON records.
fn split<const N: usize>(code: u8, body: &[u8]) -> Result<([Counter; N], &[u8]), FatalCode> {
    let (header, payload) = body
        .split_at_checked(N * COUNTER_BYTES)
        .ok_or(FatalCode::InvalidRecord)?;
    let (words, _) = header.as_chunks::<COUNTER_BYTES>();
    let counters: [Option<Counter>; N] =
        array::from_fn(|index| Counter::new(u64::from_be_bytes(words[index])));
    match counters.iter().position(Option::is_none) {
        Some(index) => Err(zero(code, index)),
        None => Ok((counters.map(Option::unwrap), payload)),
    }
}

fn zero(code: u8, index: usize) -> FatalCode {
    if index == 0 && correlated(code) {
        FatalCode::InvalidRequestId
    } else {
        FatalCode::InvalidRecord
    }
}
