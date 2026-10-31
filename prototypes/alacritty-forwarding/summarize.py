#!/usr/bin/env python3
"""THROWAWAY #48: condense measurements/raw.jsonl into summary tables."""

import json
import sys

R = [json.loads(line) for line in open(sys.argv[1])]


def table(title, command, keys, where=lambda r: True):
    print(f"\n== {title}")
    print(" ".join(keys))
    for r in R:
        if r["command"] == command and where(r):
            print(" ".join(str(r.get(k, "-")) for k in keys))


def mib(r, key):
    value = r.get(key)
    return "-" if value is None else f"{int(value) / 1048576:.1f}MiB"


for r in R:
    for key in ["engine_filled_bytes", "grid_clone_bytes", "restore_bytes",
                "restore_heap_peak_bytes", "max_rss_bytes", "heap_growth_bytes",
                "heap_peak_growth_bytes"]:
        if key in r:
            r[key + "_m"] = mib(r, key)

table("storage (engine heap after fill; primary grid clone; materialized restore)",
      "storage", ["fill", "link_bytes", "cols", "rows", "history", "outcome",
                  "engine_filled_bytes_m", "bytes_per_retained_cell",
                  "grid_clone_bytes_m", "grid_clone_ms", "restore_bytes_m",
                  "restore_ms", "restore_heap_peak_bytes_m", "max_rss_bytes_m"])
table("resize (width -1 then back, worst reflow)", "resize",
      ["fill", "cols", "rows", "history", "outcome", "worst_resize_ms"])
table("sync flush (one stop_sync of N buffered bytes)", "sync",
      ["stream", "cols", "rows", "buffered_bytes", "outcome", "flush_ms"])
table("control strings and retained OSC state", "parse",
      ["stream", "link_bytes", "osc_cap", "history", "fed_bytes", "outcome",
       "heap_growth_bytes_m", "heap_peak_growth_bytes_m", "breaches", "mib_per_s",
       "max_rss_bytes_m"],
      lambda r: r.get("stream") in {"osc_body", "osc8_cell", "title_stack"}
      or (r.get("stream") in {"dcs_body", "apc_body"} and "cols" not in r))
table("realistic throughput by engine feed slice", "parse",
      ["stream", "cols", "rows", "history", "chunk", "fed_bytes", "outcome", "mib_per_s",
       "ns_per_byte", "max_chunk_ms", "max_rss_bytes_m"],
      lambda r: r.get("stream") in {"log", "tui", "unicode", "sgr_cells"})
table("adversarial work per engine feed slice (history 10k)", "parse",
      ["stream", "cols", "rows", "chunk", "fed_bytes", "outcome", "ns_per_byte",
       "max_chunk_ms"],
      lambda r: r.get("stream") in {"rep", "rep_insert", "decaln", "ich", "il_dl",
                                    "scroll", "clear", "combining"}
      or (r.get("stream") in {"dcs_body", "apc_body"} and "cols" in r))
for r in R:
    if r["command"] == "deliver" and r.get("outcome") == "ok":
        report = r["reader_report"]
        r["reader_mib_per_s"] = report["mib_per_s"]
        r["reader_rss_m"] = f"{report['max_rss_bytes'] / 1048576:.1f}MiB"
table("restoration delivery over a Unix socket (200x50)", "deliver",
      ["fill", "history", "reader", "rate", "chunk", "outcome", "restore_bytes_m",
       "complete", "serialise_ms", "deliver_seconds", "mib_per_s",
       "worst_progress_gap_ms", "reader_mib_per_s", "max_rss_bytes_m",
       "reader_rss_m"])
print("\n== skipped / failed")
for r in R:
    if r["outcome"] != "ok":
        print(r["command"], {k: v for k, v in r.items()
                             if k not in {"command", "stderr"}})
