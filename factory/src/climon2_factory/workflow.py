"""Feature-run state machine: advances a run until it reaches a pause that needs the user."""

import json
from dataclasses import dataclass
from pathlib import Path

from climon2_factory.artifacts import ArtifactStore, State
from climon2_factory.ports import (
    AgentRequest,
    Agents,
    ContractSubmitted,
    Executor,
    Finding,
    NegativeControl,
    Outcome,
    ReviewSubmitted,
    Role,
    Severity,
    Submission,
    TestReport,
    TestsSubmitted,
)
from climon2_factory.runs import Run, RunStore

MAX_REVIEW_ROUNDS = 2

DRAFT_CONTRACT = "draft-contract"
REVIEW_CONTRACT = "review-contract"
AWAIT_CONTRACT_APPROVAL = "await-contract-approval"
WRITE_TESTS = "write-tests"
RED_RUN = "red-run"
REVIEW_TESTS = "review-tests"
AWAIT_TESTS_APPROVAL = "await-tests-approval"


class StepFailed(Exception):
    """An agent or executor step ended without producing what the workflow requires."""


@dataclass(frozen=True)
class Pause:
    reason: str
    next_command: str
    details: tuple[str, ...] = ()


@dataclass(frozen=True)
class _Phase:
    """An author → review → approval cycle producing one artifact."""

    kind: str
    author: Role
    author_stage: str
    review_stage: str
    approval_stage: str
    next_stage: str


_CONTRACT = _Phase(
    "contract",
    Role.CONTRACT_AUTHOR,
    DRAFT_CONTRACT,
    REVIEW_CONTRACT,
    AWAIT_CONTRACT_APPROVAL,
    WRITE_TESTS,
)
_TESTS = _Phase(
    "tests", Role.TEST_AUTHOR, WRITE_TESTS, REVIEW_TESTS, AWAIT_TESTS_APPROVAL, "implement"
)
_PHASES = (_CONTRACT, _TESTS)


class Workflow:
    def __init__(self, repo_root: Path, runs: RunStore, agents: Agents, executor: Executor) -> None:
        self._repo = repo_root
        self._runs = runs
        self._agents = agents
        self._executor = executor
        self._store = ArtifactStore(repo_root)

    def advance(self, run: Run) -> Pause:
        while True:
            if run.stage == RED_RUN:
                self._red_run(run)
                continue
            phase = self._phase(run.stage)
            if run.stage == phase.author_stage:
                self._author(run, phase)
            elif run.stage == phase.review_stage:
                self._review(run, phase)
            elif self._store.status(self._name(run, phase)).state is State.APPROVED:
                run.review_round = 0
                self._move(run, phase.next_stage)
            else:
                return self.pause(run)

    def pause(self, run: Run) -> Pause:
        phase = self._phase(run.stage)
        name = self._name(run, phase)
        hash = self._store.status(name).hash
        details = tuple(f"{f.severity}: {f.summary}" for f in self._current_findings(run, phase))
        return Pause(
            reason=f"awaiting approval: {name} {hash}",
            next_command=f"factory approve {name} {hash}",
            details=details,
        )

    def _author(self, run: Run, phase: _Phase) -> None:
        findings = self._current_findings(run, phase)
        task = f"revise-{phase.kind}" if findings else phase.author_stage
        submission = self._agent(phase.author, run, task, findings)
        if phase is _CONTRACT:
            _expect(submission, ContractSubmitted)
            self._store.record(
                self._name(run, phase),
                paths=[f"features/{run.feature}/contract.md"],
                path_inputs=[f"features/{run.feature}/brief.md", "CONTEXT.md"],
            )
            self._move(run, REVIEW_CONTRACT)
        else:
            self._record_tests(run, _expect(submission, TestsSubmitted).negative_controls)
            self._move(run, RED_RUN)

    def _record_tests(self, run: Run, controls: tuple[NegativeControl, ...]) -> None:
        declared = f"features/{run.feature}/negative-controls.json"
        _write_json(
            self._repo / declared,
            [
                {
                    "name": c.name,
                    "path": c.path,
                    "target_test": c.target_test,
                    "expected_failure": c.expected_failure,
                }
                for c in controls
            ],
        )
        self._store.record(
            f"{run.feature}/tests",
            paths=[f"features/{run.feature}/tests", declared, *(c.path for c in controls)],
            artifact_inputs=[f"{run.feature}/contract"],
        )

    def _red_run(self, run: Run) -> None:
        features = (run.feature,)
        scaffold = self._executor.run_acceptance(self._repo, features, None)
        controls = [
            (c, self._executor.run_acceptance(self._repo, features, c["path"]))
            for c in self._negative_controls(run)
        ]
        evidence = f"features/{run.feature}/evidence/red.json"
        _write_json(
            self._repo / evidence,
            {
                "tests_hash": self._store.status(f"{run.feature}/tests").hash,
                "scaffold": _report_json(scaffold),
                "negative_controls": [
                    {"name": c["name"], "report": _report_json(r)} for c, r in controls
                ],
            },
        )
        self._store.record(
            f"{run.feature}/red-evidence",
            paths=[evidence],
            artifact_inputs=[f"{run.feature}/tests"],
        )
        problems = _red_problems(scaffold, controls)
        if problems:
            self._route_findings(run, _TESTS, problems)
        else:
            self._move(run, REVIEW_TESTS)

    def _negative_controls(self, run: Run) -> list[dict[str, str]]:
        path = self._repo / f"features/{run.feature}/negative-controls.json"
        return json.loads(path.read_text())

    def _review(self, run: Run, phase: _Phase) -> None:
        submission = self._agent(Role.REVIEWER, run, phase.review_stage)
        self._route_findings(run, phase, _expect(submission, ReviewSubmitted).findings)

    def _route_findings(self, run: Run, phase: _Phase, findings: tuple[Finding, ...]) -> None:
        """Blocking findings return the artifact to its author, at most MAX_REVIEW_ROUNDS times."""
        run.review_round += 1
        self._write_findings(run, phase, findings)
        blocking = any(f.severity is Severity.BLOCKING for f in findings)
        if blocking and run.review_round < MAX_REVIEW_ROUNDS:
            self._move(run, phase.author_stage)
        else:
            self._move(run, phase.approval_stage)

    def _findings_path(self, run: Run, phase: _Phase) -> str:
        return f"features/{run.feature}/findings/{phase.kind}-{run.review_round}.json"

    def _write_findings(self, run: Run, phase: _Phase, findings: tuple[Finding, ...]) -> None:
        name = self._name(run, phase)
        rel = self._findings_path(run, phase)
        _write_json(
            self._repo / rel,
            {
                "artifact": name,
                "reviewed_hash": self._store.status(name).hash,
                "findings": [
                    {"severity": f.severity.value, "summary": f.summary} for f in findings
                ],
            },
        )
        self._store.record(f"{name}-findings", paths=[rel], artifact_inputs=[name])

    def _current_findings(self, run: Run, phase: _Phase) -> tuple[Finding, ...]:
        """Findings from the latest review of this phase, if it reviewed the current artifact."""
        if not run.review_round:
            return ()
        record = json.loads((self._repo / self._findings_path(run, phase)).read_text())
        if record["reviewed_hash"] != self._store.status(self._name(run, phase)).hash:
            return ()
        return tuple(Finding(Severity(f["severity"]), f["summary"]) for f in record["findings"])

    def _agent(
        self, role: Role, run: Run, task: str, findings: tuple[Finding, ...] = ()
    ) -> Submission:
        return self._agents.run(
            AgentRequest(
                role=role, feature=run.feature, task=task, workspace=self._repo, findings=findings
            )
        )

    def _phase(self, stage: str) -> _Phase:
        for phase in _PHASES:
            if stage in (phase.author_stage, phase.review_stage, phase.approval_stage):
                return phase
        raise RuntimeError(f"unknown stage {stage!r}")

    @staticmethod
    def _name(run: Run, phase: _Phase) -> str:
        return f"{run.feature}/{phase.kind}"

    def _move(self, run: Run, stage: str) -> None:
        run.stage = stage
        self._runs.save(run)


def _expect[T](submission: Submission, expected: type[T]) -> T:
    if not isinstance(submission, expected):
        raise StepFailed(f"expected {expected.__name__}, got {type(submission).__name__}")
    return submission


def _red_problems(
    scaffold: TestReport, controls: list[tuple[dict[str, str], TestReport]]
) -> tuple[Finding, ...]:
    problems: list[str] = []
    if not scaffold.results:
        problems.append("no acceptance tests ran against the scaffold")
    problems += [
        f"{r.test_id} passed against the unimplemented scaffold"
        for r in scaffold.results
        if r.outcome is Outcome.PASSED
    ]
    for control, control_report in controls:
        name, target = control["name"], control["target_test"]
        result = next((r for r in control_report.results if r.test_id == target), None)
        if result is None or result.outcome is not Outcome.FAILED:
            problems.append(f"negative control {name} did not fail {target}")
        elif control["expected_failure"] not in result.message:
            problems.append(f"negative control {name} failed {target} for the wrong reason")
    return tuple(Finding(Severity.BLOCKING, p) for p in problems)


def _report_json(report: TestReport) -> dict[str, object]:
    return {
        "label": report.label,
        "results": [
            {"test_id": r.test_id, "outcome": r.outcome.value, "message": r.message}
            for r in report.results
        ],
    }


def _write_json(path: Path, data: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + "\n")
