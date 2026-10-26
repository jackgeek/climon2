"""The Executor that runs acceptance tests in fresh, offline, factory-owned containers."""

import hashlib
import io
import os
import subprocess
import tarfile
import uuid
from collections.abc import Generator, Sequence
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path

from climon2_factory.ports import TestReport
from climon2_factory.reports import (
    MAX_REPORT_BYTES,
    build_failed_report,
    collected_tests,
    failed_report,
    parse_report,
)

LABEL = "local-container"
CONTAINERS_DIR = Path(__file__).resolve().parents[2] / "containers"
RANCHER_BIN = Path("/Applications/Rancher Desktop.app/Contents/Resources/resources/darwin/bin")
PRODUCT_DIR = "climon2"
BUILD_FAILED = 97
NOT_COPIED = frozenset({".git", ".factory", "factory", "target", "__pycache__", ".venv"})
CONTAINER_LABEL = "climon2-factory.executor"
_LOG_TAIL = 20_000


@dataclass(frozen=True)
class Docker:
    """The docker CLI, talking to one daemon. Runs with a minimal environment."""

    cli: Path
    host: str

    @classmethod
    def rancher(cls) -> "Docker":
        return cls(RANCHER_BIN / "docker", f"unix://{Path.home()}/.rd/docker.sock")

    def __call__(
        self,
        *args: str,
        stdin: bytes | None = None,
        timeout: float | None = None,
        check: bool = True,
    ) -> subprocess.CompletedProcess[bytes]:
        env = {
            # The CLI finds its credential helpers next to itself.
            "PATH": f"{self.cli.parent}:/usr/bin:/bin",
            "DOCKER_HOST": self.host,
            "HOME": str(Path.home()),
        }
        result = subprocess.run(
            [str(self.cli), *args],
            check=False,
            input=stdin,
            capture_output=True,
            env=env,
            timeout=timeout,
        )
        if check and result.returncode != 0:
            raise DockerError(f"docker {args[0]} failed: {result.stderr.decode(errors='replace')}")
        return result


class DockerError(RuntimeError):
    pass


def build_image(docker: Docker, context: Path = CONTAINERS_DIR) -> str:
    """Build the factory image and return its content-addressed id (`sha256:…`)."""
    return docker("build", "-q", str(context)).stdout.decode().strip()


class ContainerExecutor:
    def __init__(
        self,
        docker: Docker,
        image: str,
        time_limit_s: float = 1800,
        cpus: str = "2",
        memory: str = "3g",
    ) -> None:
        self._docker = docker
        self._image = image
        self._time_limit_s = time_limit_s
        self._limits = ("--cpus", cpus, "--memory", memory, "--pids-limit", "1024")

    def run_acceptance(
        self, workspace: Path, features: tuple[str, ...], subject: str | None
    ) -> TestReport:
        archive = _workspace_tar(workspace)
        test_dirs = tuple(f"features/{f}/tests" for f in features)
        collected = self._collect(archive, test_dirs)
        if isinstance(collected, TestReport):
            return collected
        product = subject or PRODUCT_DIR
        vendor = self._vendor(archive, workspace, product)
        with self._container(
            archive,
            ["/opt/factory/run-acceptance.sh", product, *test_dirs],
            mounts=(f"{vendor}:/vendor:ro",) if vendor else (),
        ) as box:
            exit_code = box.wait(self._time_limit_s)
            if exit_code is None:
                reason = f"acceptance run timed out after {self._time_limit_s:g}s"
                return failed_report(collected, reason, LABEL)
            if exit_code == BUILD_FAILED:
                return build_failed_report(collected, box.logs(), LABEL)
            return parse_report(box.read("/out/report.xml"), collected, exit_code, LABEL)

    def _collect(self, archive: bytes, test_dirs: Sequence[str]) -> tuple[str, ...] | TestReport:
        """The expected tests, collected where no product code is ever built or run."""
        command = ["python3", "-m", "pytest", "-p", "no:cacheprovider", "--rootdir=/work"]
        with self._container(archive, [*command, "--collect-only", "-q", *test_dirs]) as box:
            exit_code = box.wait(self._time_limit_s)
            output = box.logs()
        if exit_code == 5:
            return ()
        if exit_code != 0:
            reason = f"collection failed (exit {exit_code}):\n{output}"
            return failed_report((), reason, LABEL, fallback="<collect>")
        return collected_tests(output)

    def _vendor(self, archive: bytes, workspace: Path, product: str) -> str | None:
        """A volume holding the product's locked crates, fetched once per Cargo.lock.

        This is the only networked step. `cargo vendor` downloads sources but builds nothing."""
        lock = workspace / product / "Cargo.lock"
        if not lock.is_file():
            return None
        volume = f"climon2-factory-vendor-{hashlib.sha256(lock.read_bytes()).hexdigest()[:16]}"
        probe = ["test", "-f", "/vendor/.complete"]
        with self._container(None, probe, mounts=(f"{volume}:/vendor:ro",)) as box:
            if box.wait(60) == 0:
                return volume
        with self._container(
            archive,
            ["/opt/factory/vendor-crates.sh", product],
            mounts=(f"{volume}:/vendor",),
            network="bridge",
        ) as box:
            exit_code = box.wait(self._time_limit_s)
            if exit_code != 0:
                raise DockerError(f"vendoring crates failed (exit {exit_code}):\n{box.logs()}")
        return volume

    @contextmanager
    def _container(
        self,
        archive: bytes | None,
        command: Sequence[str],
        mounts: Sequence[str] = (),
        network: str = "none",
    ) -> Generator["_Box"]:
        name = f"climon2-factory-{uuid.uuid4().hex[:12]}"
        args = ["create", "--name", name, "--label", CONTAINER_LABEL, "--network", network]
        args += ["--cap-drop", "ALL", "--security-opt", "no-new-privileges", *self._limits]
        for mount in mounts:
            args += ["-v", mount]
        self._docker(*args, self._image, *command)
        try:
            if archive is not None:
                self._docker("cp", "-", f"{name}:/work", stdin=archive)
            self._docker("start", name)
            yield _Box(self._docker, name)
        finally:
            self._docker("rm", "-f", "-v", name, check=False)


class _Box:
    def __init__(self, docker: Docker, name: str) -> None:
        self._docker = docker
        self._name = name

    def wait(self, timeout_s: float) -> int | None:
        """The container's exit code, or None if it was killed for running too long."""
        try:
            result = self._docker("wait", self._name, timeout=timeout_s)
        except subprocess.TimeoutExpired:
            self._docker("kill", self._name, check=False)
            return None
        return int(result.stdout.decode().strip())

    def logs(self) -> str:
        result = self._docker("logs", self._name, check=False)
        return (result.stdout + result.stderr).decode(errors="replace")[-_LOG_TAIL:]

    def read(self, path: str) -> bytes:
        """One file from the container, or b"" if absent. Oversized files are truncated so the
        report parser rejects them."""
        result = self._docker("cp", f"{self._name}:{path}", "-", check=False)
        if result.returncode != 0:
            return b""
        with tarfile.open(fileobj=io.BytesIO(result.stdout)) as archive:
            member = archive.next()
            if member is None or not member.isfile():
                return b""
            data = archive.extractfile(member)
            assert data is not None
            return data.read(MAX_REPORT_BYTES + 1)


def _workspace_tar(workspace: Path) -> bytes:
    """The workspace, minus VCS data, build output and the factory, as root-owned files."""

    def keep(info: tarfile.TarInfo) -> tarfile.TarInfo | None:
        if NOT_COPIED.intersection(Path(info.name).parts):
            return None
        info.uid = info.gid = 0
        info.uname = info.gname = "root"
        info.mode |= 0o444 | (0o111 if info.isdir() or info.mode & 0o100 else 0)
        return info

    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as archive:
        for entry in sorted(os.listdir(workspace)):
            archive.add(workspace / entry, arcname=entry, filter=keep)
    return buffer.getvalue()
