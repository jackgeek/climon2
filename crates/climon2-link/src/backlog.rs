//! The bound on stream-0 records an end has received and its caller has
//! not yet taken (#480, item 17). An honest relay stays far below it,
//! because it keeps at most one pending advertisement change for each
//! session; breaking it is a protocol violation.

/// The most stream-0 records the queue holds.
const RECORDS_MOST: usize = 1_024;
/// The most bytes, tags included, the queue holds.
const BYTES_MOST: usize = 1_048_576;

/// The stream-0 records received and not yet taken: how many, and their
/// bytes.
#[derive(Default)]
pub(crate) struct Backlog {
    records: usize,
    bytes: usize,
}

impl Backlog {
    /// Admits one more record, `length` bytes long, tag included, unless
    /// that would take the queue past either bound.
    pub(crate) fn admit(&mut self, length: usize) -> bool {
        let records = self.records + 1;
        let bytes = self.bytes + length;
        let room = records <= RECORDS_MOST && bytes <= BYTES_MOST;
        if room {
            *self = Self { records, bytes };
        }
        room
    }
}
