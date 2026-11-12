"""Require unchanged workspace tests to pass under the mutation workers' load.

The pinned mutator counts a mutant as killed when its test run fails, or outlives
`timeout factor x the unloaded baseline run` (never less than two seconds), and it
starts every test run in its own session (mutator `engine._apply_selected` and
`runner.CommandRunner.run`). Each suite here runs the same way, so any suite that
fails or outlives that timeout could have hidden a surviving mutant.
"""

import argparse
from collections import Counter
from contextlib import suppress
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time

MINIMUM_TIMEOUT = 2.0
FAILED_TEST = re.compile(r"^test (.+) \.\.\. FAILED$", re.MULTILINE)
FAILED_TARGET = re.compile(r"^error: test failed, to rerun pass `(.+)`$", re.MULTILINE)
SLOW_TEST = re.compile(r"^test (.+) has been running for over 60 seconds$", re.MULTILINE)
FINISHED_TEST = re.compile(r"^test (.+) \.\.\. ", re.MULTILINE)
COPY = ["-cRp"] if sys.platform == "darwin" else ["-a", "--reflink=auto"]


def positive_integer(value):
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("must be at least one")
    return number


def positive_number(value):
    number = float(value)
    if not number > 0:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def start(root, cargo, environment, target, log, no_run):
    command = [
        cargo, "test", "--workspace", "--locked", "--no-fail-fast",
        "--target-dir", str(target),
    ]
    if no_run:
        command.append("--no-run")
    output = log.open("w")
    try:
        process = subprocess.Popen(
            command, cwd=root, env=environment, stdout=output,
            stderr=subprocess.STDOUT, start_new_session=True,
        )
    except BaseException:
        output.close()
        raise
    return log, output, process


def run_suites(root, cargo, environment, suites, timeout=None, no_run=False):
    """Runs `(target, log)` suites at once, each in its own session.

    Returns the logs of failed suites and of those killed at `timeout` seconds.
    Suites still running when the measurement is interrupted are killed too.
    """
    started = []
    timed_out = []
    try:
        for target, log in suites:
            started.append(start(root, cargo, environment, target, log, no_run))
        deadline = None if timeout is None else time.monotonic() + timeout
        for _, _, process in started:
            remaining = None if deadline is None else max(0.0, deadline - time.monotonic())
            with suppress(subprocess.TimeoutExpired):
                process.wait(remaining)
        timed_out = [log for log, _, process in started if process.poll() is None]
    finally:
        for log, output, process in started:
            if process.poll() is None:
                with suppress(ProcessLookupError):
                    os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            if log in timed_out:
                print(f"Killed at the {timeout:.1f}s mutation timeout", file=output)
            print(f"exit={process.returncode}", file=output)
            output.close()
    failed = [log for log, _, process in started if process.returncode != 0]
    return failed, timed_out


def report_build_failure(log):
    print(f"Mutation load could not build the workspace: {log}", file=sys.stderr)
    print(log.read_text(errors="replace"), file=sys.stderr, flush=True)


def stuck_tests(text):
    """Tests that libtest reported running over 60 seconds and that never finished."""
    return set(SLOW_TEST.findall(text)) - set(FINISHED_TEST.findall(text))


def report_failures(failed, timed_out, timeout):
    """Names each failing test and target, and each test stuck in a suite killed at the
    timeout, with the number of suites it happened in."""
    summary = (
        ("Failing tests", failed, lambda text: set(FAILED_TEST.findall(text))),
        ("Failing test targets", failed, lambda text: set(FAILED_TARGET.findall(text))),
        ("Tests running over 60 seconds when killed", timed_out, stuck_tests),
    )
    for title, logs, names_in in summary:
        names = Counter(
            name for log in logs for name in names_in(log.read_text(errors="replace"))
        )
        if names:
            print(f"{title} (failed suites):", file=sys.stderr)
            for name, suites in sorted(names.items()):
                print(f"  {suites:>3}  {name}", file=sys.stderr)
    if timed_out:
        print(f"Suites killed at the {timeout:.1f}s mutation timeout:", file=sys.stderr)
        for log in timed_out:
            print(f"  {log}", file=sys.stderr)
    print("Failed suite logs:", file=sys.stderr)
    for log in failed:
        print(f"  {log}", file=sys.stderr, flush=True)


def remove(path):
    if path.exists():
        shutil.rmtree(path)


def measure(root, cargo, workers, rounds, timeout_factor):
    target = root / "target"
    scratch = target / "mutation-load"
    # A killed measurement cannot clean up after itself, so never let its builds accumulate.
    remove(scratch)
    history = target / "flake-logs"
    history.mkdir(parents=True, exist_ok=True)
    logs = Path(tempfile.mkdtemp(prefix="run-", dir=history))
    print(f"Mutation load: {workers} workers, {rounds} rounds; logs: {logs}", flush=True)
    environment = os.environ.copy()
    for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "CARGO_BUILD_BUILD_DIR"):
        environment.pop(key, None)
    warm = target / "mutation-tests"
    failed, _ = run_suites(
        root, cargo, environment, [(warm, logs / "warm.log")], no_run=True,
    )
    if failed:
        report_build_failure(failed[0])
        return 1
    # Like the mutator's baseline: unloaded, in the warm build the workers copy.
    started = time.monotonic()
    failed, _ = run_suites(root, cargo, environment, [(warm, logs / "baseline.log")])
    timeout = max(MINIMUM_TIMEOUT, timeout_factor * (time.monotonic() - started))
    if failed:
        report_failures(failed, [], timeout)
        return 1
    print(f"Unloaded baseline passed; the mutation timeout is {timeout:.1f}s.", flush=True)
    try:
        scratch.mkdir()
        builds = [scratch / f"worker-{worker}" for worker in range(1, workers + 1)]
        for build in builds:
            subprocess.run(["cp", *COPY, str(warm), str(build)], check=True)
        for round_number in range(1, rounds + 1):
            started = time.monotonic()
            suites = [(build, logs / f"r{round_number}-{build.name}.log") for build in builds]
            failed, timed_out = run_suites(root, cargo, environment, suites, timeout)
            print(
                f"Mutation load round {round_number}/{rounds}: "
                f"{len(failed)}/{workers} suites failed "
                f"in {time.monotonic() - started:.1f}s",
                flush=True,
            )
            if failed:
                report_failures(failed, timed_out, timeout)
                return 1
    finally:
        remove(scratch)
    return 0


def terminate(signum, _frame):
    # Unwinds through the cleanup that kills running suites and removes worker builds.
    raise SystemExit(128 + signum)


def main():
    for signum in (signal.SIGTERM, signal.SIGHUP):
        # An inherited SIG_IGN (as `nohup` sets) stays: that caller asked to keep running.
        if signal.getsignal(signum) is not signal.SIG_IGN:
            signal.signal(signum, terminate)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("cargo")
    parser.add_argument("--workers", type=positive_integer, required=True)
    parser.add_argument("--rounds", type=positive_integer, required=True)
    parser.add_argument("--timeout-factor", type=positive_number, required=True)
    args = parser.parse_args()
    return measure(
        args.root.resolve(), args.cargo, args.workers, args.rounds, args.timeout_factor,
    )


if __name__ == "__main__":
    raise SystemExit(main())
