use climon2_core::metadata::SessionOrigin;
use std::io;

/// What the system says about where this process runs.
pub type Here = SessionOrigin;

/// How long `gethostuuid` may wait for the hardware UUID.
const HOST_UUID_WAIT: libc::timespec = libc::timespec {
    tv_sec: 5,
    tv_nsec: 0,
};

unsafe extern "C" {
    /// `<unistd.h>`: the host's hardware UUID.
    fn gethostuuid(id: *mut u8, wait: *const libc::timespec) -> libc::c_int;
}

/// The machine's hardware UUID, the `IOPlatformUUID` that `ioreg` shows, as 32
/// lowercase hex digits. macOS has no PID namespaces.
pub fn here() -> io::Result<SessionOrigin> {
    let mut uuid = [0u8; 16];
    // SAFETY: gethostuuid writes exactly 16 bytes and only reads the timeout.
    if unsafe { gethostuuid(uuid.as_mut_ptr(), &HOST_UUID_WAIT) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let hex: String = uuid.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(SessionOrigin {
        machine_id: Some(hex.parse().map_err(io::Error::other)?),
        pid_namespace: None,
    })
}

/// The machine ID the system keeps is stable, so the origin is what the system
/// says, wherever the climon2 home is.
pub fn origin(here: &Here, _home: &std::path::Path) -> io::Result<SessionOrigin> {
    Ok(here.clone())
}
