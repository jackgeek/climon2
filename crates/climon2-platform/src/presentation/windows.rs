//! How `list` shows a time and how wide its table may be, on Windows (#316).
//! The time is the system's local time for that instant; the width is the
//! console window's, and redirected output, which has none, gets 80 columns
//! as on Unix.
use climon2_core::Timestamp;
use std::io;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339, macros};
use windows_sys::Win32::{
    Foundation::FALSE,
    System::Console::{
        CONSOLE_SCREEN_BUFFER_INFO, GetConsoleScreenBufferInfo, GetStdHandle, STD_OUTPUT_HANDLE,
    },
};

/// The width of output that is not a console.
const REDIRECTED_COLUMNS: usize = 80;

pub fn local_datetime(timestamp: &Timestamp) -> io::Result<String> {
    let created = OffsetDateTime::parse(timestamp.as_str(), &Rfc3339).map_err(io::Error::other)?;
    let offset = UtcOffset::local_offset_at(created).map_err(io::Error::other)?;
    created
        .to_offset(offset)
        .format(macros::format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second]"
        ))
        .map_err(io::Error::other)
}

pub fn columns() -> usize {
    let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
    // The standard handle needs no closing, and `info` is writable.
    if unsafe { GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info) } == FALSE {
        return REDIRECTED_COLUMNS;
    }
    window_width(&info)
}

/// The visible window, not the screen buffer behind it, which can be wider.
fn window_width(info: &CONSOLE_SCREEN_BUFFER_INFO) -> usize {
    usize::from((info.srWindow.Right - info.srWindow.Left + 1).unsigned_abs())
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::System::Console::{CONSOLE_SCREEN_BUFFER_INFO, COORD, SMALL_RECT};

    /// A console's own report cannot be arranged in a test, so the window
    /// it reports is: columns 10 to 139 inclusive of a 300-column buffer.
    #[test]
    fn the_width_is_the_visible_windows_inclusive_column_span() {
        let info = CONSOLE_SCREEN_BUFFER_INFO {
            dwSize: COORD { X: 300, Y: 9000 },
            srWindow: SMALL_RECT {
                Left: 10,
                Top: 0,
                Right: 139,
                Bottom: 29,
            },
            ..Default::default()
        };
        assert_eq!(super::window_width(&info), 130);
    }
}
