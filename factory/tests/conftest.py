from dataclasses import dataclass, field
from pathlib import Path

import pytest
from fakes import FakeAgents, FakeClock, FakeExecutor, FakeGitHub, FakeMacOS, FakeNotifier, write

from climon2_factory.cli import Deps, main


@dataclass
class Factory:
    """Drives the factory CLI in-process against a temporary repo with fake ports."""

    repo: Path
    state_dir: Path
    capsys: pytest.CaptureFixture[str]
    agents: FakeAgents = field(default_factory=lambda: FakeAgents({}))
    executor: FakeExecutor = field(default_factory=FakeExecutor)
    github: FakeGitHub = field(default_factory=FakeGitHub)
    macos: FakeMacOS = field(default_factory=FakeMacOS)
    notifier: FakeNotifier = field(default_factory=FakeNotifier)
    clock: FakeClock = field(default_factory=FakeClock)

    def __call__(self, *argv: str) -> tuple[int, str]:
        self.capsys.readouterr()
        deps = Deps(
            repo_root=self.repo,
            state_dir=self.state_dir,
            agents=self.agents,
            executor=self.executor,
            github=self.github,
            macos=self.macos,
            notifier=self.notifier,
            clock=self.clock,
        )
        code = main(list(argv), deps)
        captured = self.capsys.readouterr()
        return code, captured.out + captured.err


@pytest.fixture
def repo(tmp_path: Path) -> Path:
    root = tmp_path / "repo"
    write(root, "CONTEXT.md", "# Glossary\n")
    write(root, "features/f1/brief.md", "Run a command in a session.\n")
    return root


@pytest.fixture
def factory(repo: Path, tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> Factory:
    return Factory(repo=repo, state_dir=tmp_path / "state", capsys=capsys)
