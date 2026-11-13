//! Host input a real terminal sends its application, encoded as xterm
//! documents it: focus reports (`CSI I`, `CSI O`), SGR mouse reports
//! (`CSI < Cb ; Cx ; Cy M` or `m`) and bracketed paste (`CSI 200 ~`, `CSI 201 ~`).
//! A real terminal sends each only in the modes the application enabled.

use alacritty_terminal::term::TermMode;

/// A mouse button that presses, drags and releases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

impl MouseButton {
    /// The SGR button code `Cb`.
    fn code(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Middle => 1,
            Self::Right => 2,
        }
    }
}

/// A host mouse action, reported in SGR encoding (`CSI ? 1006 h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mouse {
    /// The button goes down: `Cb` is the button, ending in `M`.
    Press(MouseButton),
    /// The pointer moves with the button held: `Cb` is the button plus 32.
    /// Only button-event or any-event tracking reports it.
    Drag(MouseButton),
    /// The button goes up: `Cb` is the button, ending in `m`.
    Release(MouseButton),
    /// One wheel notch up: `Cb` 64.
    WheelUp,
    /// One wheel notch down: `Cb` 65.
    WheelDown,
}

impl Mouse {
    /// The tracking modes that report this action, named for a refusal.
    fn tracking(self) -> (TermMode, &'static str) {
        match self {
            Self::Drag(_) => (
                TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION,
                "button-event or any-event mouse tracking (CSI ? 1002 h or CSI ? 1003 h)",
            ),
            _ => (
                TermMode::MOUSE_MODE,
                "mouse tracking (CSI ? 1000 h, CSI ? 1002 h or CSI ? 1003 h)",
            ),
        }
    }

    /// The SGR button code `Cb` and final byte.
    fn code(self) -> (u8, char) {
        match self {
            Self::Press(button) => (button.code(), 'M'),
            Self::Drag(button) => (button.code() + 32, 'M'),
            Self::Release(button) => (button.code(), 'm'),
            Self::WheelUp => (64, 'M'),
            Self::WheelDown => (65, 'M'),
        }
    }
}

/// The emulated host's state that decides whether and how it reports input.
pub(crate) struct Host {
    pub(crate) mode: TermMode,
    pub(crate) rows: usize,
    pub(crate) columns: usize,
}

impl Host {
    pub(crate) fn focus(&self, focused: bool) -> Result<Vec<u8>, String> {
        self.require(TermMode::FOCUS_IN_OUT, "focus reporting (CSI ? 1004 h)")?;
        Ok(if focused { b"\x1b[I" } else { b"\x1b[O" }.to_vec())
    }

    pub(crate) fn paste(&self, text: &[u8]) -> Result<Vec<u8>, String> {
        self.require(TermMode::BRACKETED_PASTE, "bracketed paste (CSI ? 2004 h)")?;
        Ok([b"\x1b[200~", text, b"\x1b[201~"].concat())
    }

    /// `row` and `column` are 0-based; SGR reports them 1-based, column first.
    pub(crate) fn mouse(
        &self,
        action: Mouse,
        row: usize,
        column: usize,
    ) -> Result<Vec<u8>, String> {
        self.require(TermMode::SGR_MOUSE, "SGR mouse encoding (CSI ? 1006 h)")?;
        let (tracking, name) = action.tracking();
        self.require(tracking, name)?;
        if row >= self.rows || column >= self.columns {
            return Err(format!(
                "cell ({row}, {column}) is outside the {}x{} screen",
                self.rows, self.columns
            ));
        }
        let (code, end) = action.code();
        Ok(format!("\x1b[<{code};{};{}{end}", column + 1, row + 1).into_bytes())
    }

    fn require(&self, needed: TermMode, name: &str) -> Result<(), String> {
        if self.mode.intersects(needed) {
            Ok(())
        } else {
            Err(format!(
                "a real terminal would not send this: the application has not enabled {name}"
            ))
        }
    }
}
