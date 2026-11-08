//! The client library's frozen C ABI bootstrap (native revision 1).

use climon2_core::native::{self, Description};

/// # Safety
///
/// `description` must be null or point to writable storage for one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn climon2_client_get_interface(
    _requested_revision: u32,
    _description: *mut *const Description,
) -> u32 {
    climon2_core::test_hook_marker!();
    native::UNSUPPORTED_REVISION
}
