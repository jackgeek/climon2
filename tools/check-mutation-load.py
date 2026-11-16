"""Require unchanged workspace tests to pass under the mutation workers' load.

The pinned mutator counts a mutant as killed when its test run fails, or outlives
`timeout factor x the unloaded baseline run` (never less than two seconds), and it
starts every test run in its own session (mutator `engine._apply_selected` and
`runner.CommandRunner.run`). Within that run, `test-mutation.py` runs the mutant's
stages in order, and an earlier stage also kills the mutant when it fails or outlives
its own stage timeout (STAGE_TIMEOUTS, which `test-mutation.py` measured unloaded).
Each suite here rehearses that run on the unchanged workspace, in a session of its own:
the earlier stages in order, each under its stage timeout, then the full workspace, all
within the mutator's timeout. Any stage that fails or outlives its timeout here could
have hidden a surviving mutant.
"""

import argparse
from collections import Counter
from contextlib import suppress
import json
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
POLL_SECONDS = 0.05
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


def load_stages(path):
    """The earlier stages, in the order workers run them, as `(arguments, stage timeout)`."""
    return [
        (selection.split(), float(timeout))
        for selection, timeout in json.loads(path.read_text()).items()
    ]


class Suite:
    """One worker's rehearsal of a mutant's run: its stages in order, each in a session of
    its own, logged to one file."""

    def __init__(self, root, cargo, environment, target, log, stages, timeout):
        self.command = [
            cargo, "test", "--workspace", "--locked", "--no-fail-fast",
            "--target-dir", str(target),
        ]
        self.root = root
        self.environment = environment
        self.log = log
        self.stages = iter(stages)
        self.started = time.monotonic()
        self.timeout = timeout
        self.process = None
        self.limit = None
        self.killed = None
        self.output = log.open("w")

    def advance(self):
        """Starts the next stage; False when none remains."""
        stage = next(self.stages, None)
        if stage is None:
            return False
        arguments, stage_timeout = stage
        self.limit = self.deadline(" ".join(arguments), stage_timeout)
        print(f"== cargo test {' '.join(arguments)}", file=self.output, flush=True)
        self.process = subprocess.Popen(
            self.command + arguments, cwd=self.root, env=self.environment,
            stdout=self.output, stderr=subprocess.STDOUT, start_new_session=True,
        )
        return True

    def deadline(self, name, stage_timeout):
        """The earlier of the stage's and the whole run's deadlines, with which one it is."""
        limits = []
        if stage_timeout is not None:
            limits.append(
                (time.monotonic() + stage_timeout, f"the {stage_timeout:.1f}s `{name}` stage timeout")
            )
        if self.timeout is not None:
            limits.append(
                (self.started + self.timeout, f"the {self.timeout:.1f}s mutation timeout")
            )
        return min(limits, default=None)

    def finished(self):
        """Whether the suite is done: passed, failed, or killed at a timeout."""
        code = self.process.poll()
        if code is None:
            if self.limit is not None and time.monotonic() >= self.limit[0]:
                self.killed = self.limit[1]
                self.stop()
                return True
            return False
        print(f"exit={code}", file=self.output, flush=True)
        return code != 0 or not self.advance()

    def failed(self):
        return self.process.returncode != 0

    def stop(self):
        with suppress(ProcessLookupError):
            os.killpg(self.process.pid, signal.SIGKILL)
        self.process.wait()
        print(f"Killed at {self.killed}", file=self.output)
        print(f"exit={self.process.returncode}", file=self.output, flush=True)

    def close(self):
        if self.process is not None and self.process.poll() is None:
            self.killed = "an interruption"
            self.stop()
        self.output.close()


def run_suites(root, cargo, environment, suites, stages, timeout=None):
    """Runs `(target, log)` suites at once. Each runs `stages`, `(arguments, stage timeout)`
    pairs with None for no stage timeout, in order, stopping at the first that fails or
    outlives its stage timeout or the suite's `timeout` seconds.

    Returns the logs of failed suites, and for those killed at a timeout, which one.
    Suites still running when the measurement is interrupted are killed too.
    """
    started = []
    try:
        for target, log in suites:
            suite = Suite(root, cargo, environment, target, log, stages, timeout)
            started.append(suite)
            suite.advance()
        running = list(started)
        while running:
            running = [suite for suite in running if not suite.finished()]
            time.sleep(POLL_SECONDS)
    finally:
        for suite in started:
            suite.close()
    failed = [suite.log for suite in started if suite.failed()]
    killed = {suite.log: suite.killed for suite in started if suite.killed}
    return failed, killed


def report_build_failure(log):
    print(f"Mutation load could not build the workspace: {log}", file=sys.stderr)
    print(log.read_text(errors="replace"), file=sys.stderr, flush=True)


def stuck_tests(text):
    """Tests that libtest reported running over 60 seconds and that never finished."""
    return set(SLOW_TEST.findall(text)) - set(FINISHED_TEST.findall(text))


def report_failures(failed, killed):
    """Names each failing test and target, and each test stuck in a suite killed at a
    timeout, with the number of suites it happened in."""
    summary = (
        ("Failing tests", failed, lambda text: set(FAILED_TEST.findall(text))),
        ("Failing test targets", failed, lambda text: set(FAILED_TARGET.findall(text))),
        ("Tests running over 60 seconds when killed", killed, stuck_tests),
    )
    for title, logs, names_in in summary:
        names = Counter(
            name for log in logs for name in names_in(log.read_text(errors="replace"))
        )
        if names:
            print(f"{title} (failed suites):", file=sys.stderr)
            for name, suites in sorted(names.items()):
                print(f"  {suites:>3}  {name}", file=sys.stderr)
    if killed:
        print("Suites killed at a timeout:", file=sys.stderr)
        for log, timeout in killed.items():
            print(f"  {log}: {timeout}", file=sys.stderr)
    print("Failed suite logs:", file=sys.stderr)
    for log in failed:
        print(f"  {log}", file=sys.stderr, flush=True)


def remove(path):
    if path.exists():
        shutil.rmtree(path)


def measure(root, cargo, workers, rounds, timeout_factor, stages):
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
        root, cargo, environment, [(warm, logs / "warm.log")], [(["--no-run"], None)],
    )
    if failed:
        report_build_failure(failed[0])
        return 1
    # Like the mutator's baseline: unloaded, in the warm build the workers copy.
    started = time.monotonic()
    failed, _ = run_suites(
        root, cargo, environment, [(warm, logs / "baseline.log")], [([], None)],
    )
    timeout = max(MINIMUM_TIMEOUT, timeout_factor * (time.monotonic() - started))
    if failed:
        report_failures(failed, {})
        return 1
    print(f"Unloaded baseline passed; the mutation timeout is {timeout:.1f}s.", flush=True)
    for arguments, stage_timeout in stages:
        print(f"Stage `{' '.join(arguments)}` timeout: {stage_timeout:.1f}s.", flush=True)
    try:
        scratch.mkdir()
        builds = [scratch / f"worker-{worker}" for worker in range(1, workers + 1)]
        for build in builds:
            subprocess.run(["cp", *COPY, str(warm), str(build)], check=True)
        for round_number in range(1, rounds + 1):
            started = time.monotonic()
            suites = [(build, logs / f"r{round_number}-{build.name}.log") for build in builds]
            failed, killed = run_suites(
                root, cargo, environment, suites, stages + [([], None)], timeout,
            )
            print(
                f"Mutation load round {round_number}/{rounds}: "
                f"{len(failed)}/{workers} suites failed "
                f"in {time.monotonic() - started:.1f}s",
                flush=True,
            )
            if failed:
                report_failures(failed, killed)
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
    parser.add_argument("--stage-timeouts", type=Path, required=True)
    args = parser.parse_args()
    return measure(
        args.root.resolve(), args.cargo, args.workers, args.rounds, args.timeout_factor,
        load_stages(args.stage_timeouts),
    )


if __name__ == "__main__":
    raise SystemExit(main())
