use climon2_core::Timestamp;
use std::io;

#[cfg(not(target_os = "macos"))]
compile_error!("climon2 local terminal presentation currently supports macOS only");

#[cfg(target_os = "macos")]
pub fn local_datetime(timestamp: &Timestamp) -> io::Result<String> {
    let created = time::OffsetDateTime::parse(
        timestamp.as_str(),
        &time::format_description::well_known::Rfc3339,
    )
    .map_err(io::Error::other)?;
    let seconds: libc::time_t = created.unix_timestamp();
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    // localtime_r writes to this exclusively owned buffer.
    if unsafe { libc::localtime_r(&seconds, local.as_mut_ptr()) }.is_null() {
        return Err(io::Error::last_os_error());
    }
    let local = unsafe { local.assume_init() };
    Ok(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        local.tm_year + 1900,
        local.tm_mon + 1,
        local.tm_mday,
        local.tm_hour,
        local.tm_min,
        local.tm_sec
    ))
}

#[cfg(target_os = "macos")]
pub fn columns() -> usize {
    let mut size = std::mem::MaybeUninit::<libc::winsize>::uninit();
    // Only the stdout terminal determines table width; redirected output defaults to 80.
    if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, size.as_mut_ptr()) } == 0 {
        let columns = unsafe { size.assume_init() }.ws_col;
        if columns != 0 {
            return columns.into();
        }
    }
    80
}
