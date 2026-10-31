#!/usr/bin/env python3
"""THROWAWAY #48: run budgets measurements one process at a time, bounded.

Each run gets BUDGET_CEILING_BYTES (allocator ceiling, aborts on breach), an
RSS watchdog over the process group, and a wall timeout. Inputs grow stepwise;
runs whose estimated memory exceeds the ceiling are recorded as skipped.
Usage: measure.py OUT.jsonl [section ...]
"""

import json
import os
import signal
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.join(HERE, ".cache", "target", "release", "budgets")
CEILING = 2 << 30
RSS_CEILING = 3 << 30
WALL = 180
SOCKET = "budgets.sock"  # relative: macOS sun_path is 104 bytes
GEOMETRIES = [(80, 24), (200, 50), (500, 200), (1000, 300)]
HISTORIES = [0, 10_000, 100_000]
OUT = None


def record(row):
    OUT.write(json.dumps(row) + "\n")
    OUT.flush()
    print(json.dumps(row)[:400], flush=True)


def group_rss(pid):
    rows = subprocess.run(
        ["ps", "-A", "-o", "pid=,pgid=,rss="], capture_output=True, text=True
    ).stdout.split()
    total = 0
    for i in range(0, len(rows), 3):
        if int(rows[i + 1]) == pid:
            total += int(rows[i + 2]) * 1024
    return total


def run(command, wall=WALL, **args):
    argv = [BIN, command] + [f"{k}={v}" for k, v in args.items()]
    env = dict(os.environ, BUDGET_CEILING_BYTES=str(CEILING))
    started = time.monotonic()
    child = subprocess.Popen(
        argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env,
        start_new_session=True, cwd=HERE,
    )
    peak, outcome = 0, None
    while child.poll() is None:
        rss = group_rss(child.pid)
        peak = max(peak, rss)
        if rss > RSS_CEILING:
            outcome = "killed_rss"
        elif time.monotonic() - started > wall:
            outcome = "killed_wall"
        if outcome:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait()
            break
        time.sleep(0.05)
    stdout, stderr = child.communicate()
    meta = {"command": command, "watchdog_peak_rss": peak,
            "wall_seconds": round(time.monotonic() - started, 3)}
    if outcome or child.returncode != 0:
        row = dict(args, **meta, outcome=outcome or f"exit_{child.returncode}",
                   stderr=stderr.decode()[-300:])
        record(row)
        return None
    row = dict(json.loads(stdout.decode().strip().splitlines()[-1]), **meta,
               outcome="ok")
    record(row)
    return row


def skip(command, reason, **args):
    record(dict(args, command=command, outcome="skipped", reason=reason))


def cells(cols, rows, history):
    return (history + rows) * cols


def storage():
    for fill in ["log", "sgr_cells"]:
        for cols, rows in GEOMETRIES:
            for history in HISTORIES:
                estimate = cells(cols, rows, history) * (24 * 2 + 30) * 1.1
                if estimate > CEILING:
                    skip("storage", f"estimate {int(estimate)} > ceiling",
                         fill=fill, cols=cols, rows=rows, history=history)
                    continue
                run("storage", fill=fill, cols=cols, rows=rows, history=history)
    for fill in ["combining", "osc8_cell"]:
        for link in ([16, 256, 4096, 65536] if fill == "osc8_cell" else [0]):
            for cols, rows in GEOMETRIES[:2]:
                for history in HISTORIES[:2]:
                    estimate = cells(cols, rows, history) * (24 * 2 + 56 * 2 + 64 + 2 * link) * 1.2
                    if estimate > CEILING:
                        skip("storage", f"estimate {int(estimate)} > ceiling", fill=fill,
                             link_bytes=link, cols=cols, rows=rows, history=history)
                        continue
                    run("storage", fill=fill, link_bytes=link, cols=cols, rows=rows,
                        history=history)


def parse():
    realistic = ["log", "tui", "unicode", "sgr_cells"]
    for stream in realistic:
        for cols, rows in [(80, 24), (200, 50), (1000, 300)]:
            for history in HISTORIES:
                if cells(cols, rows, history) * 24 * 1.2 > CEILING:
                    skip("parse", "history estimate > ceiling", stream=stream,
                         cols=cols, rows=rows, history=history)
                    continue
                run("parse", stream=stream, cols=cols, rows=rows, history=history,
                    fill="none", bytes=32 << 20, chunk=65536, millis=5000)
    # Adversarial per-dispatch / per-slice work; grow slice until > 1 s.
    for stream in ["rep", "rep_insert", "decaln", "ich", "il_dl", "scroll", "clear",
                   "combining", "dcs_body", "apc_body"]:
        for cols, rows in GEOMETRIES:
            for chunk in [16, 64, 256, 1024, 4096, 65536]:
                row = run("parse", stream=stream, cols=cols, rows=rows, history=10_000,
                          fill="log", bytes=max(chunk * 8, 4096), chunk=chunk,
                          millis=3000, wall=60)
                if row is None or float(row["max_chunk_ms"]) > 1000:
                    break


def control_strings():
    for stream in ["osc_body", "dcs_body", "apc_body"]:
        for size in [64 << 10, 1 << 20, 16 << 20, 64 << 20, 256 << 20]:
            run("parse", stream=stream, bytes=size, chunk=65536, millis=60000)
            if stream == "osc_body":
                run("parse", stream=stream, bytes=size, chunk=65536, millis=60000,
                    osc_cap=4096)
    for link in [256, 4096, 65536]:
        for cap in [0, 4096]:
            run("parse", stream="osc8_cell", link_bytes=link, history=10_000,
                fill="none", bytes=min(256 << 20, (2000 + 10_000 * 80) * (link + 32)),
                chunk=65536, millis=60000, osc_cap=cap)
    for link in [1024, 16384, 65536, 262144]:
        for cap in [0, 4096]:
            if cap == 0 and 4200 * link * 2.2 > CEILING:
                skip("parse", "title stack estimate > ceiling", stream="title_stack",
                     link_bytes=link)
                continue
            run("parse", stream="title_stack", link_bytes=link, fill="none",
                bytes=4200 * (link + 24), chunk=65536, millis=60000, osc_cap=cap)


def resize():
    for fill in ["log", "sgr_cells"]:
        for cols, rows in GEOMETRIES:
            for history in HISTORIES:
                if cells(cols, rows, history) * 24 * 2.5 > CEILING:
                    skip("resize", "estimate > ceiling", fill=fill, cols=cols,
                         rows=rows, history=history)
                    continue
                run("resize", fill=fill, cols=cols, rows=rows, history=history,
                    to_cols=cols - 1)


def sync():
    for stream in ["log", "sgr_cells", "decaln", "rep", "rep_insert"]:
        for cols, rows in [(80, 24), (200, 50), (1000, 300)]:
            for size in [4096, 16384, 65536, 262144, (2 << 20) - 64]:
                row = run("sync", stream=stream, cols=cols, rows=rows, fill="none",
                          bytes=size, wall=120)
                if row is None or float(row["flush_ms"]) > 2000:
                    break


def deliver():
    for fill in ["log", "sgr_cells"]:
        for history in HISTORIES:
            cols, rows = 200, 50
            if cells(cols, rows, history) * (24 + 30) * 1.3 > CEILING:
                skip("deliver", "estimate > ceiling", fill=fill, history=history)
                continue
            common = dict(fill=fill, cols=cols, rows=rows, history=history,
                          socket=SOCKET)
            for chunk in [4096, 16384, 65536, 262144]:
                run("deliver", chunk=chunk, reader="drain", **common)
            run("deliver", chunk=65536, reader="host", **common)
            for rate in [1 << 20, 256 << 10]:
                run("deliver", chunk=65536, reader="rate", rate=rate, millis=60000,
                    wall=120, **common)


SECTIONS = {"storage": storage, "parse": parse, "control": control_strings,
            "resize": resize, "sync": sync, "deliver": deliver}

if __name__ == "__main__":
    with open(sys.argv[1], "a") as OUT:
        for name in sys.argv[2:] or list(SECTIONS):
            SECTIONS[name]()
