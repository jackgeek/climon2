"""The real `factory` command: production dependencies, built only when first used."""

import subprocess
import sys
from datetime import UTC, datetime
from functools import cached_property
from pathlib import Path

from climon2_factory.cli import EXIT_REFUSED, Deps, main
from climon2_factory.container_executor import ContainerExecutor, Docker, build_image
from climon2_factory.copilot_agents import CopilotAgents
from climon2_factory.github_client import GitHubClient
from climon2_factory.native_macos import NativeMacOS, Toolchain
from climon2_factory.ports import AgentRequest, Submission, TestReport

REPOSITORY = "jackgeek/climon2"


def entry() -> None:
    root = _git_toplevel(Path.cwd())
    if root is None:
        print("factory: not inside a git repository", file=sys.stderr)
        sys.exit(EXIT_REFUSED)
    sys.exit(main(sys.argv[1:], real_deps(root, Path.home() / ".climon2-factory")))


def real_deps(repo_root: Path, home: Path) -> Deps:
    executor = _LazyContainers()
    return Deps(
        repo_root=repo_root,
        state_dir=home / "runs",
        agents=_LazyAgents(executor),
        executor=executor,
        github=GitHubClient(REPOSITORY),
        macos=_LazyMacOS(repo_root, home),
        notifier=MacNotifier(),
        clock=SystemClock(),
    )


class _LazyContainers:
    """Builds the factory image on first use, so read-only commands never touch Docker."""

    @cached_property
    def _executor(self) -> ContainerExecutor:
        docker = Docker.rancher()
        return ContainerExecutor(docker, build_image(docker))

    def run_acceptance(
        self, workspace: Path, features: tuple[str, ...], subject: str | None
    ) -> TestReport:
        return self._executor.run_acceptance(workspace, features, subject)

    def lock_crates(self, workspace: Path, project: str) -> str:
        return self._executor.lock_crates(workspace, project)


class _LazyAgents:
    def __init__(self, containers: _LazyContainers) -> None:
        self._agents = CopilotAgents(lock_crates=containers.lock_crates)

    def run(self, request: AgentRequest) -> Submission:
        return self._agents.run(request)


class _LazyMacOS:
    """Installs the factory toolchain on first use."""

    def __init__(self, repo_root: Path, home: Path) -> None:
        self._repo_root, self._home = repo_root, home

    @cached_property
    def _macos(self) -> NativeMacOS:
        toolchain = Toolchain.ensure(self._home / "toolchain")
        return NativeMacOS(self._repo_root, toolchain, root=self._home / "macos-runs")

    def run_acceptance(self, commit: str, features: tuple[str, ...]) -> TestReport:
        return self._macos.run_acceptance(commit, features)


class MacNotifier:
    def notify(self, message: str) -> None:
        subprocess.run(
            [
                "/usr/bin/osascript",
                "-e",
                "on run argv",
                "-e",
                'display notification (item 1 of argv) with title "climon2 factory"',
                "-e",
                "end run",
                message,
            ],
            check=False,
            capture_output=True,
        )


class SystemClock:
    def now(self) -> datetime:
        return datetime.now(UTC)


def _git_toplevel(cwd: Path) -> Path | None:
    result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=cwd,
        capture_output=True,
        text=True,
        check=False,
    )
    return Path(result.stdout.strip()) if result.returncode == 0 else None
