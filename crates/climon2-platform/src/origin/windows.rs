use climon2_core::metadata::SessionOrigin;
use std::{io, ptr};
use windows_sys::{
    Win32::{
        Foundation::ERROR_SUCCESS,
        System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW},
    },
    w,
};

/// What the system says about where this process runs.
pub type Here = SessionOrigin;

/// Room for `MachineGuid`: a 36-character GUID and its terminating NUL.
const MACHINE_GUID_UNITS: usize = 64;
/// The same room in bytes, as `RegGetValueW` measures it.
const MACHINE_GUID_BYTES: u32 = (MACHINE_GUID_UNITS * size_of::<u16>()) as u32;

/// The machine, by the `MachineGuid` Windows keeps under
/// `HKLM\SOFTWARE\Microsoft\Cryptography` across reboots. A climon2 home on a
/// network share can be read from another machine, whose PIDs say nothing
/// about this one's daemons. Windows has no PID namespaces.
pub fn here() -> io::Result<SessionOrigin> {
    let machine = machine_guid()?.parse().map_err(io::Error::other)?;
    Ok(SessionOrigin {
        machine_id: Some(machine),
        pid_namespace: None,
    })
}

fn machine_guid() -> io::Result<String> {
    let mut units = [0u16; MACHINE_GUID_UNITS];
    let mut bytes = MACHINE_GUID_BYTES;
    // SAFETY: both names are NUL-terminated literals, and `bytes` is the size
    // of the writable `units`.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Cryptography"),
            w!("MachineGuid"),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            units.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    // The value is NUL-terminated and the rest of `units` stays zero.
    let text = String::from_utf16_lossy(&units);
    Ok(text.trim_end_matches('\0').to_owned())
}

/// The machine ID the system keeps is stable, so the origin is what the system
/// says, wherever the climon2 home is.
pub fn origin(here: &Here, _home: &std::path::Path) -> SessionOrigin {
    here.clone()
}
