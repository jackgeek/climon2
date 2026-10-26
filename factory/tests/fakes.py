"""Scripted fakes for the factory's system-boundary ports."""

from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import UTC, datetime, timedelta
from pathlib import Path

from climon2_factory.ports import AgentRequest, Role, Submission, TestReport

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
