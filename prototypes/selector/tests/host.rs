use alacritty_terminal::{
    Term,
    event::{Event, EventListener},
    grid::Dimensions,
    index::{Column, Line},
    term::Config,
    vte::ansi::Processor,
};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, SlavePty};
use std::{
    io::{Read, Write},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

struct Size {
    rows: usize,
    cols: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

struct Events(Sender<Event>);
impl EventListener for Events {
    fn send_event(&self, event: Event) {
        self.0.send(event).unwrap();
    }
}

struct Host {
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    slave: Option<Box<dyn SlavePty + Send>>,
    writer: Box<dyn Write + Send>,
    output: Receiver<Vec<u8>>,
    events: Receiver<Event>,
    term: Term<Events>,
    parser: Processor,
    bytes: Vec<u8>,
}

impl Host {
    fn spawn(args: &[&str]) -> Self {
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize {
                rows: 32,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_monitor-selector-prototype"));
        command.args(args);
        command.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(command).unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let (sender, output) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if sender.send(buffer[..count].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) if error.raw_os_error() == Some(5) => break,
                    Err(error) => panic!("Host PTY read failed: {error}"),
                }
            }
        });
        let (sender, events) = mpsc::channel();
        Self {
            child,
            master: pair.master,
            slave: Some(pair.slave),
            writer,
            output,
            events,
            term: Term::new(
                Config::default(),
                &Size {
                    rows: 32,
                    cols: 100,
                },
                Events(sender),
            ),
            parser: Processor::new(),
            bytes: Vec::new(),
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }

    fn read(&mut self) {
        if let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(40)) {
            self.parser.advance(&mut self.term, &bytes);
            self.bytes.extend(bytes);
            while let Ok(event) = self.events.try_recv() {
                if let Event::PtyWrite(reply) = event {
                    self.send(reply.as_bytes());
                }
            }
        }
    }

    fn text(&self) -> String {
        (0..self.term.screen_lines())
            .map(|row| {
                (0..self.term.columns())
                    .map(|col| self.term.grid()[Line(row as i32)][Column(col)].c)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn expect(&mut self, text: &str) {
        self.wait_until(|host| host.text().contains(text), text);
    }

    fn wait_until(&mut self, predicate: impl Fn(&Self) -> bool, description: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !predicate(self) && Instant::now() < deadline {
            self.read();
        }
        assert!(predicate(self), "Missing {description:?}\n{}", self.text());
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        self.term.resize(Size {
            rows: rows.into(),
            cols: cols.into(),
        });
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
    }

    fn finish(&mut self, input_count: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.read();
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "Prototype exit: {status}\n{}",
                    self.text()
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Prototype failed to exit\n{}",
                self.text()
            );
        }
        self.expect(&format!("synthetic input events: {input_count}"));
        for marker in [
            b"\x1b[?1049l".as_slice(),
            b"\x1b[?25h",
            b"\x1b[?2004l",
            b"\x1b[?1006l",
            b"\x1b]7501;state=clear\x1b\\",
        ] {
            assert!(
                self.bytes
                    .windows(marker.len())
                    .any(|bytes| bytes == marker),
                "Missing cleanup marker {marker:?}"
            );
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if self.child.try_wait().unwrap().is_none() {
            self.child.kill().unwrap();
            self.child.wait().unwrap();
        }
        self.slave.take();
    }
}

#[test]
fn actual_host_launch_paste_fragmentation_resize_alt_and_cleanup() {
    {
        let mut host = Host::spawn(&[]);
        host.expect("Needs attention");
        host.send(b"\x1b[200~\x1dx\nno matching label\x1b[201~");
        host.expect("No matching sessions");
        host.send(b"\x1b");
        host.expect("Cancelled");
        host.send(b"z");
        host.expect("Synthetic application received");
        host.send(b"\x1d");
        host.read();
        host.send(b"s");
        host.expect("Highlighting does not select");
        host.resize(14, 26);
        host.expect("PROTOTYPE");
        host.resize(32, 100);
        host.expect("Highlighting does not select");
        host.send(b"\x1d");
        host.read();
        host.send(b"x");
        host.finish(1);
    }
    {
        let mut host = Host::spawn(&["--no-hints"]);
        host.wait_until(
            |host| {
                host.text()
                    .lines()
                    .nth(27)
                    .unwrap()
                    .contains("Review needed")
            },
            "title-only toast",
        );
        host.send(b"\x1b[13~");
        host.wait_until(
            |host| host.text().lines().nth(27).unwrap().contains("Ctrl+]+j"),
            "toast hint row",
        );
        host.send(b"Build");
        host.wait_until(
            |host| host.text().lines().nth(2).unwrap().contains("Build"),
            "search stays responsive during animation",
        );
        host.send(b"\x1b[14~\x1b[14~");
        host.wait_until(
            |host| host.text().lines().nth(26).unwrap().contains("Build"),
            "queued attention toast",
        );
        assert_eq!(
            host.bytes.iter().filter(|byte| **byte == 7).count(),
            2,
            "one bell per presented episode, not per frame or repeated F4"
        );
        host.wait_until(
            |host| !host.text().lines().nth(27).unwrap().contains("Ctrl+]+j"),
            "toast naturally fades and disappears",
        );
        host.send(b"\x1dj\x1dj");
        host.wait_until(
            |host| {
                host.bytes
                    .windows(b"7501;state=working".len())
                    .any(|bytes| bytes == b"7501;state=working")
            },
            "monitor returns to working after all pending attention is acknowledged",
        );
        assert_eq!(
            host.bytes
                .windows(b"7501;state=blocked".len())
                .filter(|bytes| *bytes == b"7501;state=blocked")
                .count(),
            1
        );
        assert_eq!(
            host.bytes
                .windows(b"7501;state=working".len())
                .filter(|bytes| *bytes == b"7501;state=working")
                .count(),
            1
        );
        host.send(b"\x1dx");
        host.finish(0);
    }
    {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config.jsonc");
        std::fs::write(
            &config,
            r#"{"monitor":{"bell":false,"show_key_hints":false}}"#,
        )
        .unwrap();
        let mut host = Host::spawn(&["--config", config.to_str().unwrap()]);
        host.wait_until(
            |host| {
                host.text()
                    .lines()
                    .nth(27)
                    .unwrap()
                    .trim_end()
                    .ends_with("│Review needed│")
            },
            "configured silent initial toast",
        );
        std::fs::write(&config, r#"{"monitor":{"bell":true}}"#).unwrap();
        host.send(b"\x1b[14~");
        host.wait_until(
            |host| {
                host.text()
                    .lines()
                    .nth(27)
                    .unwrap()
                    .trim_end()
                    .ends_with("│Shell│")
            },
            "silent queued toast uses startup snapshot",
        );
        assert_eq!(host.bytes.iter().filter(|byte| **byte == 7).count(), 0);
        assert!(
            host.bytes
                .windows(b"7501;state=blocked".len())
                .any(|bytes| bytes == b"7501;state=blocked")
        );
        host.send(b"\x1dx");
        host.finish(0);
    }
    let mut host = Host::spawn(&["--escape-prefix", "Ctrl+["]);
    host.expect("Needs attention");
    host.send(b"\x1bx");
    host.finish(0);
}
