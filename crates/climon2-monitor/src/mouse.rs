//! Host mouse reports (#113): the monitor asks the host for SGR-encoded
//! reports, `ESC [ < Cb ; Cx ; Cy` ending in `M` for a press, a wheel notch
//! or motion, and `m` for a release, with 1-based coordinates.

/// One SGR mouse report, as the host sent it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Mouse {
    /// `Cb`: the button, with the wheel and motion bits and modifiers.
    pub(crate) code: u16,
    /// The host cell, 0-based.
    pub(crate) column: u16,
    pub(crate) row: u16,
    /// `m`: a button went up.
    pub(crate) release: bool,
}

/// What a mouse report means to the selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Gesture {
    /// The left button went down.
    Click,
    WheelUp,
    WheelDown,
    /// Anything else: a release, motion or another button.
    Other,
}

/// Shift, Meta and Control.
const MODIFIERS: u16 = 4 | 8 | 16;

impl Mouse {
    /// The report whose bytes after `ESC [` are `body`, if it is one.
    pub(crate) fn parse(body: &[u8]) -> Option<Self> {
        let (last, fields) = body.strip_prefix(b"<")?.split_last()?;
        let release = [b'M', b'm'].iter().position(|end| end == last)? == 1;
        let [code, column, row] = numbers(fields)?;
        Some(Self {
            code,
            column: column.saturating_sub(1),
            row: row.saturating_sub(1),
            release,
        })
    }

    /// The gesture, whatever modifiers were held.
    pub(crate) fn gesture(&self) -> Gesture {
        match (self.code & !MODIFIERS, self.release) {
            (0, false) => Gesture::Click,
            (64, false) => Gesture::WheelUp,
            (65, false) => Gesture::WheelDown,
            _ => Gesture::Other,
        }
    }
}

/// Exactly three decimal numbers separated by `;`.
fn numbers(fields: &[u8]) -> Option<[u16; 3]> {
    let parsed: Result<Vec<u16>, _> = std::str::from_utf8(fields)
        .ok()?
        .split(';')
        .map(str::parse)
        .collect();
    parsed.ok()?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mouse(code: u16, column: u16, row: u16, release: bool) -> Option<Mouse> {
        Some(Mouse {
            code,
            column,
            row,
            release,
        })
    }

    /// INP-05 (pure supplement): SGR reports parse to their code, 0-based
    /// cell and press or release; anything else is no report.
    #[test]
    fn sgr_reports_parse_to_their_code_cell_and_release() {
        assert_eq!(Mouse::parse(b"<0;5;7M"), mouse(0, 4, 6, false));
        assert_eq!(Mouse::parse(b"<2;1;1m"), mouse(2, 0, 0, true));
        assert_eq!(Mouse::parse(b"<65;300;200M"), mouse(65, 299, 199, false));
        for body in [
            &b"<0;5;7"[..],
            b"<0;5;7c",
            b"0;5;7M",
            b"<0;5M",
            b"<0;5;7;1M",
            b"<0;x;7M",
            b"<0;5;70000M",
            b"<\xff;5;7M",
            b"<",
            b"",
        ] {
            assert_eq!(Mouse::parse(body), None, "{body:?}");
        }
    }

    /// INP-05 (pure supplement): a left press is a click and wheel notches
    /// scroll, whatever modifiers are held; releases, motion and other
    /// buttons are neither.
    #[test]
    fn left_presses_click_and_wheel_notches_scroll_with_any_modifiers() {
        let gesture = |code, release| {
            Mouse::parse(&format!("<{code};1;1{}", if release { 'm' } else { 'M' }).into_bytes())
                .unwrap()
                .gesture()
        };
        for modifiers in [0, 4, 8, 16, 28] {
            assert_eq!(gesture(modifiers, false), Gesture::Click);
            assert_eq!(gesture(64 + modifiers, false), Gesture::WheelUp);
            assert_eq!(gesture(65 + modifiers, false), Gesture::WheelDown);
        }
        for (code, release) in [
            (0, true),
            (1, false),
            (2, false),
            (32, false),
            (35, false),
            (64, true),
            (66, false),
        ] {
            assert_eq!(gesture(code, release), Gesture::Other, "{code} {release}");
        }
    }
}
