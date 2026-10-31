//! THROWAWAY #48/#43: serial bounded measurements; synthetic fixtures.
use super::{Args, LIVE, PEAK, arg, emit, max_rss_bytes, millis, secs, term, text, unit};
use alacritty_embedding_probe::delivery::{DeliveryBudget, Outbound};
use alacritty_embedding_probe::event_delivery;
use alacritty_embedding_probe::followup::{PtyBuffer, Resizes, select_shared_link};
use alacritty_embedding_probe::restore::{SlicedRestore, Snapshot, restore};
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Hyperlink;
use std::io::{ErrorKind, Read, Write};
use std::num::NonZeroU16;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant};
use vte::ansi::{Processor, StdSyncHandler};

fn feed(parser: &mut Processor<StdSyncHandler>, term: &mut Term<VoidListener>, bytes: &[u8]) {
    for chunk in bytes.chunks(64) {
        parser.advance(term, chunk);
    }
}

fn row(kind: &str, i: usize, cols: usize) -> Vec<u8> {
    if kind == "log" {
        let body = format!(
            "\x1b[{}mrow {i:06} {}\x1b[0m",
            31 + i % 6,
            "x".repeat(cols.min(60) - 12)
        );
        if i == 0 {
            body.into_bytes()
        } else {
            [b"\r\n".as_slice(), body.as_bytes()].concat()
        }
    } else {
        assert_eq!(kind, "sgr_cells");
        (0..cols)
            .flat_map(|c| unit("sgr_cells", i * cols + c, cols, 0))
            .collect()
    }
}

fn fill(term: &mut Term<VoidListener>, args: &Args) {
    let mut parser = Processor::<StdSyncHandler>::new();
    for i in 0..arg(args, "history", 0) + term.screen_lines() {
        feed(
            &mut parser,
            term,
            &row(text(args, "fill", "log"), i, term.columns()),
        );
    }
    feed(&mut parser, term, b"\x1b[H");
}

pub fn links(args: &Args) {
    let empty = LIVE.load(Relaxed);
    let mut term = term(args);
    let mut parser = Processor::<StdSyncHandler>::new();
    let before = LIVE.load(Relaxed);
    let n = arg(args, "cells", 10_000);
    let bytes = arg(args, "link_bytes", 4080);
    let kind = text(args, "kind", "unique");
    let uri = if arg(args, "uri_segments", 0) == 1 {
        let first = (bytes - 1) / 2;
        format!("{};{}", "p".repeat(first), "p".repeat(bytes - first - 1))
    } else {
        "p".repeat(bytes)
    };
    let shared = Hyperlink::new(Some("fixed"), "p".repeat(bytes));
    let started = Instant::now();
    for i in 0..n {
        if i == 0 || kind != "single" {
            let id = if kind == "unique" {
                format!("{i:08}")
            } else {
                "fixed".into()
            };
            let osc = if arg(args, "no_id", 0) == 1 {
                format!("\x1b]8;;{uri}\x1b\\")
            } else {
                format!("\x1b]8;id={id};{uri}\x1b\\")
            };
            assert!(
                osc.len() - 4 <= 4096,
                "fixture must respect decided OSC cap"
            );
            feed(&mut parser, &mut term, osc.as_bytes());
            if kind == "interned" {
                select_shared_link(&mut term, shared.clone());
            }
        }
        feed(&mut parser, &mut term, b"x");
    }
    feed(&mut parser, &mut term, b"\x1b]8;;\x1b\\");
    let filled = LIVE.load(Relaxed);
    let history = term.grid().history_size();
    drop(term);
    drop(parser);
    drop(shared);
    emit(
        args,
        vec![
            ("retained_growth_bytes", (filled - before).to_string()),
            ("released_bytes", (filled - LIVE.load(Relaxed)).to_string()),
            (
                "after_drop_growth_bytes",
                (LIVE.load(Relaxed) - empty).to_string(),
            ),
            ("history_rows", history.to_string()),
            ("fill_seconds", secs(started.elapsed())),
            ("max_rss_bytes", max_rss_bytes().to_string()),
        ],
    );
}

fn hash(mut digest: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        digest = (digest ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
    }
    digest
}

const HASH: u64 = 0xcbf29ce484222325;

fn oracle_term(term: &Term<VoidListener>) -> vt100::Parser {
    vt100::Parser::new(
        NonZeroU16::new(term.screen_lines() as u16).unwrap(),
        NonZeroU16::new(term.columns() as u16).unwrap(),
        0,
    )
}

fn hash_row(mut digest: u64, host: &vt100::Parser, line: u16, cols: usize) -> u64 {
    for c in 0..cols {
        digest = hash(
            digest,
            format!("{:?}", host.screen().cell(line, c as u16).unwrap()).as_bytes(),
        );
    }
    digest
}

/// Observe each evicted row before feeding the next, then every final screen row.
/// Thus all 100k historical rows are checked with a screen-sized oracle.
pub fn oracle(args: &Args) {
    let mut term = term(args);
    let mut source = oracle_term(&term);
    let mut parser = Processor::<StdSyncHandler>::new();
    let total = arg(args, "history", 0) + term.screen_lines();
    let cols = term.columns();
    let rows = term.screen_lines();
    let mut expected = HASH;
    for i in 0..total {
        if i >= rows {
            expected = hash_row(expected, &source, 0, cols);
        }
        let bytes = row(text(args, "fill", "log"), i, cols);
        feed(&mut parser, &mut term, &bytes);
        source.process(&bytes);
    }
    feed(&mut parser, &mut term, b"\x1b[H");
    source.process(b"\x1b[H");
    for line in 0..rows {
        expected = hash_row(expected, &source, line as u16, cols);
    }
    let mut host = oracle_term(&term);
    let mut actual = HASH;
    let mut bytes_hash = HASH;
    let mut restored_bytes = 0;
    let mut sliced = SlicedRestore::new(&mut term);
    for i in 0..total {
        if i >= rows {
            actual = hash_row(actual, &host, 0, cols);
        }
        let done = sliced.step(1);
        let bytes = sliced.take_output();
        bytes_hash = hash(bytes_hash, &bytes);
        restored_bytes += bytes.len();
        host.process(&bytes);
        assert_eq!(done, i + 1 == total);
    }
    assert!(sliced.finish().is_empty());
    for line in 0..rows {
        actual = hash_row(actual, &host, line as u16, cols);
    }
    assert_eq!(
        actual, expected,
        "independent oracle differs in text/style/history"
    );
    assert_eq!(
        host.screen().cursor_position(),
        source.screen().cursor_position()
    );
    drop(host);
    drop(source);
    let baseline = restore(&mut term);
    assert_eq!(hash(HASH, &baseline), bytes_hash);
    drop(baseline);
    let cloned = std::thread::spawn({
        let snapshot = Snapshot::capture(&mut term);
        move || snapshot.serialize()
    })
    .join()
    .unwrap();
    assert_eq!(hash(HASH, &cloned), bytes_hash);
    let mut resizes = Resizes::default();
    resizes.request(cols - 2, rows);
    resizes.request(cols - 1, rows);
    resizes.apply(&mut term);
    assert_eq!(resizes.reflows(), 1);
    emit(
        args,
        vec![
            ("oracle", "\"all_rows_text_style_cursor_pass\"".into()),
            ("checked_rows", total.to_string()),
            ("restore_bytes", restored_bytes.to_string()),
            ("byte_hash", bytes_hash.to_string()),
            ("reflows", resizes.reflows().to_string()),
            ("max_rss_bytes", max_rss_bytes().to_string()),
        ],
    );
}

struct Producer {
    socket: UnixStream,
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<usize>,
}

impl Producer {
    fn new(rate: usize) -> Self {
        let (mut writer, socket) = UnixStream::pair().unwrap();
        socket.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let thread = std::thread::spawn(move || {
            let started = Instant::now();
            let mut produced = 0;
            while !done.load(Relaxed) {
                writer.write_all(&[b'x'; 4096]).unwrap();
                produced += 4096;
                let due = started + Duration::from_secs_f64(produced as f64 / rate as f64);
                if let Some(wait) = due.checked_duration_since(Instant::now()) {
                    // Synthetic PTY pacing, NOT the delivery event loop.
                    std::thread::sleep(wait);
                }
            }
            produced
        });
        Self {
            socket,
            stop,
            thread,
        }
    }

    fn finish(
        mut self,
        parser: &mut Processor<StdSyncHandler>,
        term: &mut Term<VoidListener>,
    ) -> usize {
        self.stop.store(true, Relaxed);
        self.socket.set_nonblocking(false).unwrap();
        let mut bytes = [0; 4096];
        loop {
            let n = self.socket.read(&mut bytes).unwrap();
            if n == 0 {
                break;
            }
            feed(parser, term, &bytes[..n]);
        }
        self.thread.join().unwrap()
    }
}

fn read_available(socket: &mut UnixStream, buffer: &mut PtyBuffer) -> usize {
    let mut bytes = [0; 4096];
    let mut received = 0;
    // One bounded read per pump step. A full buffer leaves bytes in the socket.
    let space = buffer.remaining().min(bytes.len());
    if space == 0 {
        return 0;
    }
    match socket.read(&mut bytes[..space]) {
        Ok(n) => {
            buffer.push(&bytes[..n]).unwrap();
            received += n;
        }
        Err(error) if error.kind() == ErrorKind::WouldBlock => {}
        Err(error) => panic!("PTY read failed: {error}"),
    }
    received
}

pub fn snapshot(args: &Args) {
    let mut term = term(args);
    fill(&mut term, args);
    let base = LIVE.load(Relaxed);
    PEAK.store(base, Relaxed);
    let mut producer = Producer::new(arg(args, "rate", 4 << 20));
    let mut parser = Processor::<StdSyncHandler>::new();
    let mut stalls = Vec::new();
    let mut waiting = PtyBuffer::new(arg(args, "pty_cap", 16 << 20));
    let started = Instant::now();
    let mut buffered = 0;
    let mut during = 0;
    let mut full = false;
    let bytes = match text(args, "strategy", "blocking") {
        "blocking" => {
            let stall = Instant::now();
            let bytes = restore(&mut term);
            stalls.push(stall.elapsed());
            bytes
        }
        "clone" => {
            let stall = Instant::now();
            let snapshot = Snapshot::capture(&mut term);
            stalls.push(stall.elapsed());
            let worker = std::thread::spawn(move || snapshot.serialize());
            while !worker.is_finished() {
                let stall = Instant::now();
                let n = read_available(&mut producer.socket, &mut waiting);
                during += n;
                feed(&mut parser, &mut term, &waiting.drain());
                stalls.push(stall.elapsed());
                std::thread::yield_now();
            }
            worker.join().unwrap()
        }
        "sliced" => {
            let stall = Instant::now();
            let mut sliced = SlicedRestore::new(&mut term);
            stalls.push(stall.elapsed());
            loop {
                let stall = Instant::now();
                buffered += read_available(&mut producer.socket, &mut waiting);
                full |= waiting.remaining() == 0;
                let done = sliced.step(arg(args, "slice_rows", 16));
                stalls.push(stall.elapsed());
                if done {
                    break;
                }
            }
            sliced.finish()
        }
        other => panic!("unknown strategy {other}"),
    };
    let elapsed = started.elapsed();
    let peak = PEAK.load(Relaxed);
    let digest = hash(HASH, &bytes);
    let total = bytes.len();
    drop(bytes);
    feed(&mut parser, &mut term, &waiting.drain());
    let produced = producer.finish(&mut parser, &mut term);
    stalls.sort_unstable();
    emit(
        args,
        vec![
            ("restore_bytes", total.to_string()),
            ("byte_hash", digest.to_string()),
            ("restore_seconds", secs(elapsed)),
            (
                "mib_per_s",
                format!("{:.3}", total as f64 / 1048576.0 / elapsed.as_secs_f64()),
            ),
            ("pump_stall_p50_ms", millis(stalls[stalls.len() / 2])),
            ("pump_stall_max_ms", millis(*stalls.last().unwrap())),
            ("pump_steps", stalls.len().to_string()),
            ("heap_base_bytes", base.to_string()),
            ("restore_heap_peak_bytes", peak.to_string()),
            ("pty_produced_bytes", produced.to_string()),
            ("pty_buffered_bytes", buffered.to_string()),
            ("pty_parsed_during_bytes", during.to_string()),
            ("pty_backpressure", full.to_string()),
            ("live_queue_would_overflow", (during > 16 << 20).to_string()),
            ("max_rss_bytes", max_rss_bytes().to_string()),
        ],
    );
}

/// Post-snapshot catch-up is scheduled as 64 B callbacks, never one 16 MiB feed.
pub fn replay(args: &Args) {
    let mut term = term(args);
    fill(&mut term, args);
    let mut parser = Processor::<StdSyncHandler>::new();
    let bytes = vec![b'x'; arg(args, "replay_bytes", 16 << 20)];
    let mut stalls = Vec::with_capacity(bytes.len().div_ceil(64));
    let started = Instant::now();
    for chunk in bytes.chunks(64) {
        let callback = Instant::now();
        parser.advance(&mut term, chunk);
        stalls.push(callback.elapsed());
    }
    let elapsed = started.elapsed();
    let mut host = oracle_term(&term);
    host.process(&bytes);
    for (line, expected) in host.screen().rows(0, term.columns() as u16).enumerate() {
        let actual: String = (0..term.columns())
            .map(|c| {
                term.grid()[alacritty_terminal::index::Line(line as i32)]
                    [alacritty_terminal::index::Column(c)]
                .c
            })
            .collect();
        assert_eq!(actual.trim_end(), expected);
    }
    assert_eq!(
        host.screen().cursor_position(),
        (
            term.grid().cursor.point.line.0 as u16,
            term.grid().cursor.point.column.0 as u16
        )
    );
    stalls.sort_unstable();
    emit(
        args,
        vec![
            ("replay_bytes", bytes.len().to_string()),
            ("replay_seconds", secs(elapsed)),
            ("pump_stall_p50_ms", millis(stalls[stalls.len() / 2])),
            ("pump_stall_max_ms", millis(*stalls.last().unwrap())),
            (
                "replay_mib_per_s",
                format!(
                    "{:.3}",
                    bytes.len() as f64 / 1048576.0 / elapsed.as_secs_f64()
                ),
            ),
            ("oracle", "\"catchup_text_cursor_pass\"".into()),
            ("max_rss_bytes", max_rss_bytes().to_string()),
        ],
    );
}

pub fn event(args: &Args) {
    let mut term = term(args);
    fill(&mut term, args);
    let bytes = restore(&mut term);
    let total = bytes.len();
    let expected = hash(HASH, &bytes);
    let (mut socket, mut peer) = UnixStream::pair().unwrap();
    socket.set_nonblocking(true).unwrap();
    let rate = arg(args, "rate", 0);
    let reader = std::thread::spawn(move || {
        let mut buffer = [0; 65536];
        let started = Instant::now();
        let (mut received, mut digest) = (0, HASH);
        loop {
            let n = peer.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }

            received += n;
            digest = hash(digest, &buffer[..n]);
            if rate > 0 {
                let due = started + Duration::from_secs_f64(received as f64 / rate as f64);
                if let Some(wait) = due.checked_duration_since(Instant::now()) {
                    std::thread::sleep(wait);
                }
            }
        }
        (received, digest)
    });
    let started = Instant::now();
    let budget = DeliveryBudget {
        chunk_bytes: arg(args, "chunk", 4096),
        queue_bytes: 16 << 20,
        no_progress: Duration::from_secs(5),
        restoration_deadline: Duration::from_secs_f64(30.0 + total as f64 / (4 << 20) as f64),
    };
    let mut outbound = Outbound::restoring(budget, bytes, started);
    let report = event_delivery::deliver(&mut socket, &mut outbound).unwrap();
    let elapsed = started.elapsed();
    drop(socket);
    let (received, digest) = reader.join().unwrap();
    assert_eq!(received, total);
    assert_eq!(digest, expected);
    emit(
        args,
        vec![
            ("restore_bytes", total.to_string()),
            ("complete", "true".into()),
            ("deliver_seconds", secs(elapsed)),
            (
                "mib_per_s",
                format!("{:.3}", total as f64 / 1048576.0 / elapsed.as_secs_f64()),
            ),
            ("worst_progress_gap_ms", millis(report.worst_gap)),
            ("writes", report.writes.to_string()),
            ("readiness_waits", report.waits.to_string()),
            ("max_rss_bytes", max_rss_bytes().to_string()),
        ],
    );
}
