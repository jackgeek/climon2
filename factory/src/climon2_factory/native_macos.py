"""The MacOS gate: acceptance tests run natively on this Mac against a fresh checkout."""

import hashlib
import os
import pwd
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from pathlib import Path

from climon2_factory.container_executor import CONTAINERS_DIR, workspace_tar
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

    @property
    def rust_bin(self) -> Path:
        """The pinned toolchain's own binaries, bypassing the rustup proxies."""
        [toolchain] = (self.rustup_home / "toolchains").glob(f"{RUST_VERSION}-*")
        return toolchain / "bin"

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

        def checkout(work: Path) -> None:
            archive = _check(["git", "archive", "--format=tar", commit], cwd=self._source).stdout
            _check(["tar", "-xf", "-", "-C", str(work)], stdin=archive)

        return self._guarded(self._root / commit, features, checkout, sandboxed=False)

    def run_workspace(self, workspace: Path, features: tuple[str, ...]) -> TestReport:
        """Test the workspace as it is. It holds agent-written code, so everything that runs
        that code is sandboxed: no network, no writes outside the run directory and the
        temporary directories, and no reads of the user's home."""

        def copy(work: Path) -> None:
            _check(["tar", "-xf", "-", "-C", str(work)], stdin=workspace_tar(workspace))

        directory = self._root / f"workspace-{uuid.uuid4().hex[:12]}"
        return self._guarded(directory, features, copy, sandboxed=True)

    def _guarded(
        self,
        directory: Path,
        features: tuple[str, ...],
        populate: Callable[[Path], None],
        sandboxed: bool,
    ) -> TestReport:
        run = _Run(directory, time.monotonic() + self._time_limit_s)
        shutil.rmtree(run.dir, ignore_errors=True)
        try:
            work = run.dir / "work"
            work.mkdir(parents=True)
            populate(work)
            return self._run(work, features, run, sandboxed)
        except _TimedOut:
            reason = f"acceptance run timed out after {self._time_limit_s:g}s"
            return failed_report(run.expected, reason, LABEL)
        finally:
            _sweep(run.dir)
            shutil.rmtree(run.dir, ignore_errors=True)

    def _run(self, work: Path, features: Sequence[str], run: "_Run", sandboxed: bool) -> TestReport:
        home, target = run.dir / "home", run.dir / "target"
        for d in (home, target):
            d.mkdir()
        test_dirs = [f"features/{f}/tests" for f in features]
        pytest = [str(self._toolchain.python), "-m", "pytest", "-p", "no:cacheprovider"]
        env = {**self._toolchain.cargo_env(home), "CARGO_TARGET_DIR": str(target)}
        jail: list[str] = []
        cargo = [str(self._toolchain.cargo)]
        if sandboxed:
            jail = ["/usr/bin/sandbox-exec", "-p", _profile(run.dir, self._toolchain)]
            # The rustup proxies may write to the shared toolchain; the sandbox forbids that.
            env["PATH"] = f"{self._toolchain.rust_bin}:{env['PATH']}"
            env["CARGO_HOME"] = str(run.dir / "cargo-home")
            env["PATH"] = f"{_unprivileged_tools(run.dir / 'bin')}:{env['PATH']}"
            cargo = [str(self._toolchain.rust_bin / "cargo")]

        exit_code, output = run.step(
            [*jail, *pytest, "--collect-only", "-q", *test_dirs], work, env
        )
        if exit_code not in (0, 5):
            reason = f"collection failed (exit {exit_code}):\n{_tail(output)}"
            return failed_report((), reason, LABEL, fallback="<collect>")
        run.expected = collected_tests(output.decode(errors="replace"))

        product = work / "climon2"
        build = [*cargo, "build", "--release", "--locked", "--offline"]
        if sandboxed:
            # Fetching needs the network, so it runs unsandboxed; it resolves the lockfile and
            # downloads sources but runs none of the workspace's code.
            vendor = run.dir / "vendor"
            fetch = [str(self._toolchain.cargo), "vendor", "--locked", "--versioned-dirs"]
            fetch_env = self._toolchain.cargo_env(home)
            exit_code, output = run.step([*fetch, str(vendor)], product, fetch_env)
            if exit_code != 0:
                return build_failed_report(
                    run.expected, f"cargo vendor failed:\n{_tail(output)}", LABEL
                )
            build += [
                "--config",
                'source.crates-io.replace-with="vendored"',
                "--config",
                f"source.vendored.directory={_quoted(str(vendor))}",
            ]
        else:
            exit_code, output = run.step([*cargo, "fetch", "--locked"], product, env)
            if exit_code != 0:
                log = f"cargo fetch failed:\n{_tail(output)}"
                return build_failed_report(run.expected, log, LABEL)
        exit_code, output = run.step([*jail, *build], product, env)
        if exit_code != 0:
            return build_failed_report(run.expected, _tail(output), LABEL)

        report = run.dir / "report.xml"
        test_env = {**env, "CLIMON2_BIN_DIR": str(target / "release")}
        exit_code, _ = run.step(
            [*jail, *pytest, f"--junitxml={report}", *test_dirs], work, test_env
        )
        xml = report.read_bytes()[: MAX_REPORT_BYTES + 1] if report.is_file() else b""
        return parse_report(xml, run.expected, exit_code, LABEL)


def _profile(run_dir: Path, toolchain: Toolchain) -> str:
    """A sandbox-exec profile: no network but local sockets, writes only to the run directory,
    the temporary directories and devices, and none of the user's home is readable but the
    run directory and the toolchain."""
    home = Path(pwd.getpwuid(os.getuid()).pw_dir).resolve()
    python_base = Path(os.path.realpath(toolchain.python)).parents[1]
    readable = [run_dir.resolve(), toolchain.home.resolve(), python_base]

    def path(p: Path) -> str:
        return _quoted(str(p))

    return "\n".join(
        [
            "(version 1)",
            "(allow default)",
            "(deny network*)",
            "(allow network* (local unix-socket) (remote unix-socket))",
            "(deny file-write*)",
            f"(allow file-write* (subpath {path(run_dir.resolve())})"
            + ' (subpath "/private/tmp") (subpath "/private/var/folders") (subpath "/dev"))',
            f"(deny file-read* (subpath {path(home)}))",
            "(allow file-read* " + " ".join(f"(subpath {path(p)})" for p in readable) + ")",
            "(allow file-read-metadata "
            + " ".join(f"(literal {path(a)})" for a in _ancestors(readable, home))
            + ")",
        ]
    )


# The sandbox cannot exec setuid binaries; ad-hoc signed copies work for this user's processes.
SETUID_TOOLS = (Path("/bin/ps"),)


def _unprivileged_tools(directory: Path) -> Path:
    directory.mkdir()
    for tool in SETUID_TOOLS:
        copy = directory / tool.name
        shutil.copyfile(tool, copy)
        copy.chmod(0o755)
        _check(["/usr/bin/codesign", "--force", "--sign", "-", str(copy)])
    return directory


def _ancestors(paths: Sequence[Path], within: Path) -> list[Path]:
    """The directories inside `within` that lead to `paths`, so those paths can be resolved."""
    return sorted({a for p in paths for a in (p, *p.parents) if a.is_relative_to(within)})


def _quoted(value: str) -> str:
    """A double-quoted string, valid in both TOML and sandbox profiles."""
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


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
