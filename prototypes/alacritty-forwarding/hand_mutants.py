#!/usr/bin/env python3
"""THROWAWAY #48: hand mutants of the budget seams; each must fail its tests.
One cargo job, one test thread, 300 s wall limit per mutant; source restored."""
import os, signal, subprocess, sys
from pathlib import Path

MUTANTS = [
    ("admission.rs", "_ if self.held.len() >= self.cap =>", "_ if self.held.len() > self.cap =>", "admission"),
    ("admission.rs", "BEL => out.push(CAN),", "BEL => {}", "admission"),
    ("admission.rs", "CAN | SUB => out.push(byte),", "CAN | SUB => {}", "admission"),
    ("geometry.rs", "columns > limits.max_columns", "columns > limits.max_columns + 1", "geometry"),
    ("geometry.rs", "rows > limits.max_rows", "rows >= limits.max_rows", "geometry"),
    ("delivery.rs", "self.live.len() + bytes.len() > self.budget.queue_bytes", "self.live.len() > self.budget.queue_bytes", "delivery"),
    ("delivery.rs", "now >= since + self.budget.no_progress", "now > since + self.budget.no_progress + self.budget.no_progress", "delivery"),
    ("delivery.rs", "self.restoration_until.is_some_and(|until| now >= until)", "false", "delivery"),
    ("feed.rs", "parser.sync_bytes_count() > self.sync_hold_bytes", "false", "feed"),
    ("feed.rs", "let end = (self.offset + self.slice_bytes).min(self.pending.len());", "let end = self.pending.len();", "feed"),
    ("feed.rs", "if flush_if_due(parser, handler, now) {", "if false {", "feed"),
]

env = dict(os.environ, CARGO_TARGET_DIR=str(Path(".cache/target").resolve()))
for file, old, new, test in MUTANTS:
    path = Path("probe/src") / file
    original = path.read_text()
    assert original.count(old) == 1, (file, old)
    path.write_text(original.replace(old, new))
    try:
        process = subprocess.Popen(
            ["cargo", "test", "-j1", "--quiet", "--locked", "--manifest-path",
             "probe/Cargo.toml", "--test", test, "--", "--test-threads=1"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env,
            start_new_session=True)
        try:
            code = process.wait(timeout=300)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            code = "timeout"
    finally:
        path.write_text(original)
    print(f"{'KILLED' if code != 0 else 'SURVIVED'} exit={code} {file}: {old!r} -> {new!r}", flush=True)
