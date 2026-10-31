#!/usr/bin/env python3
"""THROWAWAY: bounded serial command runner; never kill unrelated processes."""
import argparse
import json
import os
import signal
import subprocess
import time


def run(argv, wall=180, ceiling=2 << 30, rss_ceiling=3 << 30, cwd=None,
        stdout=None, stderr=None):
    env = dict(os.environ, BUDGET_CEILING_BYTES=str(ceiling),
               CARGO_BUILD_JOBS="1", RUST_TEST_THREADS="1")
    child = subprocess.Popen(argv, env=env, cwd=cwd, start_new_session=True,
                             stdout=stdout, stderr=stderr)
    started, peak, outcome = time.monotonic(), 0, "ok"
    while child.poll() is None:
        probe = subprocess.run(["ps", "-A", "-o", "pid=,ppid=,rss="],
                               check=True, capture_output=True, text=True)
        rows = [tuple(map(int, row.split())) for row in probe.stdout.splitlines()]
        family = {child.pid}
        while True:
            expanded = family | {pid for pid, parent, _ in rows if parent in family}
            if expanded == family:
                break
            family = expanded
        rss = sum(rss * 1024 for pid, _, rss in rows if pid in family)
        peak = max(peak, rss)
        if rss > rss_ceiling or time.monotonic() - started > wall:
            outcome = "killed_rss" if rss > rss_ceiling else "killed_wall"
            for pid in sorted(family, reverse=True):
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            break
        time.sleep(0.05)
    code = child.wait()
    return dict(outcome=outcome if outcome != "ok" else f"exit_{code}",
                watchdog_peak_rss=peak, wall_seconds=time.monotonic() - started,
                returncode=code)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--wall", type=int, default=180)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    result = run(args.command, wall=args.wall)
    print(json.dumps(result), flush=True)
    raise SystemExit(result["returncode"] or (result["outcome"] != "exit_0"))
