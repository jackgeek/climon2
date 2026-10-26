"""The Executor that runs acceptance tests in a local VirtualBox Windows VM.

The VM (built by `windows-vm/create-vm.sh`) sits on a host-only network with no internet. Every
run restores its `clean` snapshot, uploads the workspace and the locked crates over SSH, builds
offline, runs pytest, and powers the VM off again. Windows CI stays the authoritative gate."""

import fcntl
import os
import shutil
import subprocess
import tempfile
import time
from collections.abc import Callable, Generator, Sequence
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path

from climon2_factory.container_executor import BUILD_FAILED, PRODUCT_DIR, workspace_tar
from climon2_factory.ports import TestReport
from climon2_factory.reports import (
    MAX_REPORT_BYTES,
    build_failed_report,
    collected_tests,
    failed_report,
    parse_report,
)

LABEL = "windows-vm"
RUNNER = Path(__file__).resolve().parents[2] / "windows-vm" / "run-acceptance.ps1"
SNAPSHOT = "clean"
_LOG_TAIL = 20_000

VendoredCrates = Callable[[Path, str], bytes | None]


class VMError(RuntimeError):
    pass


@dataclass(frozen=True)
class WindowsVM:
    name: str
    home: Path
    host: str
    user: str = "climon2"

    @classmethod
    def default(cls) -> "WindowsVM":
        home = Path(os.environ.get("CLIMON2_VM_HOME", Path.home() / "VMs" / "climon2-win"))
        return cls("climon2-win", home, "192.168.57.10")

    def available(self) -> bool:
        """Whether the VM has been built, provisioned and isolated."""
        if shutil.which("VBoxManage") is None or not (self.home / "id_ed25519").is_file():
            return False
        result = self._vbox("snapshot", self.name, "list", "--machinereadable", check=False)
        return f'="{SNAPSHOT}"' in result.stdout

    def running(self) -> bool:
        return self._state() in {"running", "starting", "stopping", "paused"}

    @contextmanager
    def session(self, deadline: float) -> Generator["WindowsVM"]:
        """The VM, freshly restored to its clean snapshot, for one caller at a time."""
        self.home.mkdir(parents=True, exist_ok=True)
        with open(self.home / ".lock", "w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            try:
                self._power_off()
                self._vbox("snapshot", self.name, "restore", SNAPSHOT)
                self._vbox("startvm", self.name, "--type", "headless")
                self._await_ssh(deadline)
                yield self
            finally:
                self._power_off()

    def run(self, command: str, deadline: float) -> subprocess.CompletedProcess[bytes]:
        """A cmd.exe command line in the VM. Raises TimeoutExpired at the deadline."""
        return subprocess.run(
            ["ssh", *self._ssh_options(), f"{self.user}@{self.host}", command],
            capture_output=True,
            timeout=max(1.0, deadline - time.monotonic()),
            check=False,
        )

    def put(self, data: bytes, remote: str, deadline: float) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            local = Path(tmp) / "upload"
            local.write_bytes(data)
            self._scp(str(local), f"{self.user}@{self.host}:{_sftp_path(remote)}", deadline)

    def get(self, remote: str, deadline: float) -> bytes:
        """One file from the VM, or b"" if absent. Oversized files are truncated so the report
        parser rejects them."""
        with tempfile.TemporaryDirectory() as tmp:
            local = Path(tmp) / "download"
            try:
                self._scp(f"{self.user}@{self.host}:{_sftp_path(remote)}", str(local), deadline)
            except VMError:
                return b""
            with open(local, "rb") as file:
                return file.read(MAX_REPORT_BYTES + 1)

    def _scp(self, source: str, target: str, deadline: float) -> None:
        result = subprocess.run(
            ["scp", "-q", *self._ssh_options(), source, target],
            capture_output=True,
            timeout=max(1.0, deadline - time.monotonic()),
            check=False,
        )
        if result.returncode != 0:
            raise VMError(f"scp {source} {target} failed: {result.stderr.decode(errors='replace')}")

    def _ssh_options(self) -> list[str]:
        return [
            "-i", str(self.home / "id_ed25519"),
            "-o", "IdentitiesOnly=yes",
            "-o", "BatchMode=yes",
            "-o", "StrictHostKeyChecking=no",
            "-o", "UserKnownHostsFile=/dev/null",
            "-o", "LogLevel=ERROR",
            "-o", "ConnectTimeout=5",
        ]  # fmt: skip

    def _await_ssh(self, deadline: float) -> None:
        give_up = min(deadline, time.monotonic() + 300)
        while True:
            try:
                if self.run("exit 0", give_up).returncode == 0:
                    return
            except subprocess.TimeoutExpired:
                pass
            if time.monotonic() >= give_up:
                raise VMError(f"{self.name} did not accept SSH at {self.host}")
            time.sleep(2)

    def _power_off(self) -> None:
        if self.running():
            self._vbox("controlvm", self.name, "poweroff", check=False)
        for _ in range(60):
            if not self.running() and self._session_free():
                return
            time.sleep(0.5)
        raise VMError(f"{self.name} did not power off")

    def _session_free(self) -> bool:
        info = self._vbox("showvminfo", self.name, "--machinereadable", check=False).stdout
        return 'SessionName=""' in info or "SessionName=" not in info

    def _state(self) -> str:
        info = self._vbox("showvminfo", self.name, "--machinereadable", check=False).stdout
        for line in info.splitlines():
            if line.startswith("VMState="):
                return line.split("=", 1)[1].strip('"')
        return "unknown"

    def _vbox(self, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        result = subprocess.run(
            ["VBoxManage", "-q", *args], capture_output=True, text=True, timeout=120, check=False
        )
        if check and result.returncode != 0:
            raise VMError(f"VBoxManage {' '.join(args)} failed: {result.stderr}")
        return result


def _sftp_path(windows_path: str) -> str:
    return "/" + windows_path.replace("\\", "/")


class WindowsVMExecutor:
    def __init__(
        self, vm: WindowsVM, vendored_crates: VendoredCrates, time_limit_s: float = 1800
    ) -> None:
        self._vm = vm
        self._vendored_crates = vendored_crates
        self._time_limit_s = time_limit_s

    def run_acceptance(
        self, workspace: Path, features: tuple[str, ...], subject: str | None
    ) -> TestReport:
        product = subject or PRODUCT_DIR
        test_dirs = tuple(f"features/{f}/tests" for f in features)
        archive = workspace_tar(workspace)
        crates = self._vendored_crates(workspace, product)
        deadline = time.monotonic() + self._time_limit_s
        collected: tuple[str, ...] = ()
        try:
            with self._vm.session(deadline) as vm:
                vm.put(archive, "C:/factory/work.tar", deadline)
                if crates is not None:
                    vm.put(crates, "C:/factory/crates.tar", deadline)
                vm.put(RUNNER.read_bytes(), "C:/factory/run-acceptance.ps1", deadline)
                unpacked = vm.run(_phase("unpack", product), deadline)
                if unpacked.returncode != 0:
                    raise VMError(f"unpacking the workspace failed:\n{_output(unpacked)}")

                collect = vm.run(_phase("collect", product, test_dirs), deadline)
                if collect.returncode == 5:
                    return TestReport(LABEL, ())
                if collect.returncode != 0:
                    reason = f"collection failed (exit {collect.returncode}):\n{_output(collect)}"
                    return failed_report((), reason, LABEL, fallback="<collect>")
                collected = collected_tests(collect.stdout.decode(errors="replace"))

                build = vm.run(_phase("build", product), deadline)
                if build.returncode == BUILD_FAILED:
                    return build_failed_report(collected, _output(build), LABEL)
                if build.returncode != 0:
                    raise VMError(
                        f"build phase failed (exit {build.returncode}):\n{_output(build)}"
                    )

                test = vm.run(_phase("test", product, test_dirs), deadline)
                report = vm.get("C:/factory/report.xml", deadline)
                return parse_report(report, collected, test.returncode, LABEL)
        except subprocess.TimeoutExpired:
            reason = f"acceptance run timed out after {self._time_limit_s:g}s"
            return failed_report(collected, reason, LABEL)


def _phase(phase: str, product: str, args: Sequence[str] = ()) -> str:
    runner = r"C:\factory\run-acceptance.ps1"
    return " ".join(
        ["powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File", runner]
        + [phase, product, *args]
    )


def _output(result: subprocess.CompletedProcess[bytes]) -> str:
    return (result.stdout + result.stderr).decode(errors="replace")[-_LOG_TAIL:]
