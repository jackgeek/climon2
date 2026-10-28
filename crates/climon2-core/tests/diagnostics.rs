use climon2_core::diagnostics::{CappedWriter, LOG_LIMIT, subscriber};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// #25: every file is capped at 8 MiB with exactly one final notice.
#[test]
fn log_cap_keeps_the_final_notice_inside_eight_mib() {
    let capture = Capture::default();
    let mut writer = CappedWriter::new(capture.clone());
    writer.write_all(&vec![b'x'; LOG_LIMIT - 10]).unwrap();
    writer
        .write_all(b"more bytes after reaching the cap")
        .unwrap();
    writer.write_all(b"and another message").unwrap();
    writer.flush().unwrap();
    let bytes = capture.0.lock().unwrap();
    assert_eq!(bytes.len(), LOG_LIMIT);
    assert!(bytes.ends_with(b"\nlog limit reached\n"));
    assert_eq!(
        bytes
            .windows(b"log limit reached".len())
            .filter(|value| *value == b"log limit reached")
            .count(),
        1
    );
}

/// #25: EnvFilter is honored, output is plain text, and invalid filters fail explicitly.
#[test]
fn diagnostic_filter_controls_events_without_ansi() {
    let capture = Capture::default();
    let dispatch = subscriber(CappedWriter::new(capture.clone()), Some("warn")).unwrap();
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!("hidden event");
        tracing::warn!(pid = 42, "visible event");
    });
    let bytes = capture.0.lock().unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.contains("WARN"));
    assert!(text.contains("visible event"));
    assert!(text.contains("pid=42"));
    assert!(!text.contains("hidden event"));
    assert!(!text.contains('\x1b'));
    assert!(subscriber(CappedWriter::new(io::sink()), Some("[")).is_err());
}

/// #25: a failed sink is a real error, not success-shaped log output.
#[test]
fn capped_log_preserves_write_and_flush_errors() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("write failed"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("flush failed"))
        }
    }
    let mut writer = CappedWriter::new(Broken);
    assert!(writer.write_all(b"event").is_err());
    assert!(writer.flush().is_err());
}
