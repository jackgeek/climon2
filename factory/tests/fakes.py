"""Scripted fakes for the factory's system-boundary ports."""

from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import UTC, datetime, timedelta
from pathlib import Path

from climon2_factory.ports import AgentRequest, CiResult, Role, Submission, TestReport

Script = Callable[[AgentRequest], Submission]
AcceptanceScript = Callable[[Path, tuple[str, ...], str | None], TestReport]


class FakeAgents:
    """Plays each role's scripted behaviours in order; fails loudly when a role runs out."""

    def __init__(self, scripts: dict[Role, list[Script]]) -> None:
        self._scripts = {role: list(behaviours) for role, behaviours in scripts.items()}
        self.calls: list[AgentRequest] = []

    def run(self, request: AgentRequest) -> Submission:
        self.calls.append(request)
        remaining = self._scripts.get(request.role, [])
        if not remaining:
            raise AssertionError(f"unexpected {request.role} invocation")
        return remaining.pop(0)(request)

    @property
    def roles(self) -> list[Role]:
        return [call.role for call in self.calls]


@dataclass(frozen=True)
class AcceptanceCall:
    features: tuple[str, ...]
    subject: str | None


class FakeExecutor:
    """Answers acceptance runs with a script; records each call's features and subject."""

    def __init__(self, script: AcceptanceScript | None = None) -> None:
        self._script = script
        self.calls: list[AcceptanceCall] = []

    def run_acceptance(
        self, workspace: Path, features: tuple[str, ...], subject: str | None
    ) -> TestReport:
        self.calls.append(AcceptanceCall(features, subject))
        if self._script is None:
            raise AssertionError("unexpected acceptance run")
        return self._script(workspace, features, subject)


class Crash(Exception):
    """The factory process dying mid-step (power cut, kill -9, laptop lid)."""


class FakeGitHub:
    """Each push yields commit c1, c2, …; Windows CI answers from a script keyed by commit."""

    def __init__(
        self,
        windows: Callable[[str], CiResult] | None = None,
        changed: tuple[str, ...] = ("climon2/src/main.rs",),
    ) -> None:
        self._windows = windows
        self._changed = changed
        self.pushes: list[str] = []
        self.prs: list[tuple[str, str, str]] = []
        self.pr_states: dict[int, str] = {}
        self.crash_after_opening_pr = False

    def push_candidate(self, workspace: Path, branch: str) -> str:
        self.pushes.append(branch)
        return f"c{len(self.pushes)}"

    def windows_ci(self, commit: str) -> CiResult:
        if self._windows is None:
            raise AssertionError("unexpected Windows CI request")
        return self._windows(commit)

    def changed_files(self, workspace: Path, commit: str) -> tuple[str, ...]:
        return self._changed

    def open_pr(self, branch: str, title: str, body: str) -> int:
        self.prs.append((branch, title, body))
        number = len(self.prs)
        self.pr_states[number] = "open"
        if self.crash_after_opening_pr:
            raise Crash("factory killed after opening the PR")
        return number

    def find_pr(self, branch: str) -> int | None:
        for number, (pr_branch, _, _) in enumerate(self.prs, start=1):
            if pr_branch == branch:
                return number
        return None

    def pr_state(self, number: int) -> str:
        return self.pr_states[number]


class FakeMacOS:
    def __init__(self, script: Callable[[str], TestReport] | None = None) -> None:
        self._script = script
        self.commits: list[str] = []

    def run_acceptance(self, commit: str, features: tuple[str, ...]) -> TestReport:
        self.commits.append(commit)
        if self._script is None:
            raise AssertionError("unexpected macOS run")
        return self._script(commit)


@dataclass
class FakeNotifier:
    messages: list[str] = field(default_factory=list[str])

    def notify(self, message: str) -> None:
        self.messages.append(message)


@dataclass
class FakeClock:
    current: datetime = datetime(2026, 1, 1, tzinfo=UTC)

    def now(self) -> datetime:
        return self.current

    def advance(self, delta: timedelta) -> None:
        self.current += delta


def write(root: Path, rel: str, text: str) -> None:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
