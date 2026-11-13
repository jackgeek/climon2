//! What the fixture test binaries share.

use std::time::Duration;

/// Bounds every wait for an eventual event: tens of seconds, for a host under
/// mutation-worker load, and shorter than the fixtures' own deadlines.
pub const TIMEOUT: Duration = Duration::from_secs(20);

const _: () = assert!(TIMEOUT.as_secs() < climon2_fixtures::BOUND.as_secs());
