from pathlib import Path

import pytest
from conftest import Factory
from fakes import AcceptanceCall, FakeAgents, FakeExecutor, write

from climon2_factory.artifacts import ArtifactStore, State
from climon2_factory.ports import (
    AgentRequest,
    ContractSubmitted,
    Finding,
    ImplementationSubmitted,
    NegativeControl,
    Outcome,
    ReviewSubmitted,
    Role,
    RunBudgetExhausted,
    Severity,
    TestReport,
    TestResult,
    TestsSubmitted,
)


def drafts_contract(text: str = "# Contract\n"):
    def behaviour(request: AgentRequest) -> ContractSubmitted:
        write(request.workspace, f"features/{request.feature}/contract.md", text)
        return ContractSubmitted()

    return behaviour


def reviews(*findings: Finding):
    def behaviour(request: AgentRequest) -> ReviewSubmitted:
        return ReviewSubmitted(findings=findings)

    return behaviour


def test_start_drafts_and_reviews_the_contract_then_pauses_for_approval(factory: Factory) -> None:
    factory.agents = FakeAgents(
        {Role.CONTRACT_AUTHOR: [drafts_contract()], Role.REVIEWER: [reviews()]}
    )

    code, out = factory("start", "f1")

    assert code == 0
    assert factory.agents.roles == [Role.CONTRACT_AUTHOR, Role.REVIEWER]
    contract = ArtifactStore(factory.repo).status("f1/contract")
    assert contract.state is State.UNAPPROVED
    assert f"factory approve f1/contract {contract.hash}" in out
    assert len(factory.notifier.messages) == 1

    code, out = factory("status")
    assert code == 0
    assert "f1" in out
    assert f"awaiting approval: f1/contract {contract.hash}" in out


def test_start_without_a_brief_is_refused_and_runs_no_agents(factory: Factory) -> None:
    code, out = factory("start", "no-such-feature")

    assert code == 2
    assert "features/no-such-feature/brief.md" in out
    assert factory.agents.roles == []
    assert factory("status")[1].strip() == "No active feature run."


def test_only_one_feature_run_may_be_active(factory: Factory) -> None:
    write(factory.repo, "features/f2/brief.md", "Another feature.\n")
    factory.agents = FakeAgents(
        {Role.CONTRACT_AUTHOR: [drafts_contract()], Role.REVIEWER: [reviews()]}
    )
    factory("start", "f1")

    code, out = factory("start", "f2")

    assert code == 2
    assert "f1" in out
    assert factory.agents.roles == [Role.CONTRACT_AUTHOR, Role.REVIEWER]


BLOCKING = Finding(Severity.BLOCKING, "Scenario S2 cites no requirement")
ADVISORY = Finding(Severity.ADVISORY, "Consider naming the error code")


def test_blocking_findings_send_the_contract_back_for_one_revision(factory: Factory) -> None:
    factory.agents = FakeAgents(
        {
            Role.CONTRACT_AUTHOR: [drafts_contract("v1\n"), drafts_contract("v2\n")],
            Role.REVIEWER: [reviews(BLOCKING), reviews()],
        }
    )

    code, out = factory("start", "f1")

    assert code == 0
    assert factory.agents.roles == [
        Role.CONTRACT_AUTHOR,
        Role.REVIEWER,
        Role.CONTRACT_AUTHOR,
        Role.REVIEWER,
    ]
    assert factory.agents.calls[2].findings == (BLOCKING,)
    contract = ArtifactStore(factory.repo).status("f1/contract")
    assert (factory.repo / "features/f1/contract.md").read_text() == "v2\n"
    assert f"factory approve f1/contract {contract.hash}" in out


def test_blocking_findings_after_two_rounds_pause_with_the_findings(factory: Factory) -> None:
    factory.agents = FakeAgents(
        {
            Role.CONTRACT_AUTHOR: [drafts_contract("v1\n"), drafts_contract("v2\n")],
            Role.REVIEWER: [reviews(BLOCKING), reviews(BLOCKING)],
        }
    )

    code, out = factory("start", "f1")

    assert code == 0
    assert len(factory.agents.roles) == 4
    assert "blocking: Scenario S2 cites no requirement" in out
    contract = ArtifactStore(factory.repo).status("f1/contract")
    assert f"factory approve f1/contract {contract.hash}" in out
    assert "blocking: Scenario S2 cites no requirement" in factory("status")[1]


def test_advisory_findings_do_not_block_and_are_shown(factory: Factory) -> None:
    factory.agents = FakeAgents(
        {Role.CONTRACT_AUTHOR: [drafts_contract()], Role.REVIEWER: [reviews(ADVISORY)]}
    )

    code, out = factory("start", "f1")

    assert code == 0
    assert factory.agents.roles == [Role.CONTRACT_AUTHOR, Role.REVIEWER]
    assert "advisory: Consider naming the error code" in out


NO_LIVENESS = NegativeControl(
    name="no-liveness",
    path="features/f1/negative-controls/no-liveness",
    target_test="test_list_hides_dead_sessions",
    expected_failure="dead session listed",
)


def writes_tests(*controls: NegativeControl):
    def behaviour(request: AgentRequest) -> TestsSubmitted:
        write(
            request.workspace, f"features/{request.feature}/tests/test_exec.py", "def test(): ...\n"
        )
        write(request.workspace, "climon2/src/main.rs", "fn main() { unimplemented!() }\n")
        for control in controls:
            write(request.workspace, f"{control.path}/README.md", control.name)
        return TestsSubmitted(negative_controls=controls)

    return behaviour


def red_run(workspace: Path, features: tuple[str, ...], subject: str | None) -> TestReport:
    """Every test fails against the scaffold; each control fails its target as intended."""
    if subject is None:
        return TestReport(
            label="local-container",
            results=(
                TestResult("test_exec_runs_command", Outcome.FAILED, "not implemented"),
                TestResult("test_list_hides_dead_sessions", Outcome.FAILED, "not implemented"),
            ),
        )
    return TestReport(
        label="local-container",
        results=(
            TestResult("test_exec_runs_command", Outcome.PASSED),
            TestResult("test_list_hides_dead_sessions", Outcome.FAILED, "dead session listed"),
        ),
    )


def reach_contract_approval(factory: Factory, **more: list) -> str:
    scripts = {Role.CONTRACT_AUTHOR: [drafts_contract()], Role.REVIEWER: [reviews()]}
    for role, behaviours in more.items():
        scripts.setdefault(Role(role.replace("_", "-")), []).extend(behaviours)
    factory.agents = FakeAgents(scripts)
    factory("start", "f1")
    hash = ArtifactStore(factory.repo).status("f1/contract").hash
    assert hash is not None
    return hash


def test_approving_a_hash_that_is_not_current_is_refused(factory: Factory) -> None:
    reach_contract_approval(factory)

    code, out = factory("approve", "f1/contract", "sha256:" + "0" * 64)

    assert code == 2
    assert "not the current recorded hash" in out
    assert factory.agents.roles == [Role.CONTRACT_AUTHOR, Role.REVIEWER]


def test_approving_the_contract_produces_reviewed_tests_with_red_evidence(
    factory: Factory,
) -> None:
    contract_hash = reach_contract_approval(
        factory, test_author=[writes_tests(NO_LIVENESS)], reviewer=[reviews()]
    )
    factory.executor = FakeExecutor(red_run)

    code, out = factory("approve", "f1/contract", contract_hash)

    assert code == 0
    assert factory.agents.roles == [
        Role.CONTRACT_AUTHOR,
        Role.REVIEWER,
        Role.TEST_AUTHOR,
        Role.REVIEWER,
    ]
    assert factory.executor.calls == [
        AcceptanceCall(("f1",), None),
        AcceptanceCall(("f1",), NO_LIVENESS.path),
    ]
    store = ArtifactStore(factory.repo)
    tests = store.status("f1/tests")
    assert tests.state is State.UNAPPROVED
    assert store.status("f1/red-evidence").state is State.UNAPPROVED
    assert f"factory approve f1/tests {tests.hash}" in out
    assert store.status("f1/contract").state is State.APPROVED


def report(*results: tuple[str, Outcome, str]) -> TestReport:
    return TestReport("local-container", tuple(TestResult(*r) for r in results))


F = Outcome.FAILED
P = Outcome.PASSED
BAD_RED_RUNS = {
    "a test passes against the scaffold": (
        report(("test_exec_runs_command", P, ""), ("test_list_hides_dead_sessions", F, "x")),
        report(("test_list_hides_dead_sessions", F, "dead session listed")),
        "test_exec_runs_command passed against the unimplemented scaffold",
    ),
    "the scaffold run has no tests": (
        report(),
        report(("test_list_hides_dead_sessions", F, "dead session listed")),
        "no acceptance tests ran against the scaffold",
    ),
    "control's target test passes": (
        report(("test_list_hides_dead_sessions", F, "not implemented")),
        report(("test_list_hides_dead_sessions", P, "")),
        "negative control no-liveness did not fail test_list_hides_dead_sessions",
    ),
    "control fails for the wrong reason": (
        report(("test_list_hides_dead_sessions", F, "not implemented")),
        report(("test_list_hides_dead_sessions", F, "timed out")),
        "negative control no-liveness failed test_list_hides_dead_sessions for the wrong reason",
    ),
}


@pytest.mark.parametrize("case", BAD_RED_RUNS)
def test_bad_red_evidence_sends_the_tests_back_to_the_test_author(
    factory: Factory, case: str
) -> None:
    scaffold, control, expected = BAD_RED_RUNS[case]
    runs = [scaffold, control]
    contract_hash = reach_contract_approval(
        factory,
        test_author=[writes_tests(NO_LIVENESS), writes_tests(NO_LIVENESS)],
        reviewer=[reviews()],
    )
    factory.executor = FakeExecutor(
        lambda workspace, features, subject: (
            runs.pop(0) if runs else red_run(workspace, features, subject)
        )
    )

    code, _ = factory("approve", "f1/contract", contract_hash)

    assert code == 0
    assert factory.agents.roles[2:] == [Role.TEST_AUTHOR, Role.TEST_AUTHOR, Role.REVIEWER]
    assert factory.agents.calls[3].findings == (Finding(Severity.BLOCKING, expected),)


def test_rejecting_sends_the_artifact_to_a_fresh_author_with_the_reason(factory: Factory) -> None:
    first_hash = reach_contract_approval(
        factory,
        contract_author=[drafts_contract("v2\n")],
        reviewer=[reviews()],
    )

    code, out = factory("reject", "f1/contract", "--reason", "add a resize scenario")

    assert code == 0
    assert factory.agents.roles[2:] == [Role.CONTRACT_AUTHOR, Role.REVIEWER]
    revision = factory.agents.calls[2]
    assert revision.task == "revise-contract"
    assert revision.findings == (
        Finding(Severity.BLOCKING, "user rejected: add a resize scenario"),
    )
    store = ArtifactStore(factory.repo)
    new_hash = store.status("f1/contract").hash
    assert new_hash != first_hash
    assert f"factory approve f1/contract {new_hash}" in out
    assert [d.decision for d in store.history("f1/contract")[0].decisions] == ["rejected"]


def test_rejection_requires_a_reason(factory: Factory) -> None:
    reach_contract_approval(factory)

    code, _ = factory("reject", "f1/contract")

    assert code == 2
    assert ArtifactStore(factory.repo).status("f1/contract").state is State.UNAPPROVED


def test_only_the_artifact_the_run_is_waiting_for_can_be_decided(factory: Factory) -> None:
    reach_contract_approval(factory)
    write(factory.repo, "features/f1/other.md", "x\n")
    other = ArtifactStore(factory.repo).record("f1/other", paths=["features/f1/other.md"])

    approve_code, approve_out = factory("approve", "f1/other", other.hash)
    reject_code, _ = factory("reject", "f1/other", "--reason", "no")

    assert (approve_code, reject_code) == (2, 2)
    assert "waiting for f1/contract" in approve_out
    assert ArtifactStore(factory.repo).status("f1/other").state is State.UNAPPROVED


def reach_tests_approval(factory: Factory, **more: list) -> str:
    contract_hash = reach_contract_approval(
        factory, test_author=[writes_tests(NO_LIVENESS)], reviewer=[reviews()], **more
    )
    factory.executor = FakeExecutor(red_run)
    factory("approve", "f1/contract", contract_hash)
    hash = ArtifactStore(factory.repo).status("f1/tests").hash
    assert hash is not None
    return hash


def implements(
    text: str = "fn main() {}\n", runs: int = 0, also_writes: dict[str, str] | None = None
):
    def behaviour(request: AgentRequest) -> ImplementationSubmitted:
        write(request.workspace, "climon2/src/main.rs", text)
        for rel, content in (also_writes or {}).items():
            write(request.workspace, rel, content)
        assert request.run_tests is not None
        try:
            for _ in range(runs):
                request.run_tests()
        except RunBudgetExhausted:
            pass
        return ImplementationSubmitted()

    return behaviour


def failing_gate(workspace: Path, features: tuple[str, ...], subject: str | None) -> TestReport:
    return report(
        ("test_exec_runs_command", F, "exit code 1"),
        ("test_list_hides_dead_sessions", P, ""),
    )


def test_three_failed_attempts_pause_the_run_with_the_failures(factory: Factory) -> None:
    tests_hash = reach_tests_approval(
        factory, implementer=[implements("a\n"), implements("b\n"), implements("c\n")]
    )
    factory.executor = FakeExecutor(failing_gate)

    code, out = factory("approve", "f1/tests", tests_hash)

    assert code == 0
    implementer_calls = [c for c in factory.agents.calls if c.role is Role.IMPLEMENTER]
    assert len(implementer_calls) == 3
    failure = Finding(Severity.BLOCKING, "test_exec_runs_command failed: exit code 1")
    assert [c.findings for c in implementer_calls] == [(), (failure,), (failure,)]
    assert factory.executor.calls == [AcceptanceCall(("f1",), None)] * 3
    assert "implementation failed after 3 attempts" in out
    assert "Next: factory resume" in out
    assert "implementation failed after 3 attempts" in factory("status")[1]


def test_implementer_changes_outside_climon2_fail_the_attempt_and_are_reverted(
    factory: Factory,
) -> None:
    tampered = {
        "features/f1/tests/test_exec.py": "def test(): assert True\n",
        "features/f1/new.md": "sneaky\n",
    }
    tests_hash = reach_tests_approval(
        factory,
        implementer=[implements(also_writes=tampered), implements(), implements()],
    )
    factory.executor = FakeExecutor(failing_gate)

    factory("approve", "f1/tests", tests_hash)

    second = [c for c in factory.agents.calls if c.role is Role.IMPLEMENTER][1]
    assert second.findings == (
        Finding(Severity.BLOCKING, "implementer changed protected path features/f1/new.md"),
        Finding(
            Severity.BLOCKING,
            "implementer changed protected path features/f1/tests/test_exec.py",
        ),
    )
    assert (factory.repo / "features/f1/tests/test_exec.py").read_text() == "def test(): ...\n"
    assert not (factory.repo / "features/f1/new.md").exists()
    assert ArtifactStore(factory.repo).status("f1/tests").state is State.APPROVED


def test_each_attempt_may_run_the_tests_at_most_ten_times(factory: Factory) -> None:
    tests_hash = reach_tests_approval(
        factory, implementer=[implements(runs=11), implements(), implements()]
    )
    factory.executor = FakeExecutor(failing_gate)

    factory("approve", "f1/tests", tests_hash)

    first = next(c for c in factory.agents.calls if c.role is Role.IMPLEMENTER)
    assert first.findings == ()
    assert len(factory.executor.calls) == 10 + 3
