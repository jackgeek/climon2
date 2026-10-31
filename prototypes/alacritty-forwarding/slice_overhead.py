#!/usr/bin/env python3
"""THROWAWAY #48: realistic-stream overhead of small engine feed slices."""
import sys
sys.argv = [sys.argv[0], sys.argv[1]]
import measure
with open(sys.argv[1], "a") as out:
    measure.OUT = out
    for stream in ["log", "tui", "unicode", "sgr_cells"]:
        for chunk in [16, 64, 256, 4096, 65536]:
            measure.run("parse", stream=stream, cols=200, rows=50, history=10_000,
                        fill="none", bytes=16 << 20, chunk=chunk, millis=10000)
