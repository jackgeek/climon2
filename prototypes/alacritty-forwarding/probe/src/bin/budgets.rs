//! THROWAWAY #48/#43: bounded cost measurements, one JSON line per run.
//!
//! Every allocation is counted; exceeding BUDGET_CEILING_BYTES (default
//! 1 GiB) fails the allocation, which aborts this process only. measure.py
//! additionally kills the process on an RSS or wall-time ceiling.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use alacritty_embedding_probe::admission::OscAdmission;
use alacritty_embedding_probe::delivery::{DeliveryBudget, Outbound};
use alacritty_embedding_probe::restore::restore;
use alacritty_embedding_probe::timing::{FirstBeginTimeout, flush_if_due};
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use vte::ansi::{Processor, StdSyncHandler};

#[path = "../followup_bench.rs"]
mod followup_bench;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static CEILING: AtomicUsize = AtomicUsize::new(1 << 30);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !reserve(layout.size()) {
            return std::ptr::null_mut();
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > layout.size() && !reserve(new_size - layout.size()) {
            return std::ptr::null_mut();
        }
        let moved = unsafe { System.realloc(ptr, layout, new_size) };
        if moved.is_null() {
            if new_size > layout.size() {
                LIVE.fetch_sub(new_size - layout.size(), Relaxed);
            }
        } else if new_size < layout.size() {
            LIVE.fetch_sub(layout.size() - new_size, Relaxed);
        }
        moved
    }
}

fn reserve(bytes: usize) -> bool {
    let live = LIVE.fetch_add(bytes, Relaxed) + bytes;
    if live > CEILING.load(Relaxed) {
        LIVE.fetch_sub(bytes, Relaxed);
        return false;
    }
    PEAK.fetch_max(live, Relaxed);
    true
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[repr(C)]
struct Rusage {
    utime_sec: i64,
    utime_usec: i32,
    stime_sec: i64,
    stime_usec: i32,
    maxrss: i64,
    rest: [i64; 13],
}

unsafe extern "C" {
    fn getrusage(who: i32, usage: *mut Rusage) -> i32;
}

fn usage() -> Rusage {
    let mut usage = std::mem::MaybeUninit::<Rusage>::zeroed();
    unsafe {
        getrusage(0, usage.as_mut_ptr());
        usage.assume_init()
    }
}

/// macOS reports ru_maxrss in bytes.
fn max_rss_bytes() -> i64 {
    usage().maxrss
}

fn cpu_seconds() -> f64 {
    let usage = usage();
    usage.utime_sec as f64
        + usage.utime_usec as f64 / 1e6
        + usage.stime_sec as f64
        + usage.stime_usec as f64 / 1e6
}

type Args = BTreeMap<String, String>;

fn arg(args: &Args, key: &str, default: usize) -> usize {
    args.get(key)
        .map_or(default, |value| value.parse().unwrap())
}

fn text<'a>(args: &'a Args, key: &str, default: &'a str) -> &'a str {
    args.get(key).map_or(default, String::as_str)
}

fn term(args: &Args) -> Term<VoidListener> {
    let config = Config {
        scrolling_history: arg(args, "history", 0),
        ..Config::default()
    };
    let size = TermSize::new(arg(args, "cols", 80), arg(args, "rows", 24));
    Term::new(config, &size, VoidListener)
}

/// One repeatable unit of a synthetic stream; `i` varies content.
fn unit(kind: &str, i: usize, cols: usize, link_bytes: usize) -> Vec<u8> {
    match kind {
        "log" => format!(
            "\x1b[{}m2026-10-31T12:00:{:02}Z INFO\x1b[0m [worker-{}] processed item {} in {}.{}ms\r\n",
            31 + i % 6,
            i % 60,
            i % 16,
            i,
            i % 97,
            i % 10
        )
        .into_bytes(),
        "tui" => {
            let row = i % 24 + 1;
            let body: String = (0..cols.saturating_sub(12))
                .map(|c| (b'a' + ((c + i) % 26) as u8) as char)
                .collect();
            format!(
                "\x1b[{row};1H\x1b[38;5;{}m\x1b[48;5;{}m{body}\x1b[0m\x1b[K",
                i % 256,
                (i * 7) % 256
            )
            .into_bytes()
        }
        "unicode" => "界e\u{301}a\u{308}字\r\n".as_bytes().to_vec(),
        "sgr_cells" => format!("\x1b[38;5;{};48;5;{}mX", i % 256, (i / 256) % 256).into_bytes(),
        "rep" => b"X\x1b[65535b".to_vec(),
        "rep_insert" => b"\x1b[4hX\x1b[65535b".to_vec(),
        "decaln" => b"\x1b#8".to_vec(),
        "clear" => b"\x1b[2J".to_vec(),
        "scroll" => b"\x1b[65535S".to_vec(),
        "il_dl" => b"\x1b[65535L\x1b[65535M".to_vec(),
        "ich" => b"\x1b[65535@".to_vec(),
        "combining" => "e\u{301}\u{302}\u{303}\u{304}\u{305}\u{306}\u{307}\u{308}\u{309}\u{30a}"
            .as_bytes()
            .to_vec(),
        "osc8_cell" => {
            let mut bytes = b"\x1b]8;;https://h/".to_vec();
            bytes.extend(format!("{i:08}").bytes());
            bytes.resize(bytes.len() + link_bytes.saturating_sub(16), b'p');
            bytes.extend(b"\x1b\\x");
            bytes
        }
        "title_stack" => {
            let mut bytes = format!("\x1b]2;{i:08}").into_bytes();
            bytes.resize(bytes.len() + link_bytes.saturating_sub(8), b't');
            bytes.extend(b"\x07\x1b[22;0t");
            bytes
        }
        "dcs_body" | "apc_body" | "osc_body" => vec![b'q'; 4096],
        _ => panic!("unknown stream {kind}"),
    }
}

fn opener(kind: &str) -> &'static [u8] {
    match kind {
        "dcs_body" => b"\x1bPq",
        "apc_body" => b"\x1b_G",
        "osc_body" => b"\x1b]8;;",
        _ => b"",
    }
}

fn prefill(term: &mut Term<VoidListener>, parser: &mut Processor<StdSyncHandler>, args: &Args) {
    let rows = arg(args, "history", 0) + arg(args, "rows", 24);
    let kind = text(args, "fill", "log");
    let cols = term.columns();
    let units = match kind {
        "log" => rows,
        "sgr_cells" | "osc8_cell" | "combining" => rows * cols,
        "none" => 0,
        _ => panic!("unknown fill {kind}"),
    };
    let link_bytes = arg(args, "link_bytes", 0);
    let mut buffer = Vec::new();
    for i in 0..units {
        buffer.extend(unit(kind, i, cols, link_bytes));
        if buffer.len() > 1 << 16 {
            parser.advance(term, &buffer);
            buffer.clear();
        }
    }
    parser.advance(term, &buffer);
}

fn emit(args: &Args, fields: Vec<(&str, String)>) {
    let mut body: Vec<String> = args
        .iter()
        .map(|(key, value)| format!("\"{key}\":\"{value}\""))
        .collect();
    body.extend(
        fields
            .into_iter()
            .map(|(key, value)| format!("\"{key}\":{value}")),
    );
    println!("{{{}}}", body.join(","));
}

fn secs(duration: Duration) -> String {
    format!("{:.6}", duration.as_secs_f64())
}

fn millis(duration: Duration) -> String {
    format!("{:.3}", duration.as_secs_f64() * 1e3)
}

/// Feed a stream (optionally through OSC admission) until `bytes` or `millis`.
fn parse(args: &Args) {
    let mut term = term(args);
    let mut parser = Processor::<StdSyncHandler>::new();
    prefill(&mut term, &mut parser, args);
    let kind = text(args, "stream", "log");
    let cols = term.columns();
    let budget_bytes = arg(args, "bytes", 4 << 20);
    let chunk = arg(args, "chunk", 4096);
    let limit = Duration::from_millis(arg(args, "millis", 3000) as u64);
    let link_bytes = arg(args, "link_bytes", 0);
    let cap = arg(args, "osc_cap", 0);
    let mut admission = (cap > 0).then(|| OscAdmission::new(cap));
    let live_before = LIVE.load(Relaxed);
    PEAK.store(live_before, Relaxed);
    let (mut fed, mut i, mut max_chunk) = (0usize, 0usize, Duration::ZERO);
    let mut buffer = opener(kind).to_vec();
    let mut forwarded = Vec::new();
    let cpu = cpu_seconds();
    let started = Instant::now();
    while fed < budget_bytes && started.elapsed() < limit {
        while buffer.len() < chunk {
            buffer.extend(unit(kind, i, cols, link_bytes));
            i += 1;
        }
        let tick = Instant::now();
        match admission.as_mut() {
            Some(admission) => {
                forwarded.clear();
                admission.admit(&buffer, &mut forwarded);
                parser.advance(&mut term, &forwarded);
            }
            None => parser.advance(&mut term, &buffer),
        }
        max_chunk = max_chunk.max(tick.elapsed());
        fed += buffer.len();
        buffer.clear();
    }
    let elapsed = started.elapsed();
    emit(
        args,
        vec![
            ("fed_bytes", fed.to_string()),
            ("seconds", secs(elapsed)),
            ("cpu_seconds", format!("{:.6}", cpu_seconds() - cpu)),
            (
                "mib_per_s",
                format!("{:.3}", fed as f64 / 1048576.0 / elapsed.as_secs_f64()),
            ),
            (
                "ns_per_byte",
                format!("{:.1}", elapsed.as_nanos() as f64 / fed as f64),
            ),
            ("max_chunk_ms", millis(max_chunk)),
            (
                "heap_growth_bytes",
                (LIVE.load(Relaxed) as i64 - live_before as i64).to_string(),
            ),
            (
                "heap_peak_growth_bytes",
                (PEAK.load(Relaxed) - live_before).to_string(),
            ),
            ("max_rss_bytes", max_rss_bytes().to_string()),
            ("history_rows", term.grid().history_size().to_string()),
            (
                "breaches",
                admission.map_or(0, |a| a.breaches()).to_string(),
            ),
        ],
    );
}

/// Engine heap for a geometry/history/fill, plus restore and clone costs.
fn storage(args: &Args) {
    let empty = LIVE.load(Relaxed);
    let mut term = term(args);
    let created = LIVE.load(Relaxed);
    let mut parser = Processor::<StdSyncHandler>::new();
    let parser_bytes = LIVE.load(Relaxed) - created;
    let started = Instant::now();
    prefill(&mut term, &mut parser, args);
    let fill = started.elapsed();
    let filled = LIVE.load(Relaxed);
    let engine = filled - empty - parser_bytes;
    let started = Instant::now();
    let grid_clone = term.grid().clone();
    let clone = started.elapsed();
    let clone_bytes = LIVE.load(Relaxed) - filled;
    drop(grid_clone);
    let before_restore = LIVE.load(Relaxed);
    PEAK.store(before_restore, Relaxed);
    let started = Instant::now();
    let bytes = restore(&mut term);
    let restore_time = started.elapsed();
    let restore_peak = PEAK.load(Relaxed) - before_restore;
    let cells = (term.grid().history_size() + term.screen_lines()) * term.columns();
    emit(
        args,
        vec![
            ("engine_new_bytes", (created - empty).to_string()),
            ("parser_new_bytes", parser_bytes.to_string()),
            ("engine_filled_bytes", engine.to_string()),
            ("history_rows", term.grid().history_size().to_string()),
            ("retained_cells", cells.to_string()),
            (
                "bytes_per_retained_cell",
                format!("{:.2}", engine as f64 / cells as f64),
            ),
            ("fill_seconds", secs(fill)),
            ("grid_clone_bytes", clone_bytes.to_string()),
            ("grid_clone_ms", millis(clone)),
            ("restore_bytes", bytes.len().to_string()),
            ("restore_ms", millis(restore_time)),
            ("restore_heap_peak_bytes", restore_peak.to_string()),
            ("max_rss_bytes", max_rss_bytes().to_string()),
        ],
    );
}

/// Width change with retained history: reflow cost blocks the pump.
fn resize(args: &Args) {
    let mut term = term(args);
    let mut parser = Processor::<StdSyncHandler>::new();
    prefill(&mut term, &mut parser, args);
    let rows = arg(args, "rows", 24);
    let mut worst = Duration::ZERO;
    for cols in [arg(args, "to_cols", 79), arg(args, "cols", 80)] {
        let started = Instant::now();
        term.resize(TermSize::new(cols, rows));
        worst = worst.max(started.elapsed());
    }
    emit(
        args,
        vec![
            ("worst_resize_ms", millis(worst)),
            ("history_rows", term.grid().history_size().to_string()),
            ("max_rss_bytes", max_rss_bytes().to_string()),
        ],
    );
}

/// Worst single synchronized-update flush: BSU, ~2 MiB of `stream`, deadline.
fn sync(args: &Args) {
    let mut term = term(args);
    let mut prefill_parser = Processor::<StdSyncHandler>::new();
    prefill(&mut term, &mut prefill_parser, args);
    let mut parser = Processor::<FirstBeginTimeout>::new();
    let kind = text(args, "stream", "rep");
    let target = arg(args, "bytes", (2 << 20) - 64);
    let mut payload = b"\x1b[?2026h".to_vec();
    let mut i = 0;
    loop {
        let next = unit(kind, i, term.columns(), 0);
        if payload.len() + next.len() > target {
            break;
        }
        payload.extend(next);
        i += 1;
    }
    let started = Instant::now();
    parser.advance(&mut term, &payload);
    let buffering = started.elapsed();
    let buffered = parser.sync_bytes_count();
    let deadline = parser.sync_timeout().deadline().expect("sync active");
    let started = Instant::now();
    assert!(flush_if_due(&mut parser, &mut term, deadline));
    let flush = started.elapsed();
    emit(
        args,
        vec![
            ("payload_bytes", payload.len().to_string()),
            ("buffered_bytes", buffered.to_string()),
            ("buffer_ms", millis(buffering)),
            ("flush_ms", millis(flush)),
            ("max_rss_bytes", max_rss_bytes().to_string()),
        ],
    );
}

/// Restoration over a real Unix socket to a reader child process.
fn deliver(args: &Args) {
    let mut term = term(args);
    let mut parser = Processor::<StdSyncHandler>::new();
    prefill(&mut term, &mut parser, args);
    let started = Instant::now();
    let bytes = restore(&mut term);
    let serialise = started.elapsed();
    let total = bytes.len();
    let path = text(args, "socket", "budgets.sock").to_owned();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("reader")
        .args([
            format!("socket={path}"),
            format!("reader={}", text(args, "reader", "drain")),
            format!("rate={}", arg(args, "rate", 0)),
            format!("cols={}", term.columns()),
            format!("rows={}", term.screen_lines()),
        ])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let (mut socket, _) = listener.accept().unwrap();
    socket.set_nonblocking(true).unwrap();
    let budget = DeliveryBudget {
        chunk_bytes: arg(args, "chunk", 65536),
        queue_bytes: 0,
        no_progress: Duration::from_secs(3600),
        restoration_deadline: Duration::from_secs(3600),
    };
    let wall_limit = Duration::from_millis(arg(args, "millis", 60_000) as u64);
    let started = Instant::now();
    let mut outbound = Outbound::restoring(budget, bytes, started);
    let (mut last_progress, mut worst_gap) = (started, Duration::ZERO);
    let (mut writes, mut sent) = (0u64, 0usize);
    while let Some(chunk) = outbound.next_chunk() {
        if started.elapsed() > wall_limit {
            break;
        }
        match socket.write(chunk) {
            Ok(written) => {
                let now = Instant::now();
                worst_gap = worst_gap.max(now - last_progress);
                last_progress = now;
                writes += 1;
                sent += written;
                outbound.wrote(written, now);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_micros(200));
            }
            Err(error) => panic!("{error}"),
        }
    }
    let elapsed = started.elapsed();
    drop(socket);
    let mut report = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut report)
        .unwrap();
    child.wait().unwrap();
    let _ = std::fs::remove_file(&path);
    emit(
        args,
        vec![
            ("restore_bytes", total.to_string()),
            ("sent_bytes", sent.to_string()),
            ("complete", (sent == total).to_string()),
            ("serialise_ms", millis(serialise)),
            ("deliver_seconds", secs(elapsed)),
            (
                "mib_per_s",
                format!("{:.3}", sent as f64 / 1048576.0 / elapsed.as_secs_f64()),
            ),
            ("worst_progress_gap_ms", millis(worst_gap)),
            ("writes", writes.to_string()),
            ("max_rss_bytes", max_rss_bytes().to_string()),
            ("reader_report", report.trim().to_owned()),
        ],
    );
}

/// Reader child: drains, rate-limits, or parses as a host-emulator proxy.
fn reader(args: &Args) {
    let mut socket = UnixStream::connect(text(args, "socket", "budgets.sock")).unwrap();
    let rate = arg(args, "rate", 0) as f64;
    let mut host = (text(args, "reader", "drain") == "host")
        .then(|| (term(args), Processor::<StdSyncHandler>::new()));
    let mut buffer = vec![0; 65536];
    let started = Instant::now();
    let (mut received, mut worst_read) = (0usize, Duration::ZERO);
    let mut last = started;
    loop {
        let read = socket.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        received += read;
        if let Some((term, parser)) = host.as_mut() {
            parser.advance(term, &buffer[..read]);
        }
        if rate > 0.0 {
            let due = started + Duration::from_secs_f64(received as f64 / rate);
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
        let now = Instant::now();
        worst_read = worst_read.max(now - last);
        last = now;
    }
    let elapsed = started.elapsed().as_secs_f64();
    println!(
        "{{\"received\":{received},\"seconds\":{elapsed:.4},\"mib_per_s\":{:.3},\"worst_read_gap_ms\":{:.3},\"max_rss_bytes\":{}}}",
        received as f64 / 1048576.0 / elapsed,
        worst_read.as_secs_f64() * 1e3,
        max_rss_bytes()
    );
}

fn main() {
    if let Ok(ceiling) = std::env::var("BUDGET_CEILING_BYTES") {
        CEILING.store(ceiling.parse().unwrap(), Relaxed);
    }
    let mut argv = std::env::args().skip(1);
    let command = argv.next().expect("command");
    let args: Args = argv
        .map(|pair| {
            let (key, value) = pair.split_once('=').expect("key=value");
            (key.to_owned(), value.to_owned())
        })
        .collect();
    match command.as_str() {
        "parse" => parse(&args),
        "storage" => storage(&args),
        "resize" => resize(&args),
        "sync" => sync(&args),
        "deliver" => deliver(&args),
        "reader" => reader(&args),
        "links" => followup_bench::links(&args),
        "snapshot" => followup_bench::snapshot(&args),
        "oracle" => followup_bench::oracle(&args),
        "event" => followup_bench::event(&args),
        "replay" => followup_bench::replay(&args),
        _ => panic!("unknown command {command}"),
    }
    std::io::stdout().flush().unwrap();
}
