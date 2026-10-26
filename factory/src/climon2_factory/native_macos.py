"""The MacOS gate: acceptance tests run natively on this Mac against a fresh checkout."""

import hashlib
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

from climon2_factory.container_executor import CONTAINERS_DIR
from climon2_factory.ports import TestReport
from climon2_factory.reports import (
    MAX_REPORT_BYTES,
    build_failed_report,
    collected_tests,
    failed_report,
    parse_report,
)

LABEL = "macos-native"
FACTORY_HOME = Path.home() / ".climon2-factory"
RUST_VERSION = "1.96.0"
RUSTUP_INIT_URL = (
    "https://static.rust-lang.org/rustup/archive/1.29.2/x86_64-apple-darwin/rustup-init"
)
RUSTUP_INIT_SHA256 = "b9610730ddeef52765723847866fd1e3540b2bf7d5ab0c324aa143a5edf25ba0"
REQUIREMENTS = CONTAINERS_DIR / "requirements.txt"
_COMMIT = re.compile(r"[0-9a-f]{40}")


@dataclass(frozen=True)
class Toolchain:
    """A factory-owned, pinned Rust toolchain and pytest, kept apart from the user's own."""

    home: Path

    @property
    def rustup_home(self) -> Path:
        return self.home / "rustup"

    @property
    def cargo_home(self) -> Path:
        return self.home / "cargo"

    @property
    def cargo(self) -> Path:
        return self.cargo_home / "bin" / "cargo"

    @property
    def python(self) -> Path:
        return self.home / "pytest" / "bin" / "python"

    @classmethod
    def ensure(cls, home: Path = FACTORY_HOME / "toolchain") -> "Toolchain":
        """Install the toolchain into `home` unless this exact version is already there."""
        toolchain = cls(home)
        requirements = hashlib.sha256(REQUIREMENTS.read_bytes()).hexdigest()[:16]
        marker = home / f".complete-rust-{RUST_VERSION}-pytest-{requirements}"
        if marker.is_file():
            return toolchain
        home.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory() as tmp:
            init = Path(tmp) / "rustup-init"
            _check(["curl", "-sSfL", "--proto", "=https", "-o", str(init), RUSTUP_INIT_URL])
            if hashlib.sha256(init.read_bytes()).hexdigest() != RUSTUP_INIT_SHA256:
                raise ToolchainError("rustup-init does not match its pinned hash")
            init.chmod(0o755)
            env = {**toolchain.cargo_env(Path(tmp)), "RUSTUP_INIT_SKIP_PATH_CHECK": "yes"}
            _check(
                [
                    str(init),
                    "-y",
                    "--no-modify-path",
                    "--profile",
                    "minimal",
                    "--default-toolchain",
                    RUST_VERSION,
                ],
                env=env,
            )
        _check([sys.executable, "-m", "venv", "--clear", str(home / "pytest")])
        pip = [str(toolchain.python), "-m", "pip", "install", "-q", "--disable-pip-version-check"]
        _check([*pip, "--require-hashes", "--no-deps", "-r", str(REQUIREMENTS)])
        for old in home.glob(".complete-*"):
            old.unlink()
        marker.touch()
        return toolchain

    def cargo_env(self, home: Path) -> dict[str, str]:
        return {
            "PATH": f"{self.cargo_home / 'bin'}:/usr/bin:/bin:/usr/sbin:/sbin",
            "HOME": str(home),
            "RUSTUP_HOME": str(self.rustup_home),
            "CARGO_HOME": str(self.cargo_home),
            "RUSTUP_TOOLCHAIN": RUST_VERSION,
        }


class ToolchainError(RuntimeError):
    pass


class NativeMacOS:
    def __init__(
        self,
        source_repo: Path,
        toolchain: Toolchain,
        root: Path = FACTORY_HOME / "macos-runs",
        time_limit_s: float = 1800,
    ) -> None:
        self._source = source_repo
        self._toolchain = toolchain
        self._root = root
        self._time_limit_s = time_limit_s

    def run_acceptance(self, commit: str, features: tuple[str, ...]) -> TestReport:
        if not _COMMIT.fullmatch(commit):
            raise ValueError(f"not a full commit hash: {commit!r}")
        run = _Run(self._root / commit, time.monotonic() + self._time_limit_s)
        shutil.rmtree(run.dir, ignore_errors=True)
        try:
            return self._run(commit, features, run)
        except _TimedOut:
            reason = f"acceptance run timed out after {self._time_limit_s:g}s"
            return failed_report(run.expected, reason, LABEL)
        finally:
            _sweep(run.dir)
            shutil.rmtree(run.dir, ignore_errors=True)

    def _run(self, commit: str, features: Sequence[str], run: "_Run") -> TestReport:
        work, home, target = run.dir / "work", run.dir / "home", run.dir / "target"
        for d in (work, home, target):
            d.mkdir(parents=True)
        archive = _check(["git", "archive", "--format=tar", commit], cwd=self._source).stdout
        _check(["tar", "-xf", "-", "-C", str(work)], stdin=archive)
        test_dirs = [f"features/{f}/tests" for f in features]
        pytest = [str(self._toolchain.python), "-m", "pytest", "-p", "no:cacheprovider"]
        cargo = str(self._toolchain.cargo)
        env = {**self._toolchain.cargo_env(home), "CARGO_TARGET_DIR": str(target)}

        exit_code, output = run.step([*pytest, "--collect-only", "-q", *test_dirs], work, env)
        if exit_code not in (0, 5):
            reason = f"collection failed (exit {exit_code}):\n{_tail(output)}"
            return failed_report((), reason, LABEL, fallback="<collect>")
        run.expected = collected_tests(output.decode(errors="replace"))

        product = work / "climon2"
        exit_code, output = run.step([cargo, "fetch", "--locked"], product, env)
        if exit_code != 0:
            log = f"cargo fetch failed:\n{_tail(output)}"
            return build_failed_report(run.expected, log, LABEL)
        build = [cargo, "build", "--release", "--locked", "--offline"]
        exit_code, output = run.step(build, product, env)
        if exit_code != 0:
            return build_failed_report(run.expected, _tail(output), LABEL)

        report = run.dir / "report.xml"
        test_env = {**env, "CLIMON2_BIN_DIR": str(target / "release")}
        exit_code, _ = run.step([*pytest, f"--junitxml={report}", *test_dirs], work, test_env)
        xml = report.read_bytes()[: MAX_REPORT_BYTES + 1] if report.is_file() else b""
        return parse_report(xml, run.expected, exit_code, LABEL)


class _TimedOut(Exception):
    pass


class _Run:
    def __init__(self, directory: Path, deadline: float) -> None:
        self.dir = directory
        self.deadline = deadline
        self.expected: tuple[str, ...] = ()

    def step(self, args: Sequence[str], cwd: Path, env: dict[str, str]) -> tuple[int, bytes]:
        """Run in a new process group; the whole group is killed at the deadline."""
        process = subprocess.Popen(
            args,
            cwd=cwd,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            output, _ = process.communicate(timeout=max(0.0, self.deadline - time.monotonic()))
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.communicate()
            raise _TimedOut from None
        return process.returncode, output


def _sweep(directory: Path) -> None:
    """Kill this user's processes still working inside `directory`, such as detached children
    of the tests. A process that changed directory elsewhere escapes this (no VM yet)."""
    inside = str(directory.resolve())
    for _ in range(5):
        lsof = subprocess.run(
            ["lsof", "-a", "-d", "cwd", "-u", str(os.getuid()), "-Fn"],
            capture_output=True,
            text=True,
            check=False,
        ).stdout
        pids: list[int] = []
        pid = None
        for line in lsof.splitlines():
            if line.startswith("p"):
                pid = int(line[1:])
            elif line.startswith("n") and pid is not None:
                path = line[1:]
                if path == inside or path.startswith(inside + "/"):
                    pids.append(pid)
        if not pids:
            return
        for pid in pids:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        time.sleep(0.2)


def _tail(output: bytes) -> str:
    return output[-20_000:].decode(errors="replace")


def _check(
    args: Sequence[str],
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
    stdin: bytes | None = None,
) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run(args, cwd=cwd, env=env, input=stdin, capture_output=True, check=False)
    if result.returncode != 0:
        raise ToolchainError(f"{args[0]} failed: {_tail(result.stderr)}")
    return result
