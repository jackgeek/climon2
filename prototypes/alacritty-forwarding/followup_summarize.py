#!/usr/bin/env python3
"""THROWAWAY: summarize raw evidence; reject unmatched restore hashes."""
import collections
import json
import statistics

rows = [json.loads(line) for line in open("followup-measurements/raw.jsonl")]
ok = [r for r in rows if r["outcome"] == "exit_0"]
oracles = {(r["cols"], r["rows"], r["history"], r["fill"]): r
           for r in ok if r["command"] == "oracle"}
for r in ok:
    if r["command"] == "snapshot":
        key = (r["cols"], r["rows"], r["history"], r["fill"])
        # Earlier, preserved smoke log predates the fixture's explicit CUP.
        if "repeat" in r:
            assert key in oracles, ("missing independent oracle", key)
            assert r["byte_hash"] == oracles[key]["byte_hash"], ("snapshot changed", r)
            assert r["restore_bytes"] == oracles[key]["restore_bytes"]


def med(group, key):
    return statistics.median(float(r[key]) for r in group)


def high(group, key):
    return max(float(r[key]) for r in group)


print("THROWAWAY #48/#43; full raw records retained, including initial smoke failure.")
print("Processes:", len(rows), "successful:", len(ok), "failed:", len(rows) - len(ok))
print("Matrix failures:", sum(r["outcome"] != "exit_0" and "repeat" in r for r in rows))
print("RSS = process ru_maxrss, includes setup/measurement/cleanup; watchdog sampled independently.")
print("MiB = 2^20 B. Callback p50 is median of per-run medians (three repeats).")
print("Critical p50 is median of the worst callback per restore, not a percentile over idle loops.")
print("All repeated snapshot byte hashes must match their all-row independent oracle.\n")

print("HYPERLINKS: requested live growth, MiB (includes grid allocation)")
print("URI bytes | cells | single OSC | repeated OSC | unique OSC | public intern seam")
for length in [16, 256, 4080]:
    for cells in [2000, 10_000, 100_000]:
        group = {(r["kind"]): r for r in ok if r["command"] == "links"
                 and str(r["link_bytes"]) == str(length) and str(r["cells"]) == str(cells)}
        if len(group) == 4:
            print(length, cells, *[round(group[k]["retained_growth_bytes"] / (1 << 20), 3)
                                  for k in ["single", "repeated", "unique", "interned"]], sep=" | ")
print("\nURI CAPACITY ADVERSARY (4093 B URI in two semicolon-separated segments)")
print("cells | requested growth MiB | peak RSS MiB | released B | remaining fixture seed B")
for r in ok:
    if r["command"] == "links" and r.get("uri_segments") == "1":
        print(r["cells"], f"{r['retained_growth_bytes'] / (1 << 20):.3f}",
              f"{r['max_rss_bytes'] / (1 << 20):.3f}", r["released_bytes"],
              r["after_drop_growth_bytes"], sep=" | ")

print("\nPOST-SNAPSHOT PTY CATCH-UP: max geometry/history, 16 MiB in 64 B callbacks")
for r in ok:
    if r["command"] == "replay":
        print("repeat", r["repeat"], "p50 ms", r["pump_stall_p50_ms"],
              "max ms", r["pump_stall_max_ms"], "total s", r["replay_seconds"],
              "MiB/s", r["replay_mib_per_s"], r["oracle"])

print("\nIndependent all-row oracle coverage:")
for key, r in sorted(oracles.items(), key=lambda kv: (int(kv[0][0]), int(kv[0][2]), kv[0][3])):
    if r["history"] == "50":
        continue
    print(f"{r['cols']}x{r['rows']} history={r['history']} {r['fill']}: "
          f"{r['checked_rows']} rows, {r['restore_bytes']} B, text/style/cursor PASS, one reflow")

groups = collections.defaultdict(list)
for r in ok:
    if r["command"] == "snapshot" and "repeat" in r:
        groups[(int(r["cols"]), int(r["rows"]), int(r["history"]),
                r["fill"], r["strategy"], int(r["rate"]))].append(r)
print("\nSCHEDULING (4 MiB/s PTY unless specified)")
print("geometry | history | fill | strategy | PTY MiB/s | n | callback p50 ms | "
      "critical p50 ms | max stall ms | peak RSS MiB | median serialize MiB/s | "
      "max buffered PTY B | parsed during B | backpressure | B6 overflow")
for (cols, lines, history, fill, strategy, rate), group in sorted(groups.items()):
    print(f"{cols}x{lines}", history, fill, strategy, rate // (1 << 20), len(group),
          f"{med(group, 'pump_stall_p50_ms'):.3f}", f"{med(group, 'pump_stall_max_ms'):.3f}",
          f"{high(group, 'pump_stall_max_ms'):.3f}",
          f"{high(group, 'max_rss_bytes') / (1 << 20):.1f}",
          f"{med(group, 'mib_per_s'):.1f}", int(high(group, "pty_buffered_bytes")),
          int(high(group, "pty_parsed_during_bytes")),
          any(r["pty_backpressure"] for r in group),
          any(r["live_queue_would_overflow"] for r in group), sep=" | ")

groups = collections.defaultdict(list)
for r in ok:
    if r["command"] == "event" and "repeat" in r:
        groups[(int(r["cols"]), int(r["rows"]), int(r["history"]),
                r["fill"], int(r["chunk"]), int(r.get("rate", 0)))].append(r)
print("\nEVENT-DRIVEN UNIX SOCKET DELIVERY")
print("geometry | history | fill | chunk B | reader B/s (0=drain) | n | "
      "median MiB/s | max progress gap ms | peak RSS MiB | readiness waits range")
for (cols, lines, history, fill, chunk, rate), group in sorted(groups.items()):
    assert all(r["complete"] for r in group)
    print(f"{cols}x{lines}", history, fill, chunk, rate, len(group),
          f"{med(group, 'mib_per_s'):.1f}", f"{high(group, 'worst_progress_gap_ms'):.3f}",
          f"{high(group, 'max_rss_bytes') / (1 << 20):.1f}",
          f"{min(r['readiness_waits'] for r in group)}..{max(r['readiness_waits'] for r in group)}",
          sep=" | ")
