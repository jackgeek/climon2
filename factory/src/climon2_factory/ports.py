"""System-boundary ports. Real implementations arrive in build steps 3 and 4; tests use fakes."""

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


Submission = ContractSubmitted | ReviewSubmitted | TestsSubmitted


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
