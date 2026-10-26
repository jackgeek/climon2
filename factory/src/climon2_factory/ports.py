"""System-boundary ports. Real implementations arrive in build steps 3 and 4; tests use fakes."""

from collections.abc import Callable
from dataclasses import dataclass
from datetime import datetime
from enum import StrEnum
from pathlib import Path
from typing import Protocol


class Role(StrEnum):
    CONTRACT_AUTHOR = "contract-author"
    TEST_AUTHOR = "test-author"
    IMPLEMENTER = "implementer"
    REVIEWER = "reviewer"


class Severity(StrEnum):
    BLOCKING = "blocking"
    ADVISORY = "advisory"


@dataclass(frozen=True)
class Finding:
    severity: Severity
    summary: str


@dataclass(frozen=True)
class AgentRequest:
    role: Role
    feature: str
    task: str
    workspace: Path
    findings: tuple[Finding, ...] = ()
    run_tests: "Callable[[], TestReport] | None" = None
    run_windows_tests: "Callable[[], TestReport] | None" = None


class RunBudgetExhausted(Exception):
    """The agent has used all of this attempt's acceptance-test runs."""


@dataclass(frozen=True)
class ContractSubmitted:
    pass


@dataclass(frozen=True)
class ReviewSubmitted:
    findings: tuple[Finding, ...]


@dataclass(frozen=True)
class NegativeControl:
    """A deliberately incorrect program and the test failure it must provoke."""

    name: str
    path: str
    target_test: str
    expected_failure: str


@dataclass(frozen=True)
class TestsSubmitted:
    __test__ = False  # not a pytest test class

    negative_controls: tuple[NegativeControl, ...]


@dataclass(frozen=True)
class ImplementationSubmitted:
    pass


Submission = ContractSubmitted | ReviewSubmitted | TestsSubmitted | ImplementationSubmitted


class Agents(Protocol):
    def run(self, request: AgentRequest) -> Submission: ...


class Outcome(StrEnum):
    PASSED = "passed"
    FAILED = "failed"


@dataclass(frozen=True)
class TestResult:
    __test__ = False  # not a pytest test class

    test_id: str
    outcome: Outcome
    message: str = ""


@dataclass(frozen=True)
class TestReport:
    __test__ = False  # not a pytest test class

    label: str
    results: tuple[TestResult, ...]


class Executor(Protocol):
    def run_acceptance(
        self, workspace: Path, features: tuple[str, ...], subject: str | None
    ) -> TestReport:
        """Run the named features' acceptance tests against climon2 built from the workspace,
        or against the negative control at `subject`."""
        ...


class Notifier(Protocol):
    def notify(self, message: str) -> None: ...


class Clock(Protocol):
    def now(self) -> datetime: ...


@dataclass(frozen=True)
class CiResult:
    commit: str
    report: TestReport


class GitHub(Protocol):
    def push_candidate(self, workspace: Path, branch: str) -> str:
        """Commit the workspace to `branch`, push it, and return the commit SHA."""
        ...

    def windows_ci(self, commit: str) -> CiResult:
        """Wait for the Windows acceptance workflow on `commit` and return its result."""
        ...

    def changed_files(self, workspace: Path, commit: str) -> tuple[str, ...]:
        """Files `commit` changes relative to the main branch."""
        ...

    def open_pr(self, branch: str, title: str, body: str) -> int:
        """Open a pull request from `branch` into main and return its number."""
        ...

    def find_pr(self, branch: str) -> int | None:
        """The pull request already opened from `branch`, if any."""
        ...

    def pr_state(self, number: int) -> str:
        """One of "open", "merged" or "closed"."""
        ...


class MacOS(Protocol):
    def run_acceptance(self, commit: str, features: tuple[str, ...]) -> TestReport:
        """Run the features' acceptance tests natively on this Mac against a fresh clone at
        `commit`. Only called after the user approves the candidate."""
        ...
