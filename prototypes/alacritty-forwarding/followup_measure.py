#!/usr/bin/env python3
"""THROWAWAY qualification, one bounded measurement at a time.

From this directory: python3 followup_measure.py SECTION [...]
Sections: smoke, links, oracle, snapshot, event, capacity, replay.
Raw runs append to followup-measurements/raw.jsonl. Stop on any failed process.
"""
import json
import os
import sys
from bounded import run

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.join(HERE, ".cache/target/release/budgets")
OUT = os.path.join(HERE, "followup-measurements")
GEOMETRIES = [(80, 24), (200, 50), (500, 200)]


def measure(command, **args):
    large = args.get("cols") == 500 and args.get("history", 0) == 100_000
    # Max SGR: ~1.2 GB engine + 1.2 GB grid clone + <=2.15 GB output capacity.
    # 6 GiB heap / 8 GiB RSS admits it with margin on this 64 GB host.
    ceiling, rss = (6 << 30, 8 << 30) if large else (2 << 30, 3 << 30)
    argv = [BIN, command] + [f"{key}={value}" for key, value in args.items()]
    with open(os.path.join(OUT, "last.stdout"), "w") as stdout, \
         open(os.path.join(OUT, "last.stderr"), "w") as stderr:
        result = run(argv, wall=600, ceiling=ceiling, rss_ceiling=rss,
                     cwd=HERE, stdout=stdout, stderr=stderr)
    result.update(command=command, heap_ceiling_bytes=ceiling, rss_ceiling_bytes=rss)
    if result["returncode"] == 0:
        with open(os.path.join(OUT, "last.stdout")) as output:
            result.update(json.loads(output.read().strip().splitlines()[-1]))
    else:
        result.update(args)
        with open(os.path.join(OUT, "last.stderr")) as errors:
            result["stderr"] = errors.read()[-2000:]
    with open(os.path.join(OUT, "raw.jsonl"), "a") as raw:
        raw.write(json.dumps(result) + "\n")
    print(json.dumps(result), flush=True)
    if result["outcome"] != "exit_0":
        raise SystemExit("FAILED measurement; raw evidence retained")


def smoke():
    for kind in ["log", "sgr_cells"]:
        measure("oracle", cols=80, rows=24, history=50, fill=kind)
        for strategy in ["blocking", "clone", "sliced"]:
            measure("snapshot", cols=80, rows=24, history=50, fill=kind,
                    strategy=strategy, rate=4 << 20, slice_rows=16)
    measure("event", cols=80, rows=24, history=100, fill="sgr_cells", chunk=4096)
    for kind in ["single", "repeated", "unique", "interned"]:
        measure("links", cols=80, rows=24, history=200, cells=10_000,
                kind=kind, link_bytes=4080)


def links():
    for length in [16, 256, 4080]:
        for cells in [2000, 10_000, 100_000]:
            for kind in ["single", "repeated", "unique", "interned"]:
                measure("links", cols=80, rows=24, history=2000, cells=cells,
                        kind=kind, link_bytes=length)

def capacity():
    for cells in [2000, 10_000, 100_000]:
        measure("links", cols=80, rows=24, history=2000, cells=cells,
                kind="unique", link_bytes=4093, no_id=1, uri_segments=1)

def replay():
    for repeat in range(3):
        measure("replay", cols=500, rows=200, history=100_000, fill="sgr_cells",
                replay_bytes=16 << 20, repeat=repeat)


def oracle():
    for cols, rows in GEOMETRIES:
        for history in [0, 10_000, 100_000]:
            for fill in ["log", "sgr_cells"]:
                measure("oracle", cols=cols, rows=rows, history=history, fill=fill)


def snapshot():
    for cols, rows in GEOMETRIES:
        for history in [0, 10_000, 100_000]:
            for fill in ["log", "sgr_cells"]:
                for strategy in ["blocking", "clone", "sliced"]:
                    for repeat in range(3):
                        measure("snapshot", cols=cols, rows=rows, history=history,
                                fill=fill, strategy=strategy, rate=4 << 20,
                                slice_rows=16, repeat=repeat)
    for strategy in ["clone", "sliced"]:
        for repeat in range(3):
            measure("snapshot", cols=500, rows=200, history=100_000, fill="sgr_cells",
                    strategy=strategy, rate=32 << 20, slice_rows=16, repeat=repeat)


def event():
    for cols, rows, history in [(200, 50, 0), (200, 50, 10_000),
                                (200, 50, 100_000), (500, 200, 100_000)]:
        for fill in ["log", "sgr_cells"]:
            for chunk in [4096, 16384]:
                for repeat in range(3):
                    measure("event", cols=cols, rows=rows, history=history,
                            fill=fill, chunk=chunk, repeat=repeat)
    for repeat in range(3):
        measure("event", cols=200, rows=50, history=10_000, fill="sgr_cells",
                chunk=4096, rate=4 << 20, repeat=repeat)


if __name__ == "__main__":
    os.makedirs(OUT, exist_ok=True)
    for section in sys.argv[1:] or ["smoke"]:
        globals()[section]()
