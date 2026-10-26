from pathlib import Path

import pytest
from conftest import Factory
from fakes import AcceptanceCall, FakeAgents, FakeExecutor, write

from climon2_factory.artifacts import ArtifactStore, State
from climon2_factory.ports import (
    AgentRequest,
    ContractSubmitted,
    Finding,
    NegativeControl,
    Outcome,
    ReviewSubmitted,
    Role,
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
