"""Require unchanged workspace tests to pass under the mutation workers' load."""

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

FAILED_TEST = re.compile(r"^test (.+) \.\.\. FAILED$", re.MULTILINE)
FAILED_TARGET = re.compile(r"^error: test failed, to rerun pass `(.+)`$", re.MULTILINE)


def positive_integer(value):
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("must be at least one")
    return number


def test_workspace(root, cargo, target, log, environment, no_run=False):
    command = [
        cargo, "test", "--workspace", "--locked", "--no-fail-fast",
        "--target-dir", str(target),
    ]
    if no_run:
        command.append("--no-run")
    with log.open("w") as output:
        result = subprocess.run(
            command, cwd=root, env=environment, stdout=output,
            stderr=subprocess.STDOUT,
        )
        print(f"exit={result.returncode}", file=output)
    return result.returncode


def report_build_failure(log):
    print(f"Mutation load could not build the workspace: {log}", file=sys.stderr)
    print(log.read_text(errors="replace"), file=sys.stderr, flush=True)


def report_failures(logs):
    """Names each failing test and target with the number of suites it failed in."""
    tests = Counter()
    targets = Counter()
    for log in logs:
        text = log.read_text(errors="replace")
        tests.update(set(FAILED_TEST.findall(text)))
        targets.update(set(FAILED_TARGET.findall(text)))
    for title, failures in (("Failing tests", tests), ("Failing test targets", targets)):
        print(f"{title} (failed suites):", file=sys.stderr)
        for name, suites in sorted(failures.items()):
            print(f"  {suites:>3}  {name}", file=sys.stderr)
        if not failures:
            print("  none recognized; read the logs", file=sys.stderr)
    print("Failed suite logs:", file=sys.stderr)
    for log in logs:
        print(f"  {log}", file=sys.stderr, flush=True)


def remove(path):
    if path.exists():
        shutil.rmtree(path)


def measure(root, cargo, workers, rounds):
    target = root / "target"
    logs = target / "flake-logs"
    logs.mkdir(parents=True, exist_ok=True)
    logs = Path(tempfile.mkdtemp(prefix="run-", dir=logs))
    print(f"Mutation load: {workers} workers, {rounds} rounds; logs: {logs}", flush=True)
    environment = os.environ.copy()
    for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "CARGO_BUILD_BUILD_DIR"):
        environment.pop(key, None)
    warm = target / "mutation-tests"
    warm_log = logs / "warm.log"
    if test_workspace(root, cargo, warm, warm_log, environment, no_run=True):
        report_build_failure(warm_log)
        return 1
    scratch = target / "mutation-load"
    # A killed measurement cannot clean up after itself, so never let its builds accumulate.
    remove(scratch)
    try:
        scratch.mkdir()
        builds = [scratch / f"worker-{worker}" for worker in range(1, workers + 1)]
        options = ["-cRp"] if sys.platform == "darwin" else ["-a", "--reflink=auto"]
        for build in builds:
            subprocess.run(["cp", *options, str(warm), str(build)], check=True)
        with ThreadPoolExecutor(max_workers=workers) as pool:
            for round_number in range(1, rounds + 1):
                started = time.monotonic()
                run_logs = [logs / f"r{round_number}-{build.name}.log" for build in builds]
                results = list(pool.map(
                    lambda build, log: test_workspace(
                        root, cargo, build, log, environment,
                    ),
                    builds, run_logs,
                ))
                failed = [log for log, status in zip(run_logs, results) if status]
                print(
                    f"Mutation load round {round_number}/{rounds}: "
                    f"{len(failed)}/{workers} suites failed "
                    f"in {time.monotonic() - started:.1f}s",
                    flush=True,
                )
                if failed:
                    report_failures(failed)
                    return 1
    finally:
        remove(scratch)
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("cargo")
    parser.add_argument("workers", type=positive_integer)
    parser.add_argument("rounds", type=positive_integer)
    args = parser.parse_args()
    return measure(args.root.resolve(), args.cargo, args.workers, args.rounds)


if __name__ == "__main__":
    raise SystemExit(main())
