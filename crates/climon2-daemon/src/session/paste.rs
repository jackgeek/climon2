const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// Whether the input actually written to the PTY left an application paste
/// open, including a partially written marker (section 1.5).
#[derive(Debug, Default)]
pub(super) struct PasteTracker {
    inside: bool,
    marker: Vec<u8>,
}

impl PasteTracker {
    pub fn written(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.byte(*byte);
        }
    }

    fn byte(&mut self, byte: u8) {
        self.marker.push(byte);
        while !self.expected().starts_with(&self.marker) {
            self.marker.remove(0);
        }
        if self.marker.len() == self.expected().len() {
            self.marker.clear();
            self.inside = !self.inside;
        }
    }

    fn expected(&self) -> &'static [u8] {
        if self.inside { PASTE_END } else { PASTE_START }
    }

    /// PTY bytes that finish an open paste while bracketed paste is still
    /// enabled; the tracker starts over either way.
    pub fn cleanup(&mut self, bracketed_paste: bool) -> Vec<u8> {
        let bytes = if bracketed_paste {
            self.remainder()
        } else {
            Vec::new()
        };
        *self = Self::default();
        bytes
    }

    fn remainder(&self) -> Vec<u8> {
        let rest = &self.expected()[self.marker.len()..];
        if self.inside {
            return rest.to_vec();
        }
        if self.marker.is_empty() {
            return Vec::new();
        }
        [rest, PASTE_END].concat()
    }
}

#[cfg(test)]
mod tests {
    use super::PasteTracker;

    #[test]
    fn paste_cleanup_finishes_a_partially_written_open_marker() {
        let mut paste = PasteTracker::default();
        paste.written(b"\x1b[20");
        assert_eq!(paste.cleanup(true), b"0~\x1b[201~");
    }

    #[test]
    fn paste_cleanup_closes_payload_and_finishes_a_partial_close_marker() {
        let mut paste = PasteTracker::default();
        paste.written(b"\x1b[200~payload");
        assert_eq!(paste.cleanup(true), b"\x1b[201~");

        paste.written(b"\x1b[200~payload\x1b[20");
        assert_eq!(paste.cleanup(true), b"1~");
    }

    #[test]
    fn closed_pastes_and_ordinary_input_need_no_cleanup() {
        let mut paste = PasteTracker::default();
        paste.written(b"typed \x1b[200~pasted\x1b[201~ more");
        assert!(paste.cleanup(true).is_empty());
        paste.written(b"\x1b[2\x1b[200~x");
        assert_eq!(paste.cleanup(true), b"\x1b[201~");
    }

    #[test]
    fn markers_split_across_writes_are_tracked() {
        let mut paste = PasteTracker::default();
        for byte in b"\x1b[200~a\x1b[201~\x1b[200" {
            paste.written(&[*byte]);
        }
        assert_eq!(paste.cleanup(true), b"~\x1b[201~");
    }

    #[test]
    fn disabled_bracketed_paste_discards_cleanup_state() {
        let mut paste = PasteTracker::default();
        paste.written(b"\x1b[200~payload");
        assert!(paste.cleanup(false).is_empty());
        assert!(paste.cleanup(true).is_empty());
    }
}
