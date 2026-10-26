"""The path-scoped tools each agent role is given. Deterministic: no model is involved."""

import os
from pathlib import Path

import pytest
from fakes import write

from climon2_factory.agent_tools import Toolbox, ToolRefused
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

READ = ("read_file", "list_files", "search")
EDIT = ("write_file", "delete_file")


class Locks:
    def __init__(self) -> None:
        self.calls: list[tuple[Path, str]] = []

    def __call__(self, workspace: Path, project: str) -> str:
        self.calls.append((workspace, project))
        return "Cargo.lock updated"


def toolbox(
    repo: Path, role: Role, run_tests: object = None, locks: Locks | None = None
) -> Toolbox:
    request = AgentRequest(role, "f1", "task", repo, run_tests=run_tests)  # type: ignore[arg-type]
    return Toolbox(request, locks or Locks())


@pytest.mark.parametrize(
    ("role", "names"),
    [
        (Role.CONTRACT_AUTHOR, (*READ, *EDIT, "submit_contract")),
        (Role.TEST_AUTHOR, (*READ, *EDIT, "update_lockfile", "submit_tests")),
        (Role.IMPLEMENTER, (*READ, *EDIT, "update_lockfile", "submit_implementation")),
        (Role.REVIEWER, (*READ, "submit_review")),
    ],
)
def test_each_role_gets_only_its_tools(repo: Path, role: Role, names: tuple[str, ...]) -> None:
    assert toolbox(repo, role).tool_names == names


def test_the_implementer_can_run_the_acceptance_tests_when_the_factory_allows(repo: Path) -> None:
    box = toolbox(repo, Role.IMPLEMENTER, run_tests=lambda: TestReport("x", ()))

    assert "run_acceptance_tests" in box.tool_names


@pytest.mark.parametrize(
    ("role", "path", "allowed"),
    [
        (Role.CONTRACT_AUTHOR, "features/f1/contract.md", True),
        (Role.CONTRACT_AUTHOR, "features/f1/brief.md", False),
        (Role.CONTRACT_AUTHOR, "CONTEXT.md", False),
        (Role.CONTRACT_AUTHOR, "climon2/src/main.rs", False),
        (Role.TEST_AUTHOR, "features/f1/tests/test_x.py", True),
        (Role.TEST_AUTHOR, "features/f1/negative-controls/bad/src/main.rs", True),
        (Role.TEST_AUTHOR, "climon2/src/main.rs", True),
        (Role.TEST_AUTHOR, "features/f1/contract.md", False),
        (Role.TEST_AUTHOR, "features/f2/tests/test_x.py", False),
        (Role.TEST_AUTHOR, "harness/drivers/pty.py", False),
        (Role.IMPLEMENTER, "climon2/src/main.rs", True),
        (Role.IMPLEMENTER, "climon2/Cargo.toml", True),
        (Role.IMPLEMENTER, "features/f1/tests/test_x.py", False),
        (Role.IMPLEMENTER, "features/f1/negative-controls/bad/src/main.rs", False),
        (Role.IMPLEMENTER, ".github/workflows/x.yml", False),
        (Role.IMPLEMENTER, "factory/src/x.py", False),
        (Role.IMPLEMENTER, ".factory/records/x.json", False),
    ],
)
def test_writes_are_confined_to_the_roles_paths(
    repo: Path, role: Role, path: str, allowed: bool
) -> None:
    box = toolbox(repo, role)

    if allowed:
        box.call("write_file", path=path, content="new")
        assert (repo / path).read_text() == "new"
    else:
        with pytest.raises(ToolRefused):
            box.call("write_file", path=path, content="new")
        assert not (repo / path).exists() or (repo / path).read_text() != "new"


@pytest.mark.parametrize(
    "path", ["climon2/../features/f1/tests/x.py", "../outside.txt", "/etc/passwd", "climon2/link/x"]
)
def test_paths_cannot_escape_their_scope(repo: Path, path: str) -> None:
    (repo / "climon2").mkdir()
    os.symlink(repo / "features", repo / "climon2" / "link")
    box = toolbox(repo, Role.IMPLEMENTER)

    with pytest.raises(ToolRefused):
        box.call("write_file", path=path, content="x")
    with pytest.raises(ToolRefused):
        box.call("delete_file", path=path)


def test_files_can_be_deleted_within_scope(repo: Path) -> None:
    write(repo, "climon2/src/old.rs", "x")
    toolbox(repo, Role.IMPLEMENTER).call("delete_file", path="climon2/src/old.rs")

    assert not (repo / "climon2/src/old.rs").exists()


def test_the_repo_can_be_read_listed_and_searched_but_not_git_or_beyond(repo: Path) -> None:
    write(repo, "climon2/src/main.rs", 'fn main() {\n    println!("hi");\n}\n')
    write(repo, ".git/config", "secret")
    write(repo.parent, "outside.txt", "secret")
    box = toolbox(repo, Role.REVIEWER)

    assert "println" in box.call("read_file", path="climon2/src/main.rs")
    listing = box.call("list_files", path=".")
    assert "climon2/src/main.rs" in listing
    assert "features/f1/brief.md" in listing
    assert ".git" not in listing
    assert "climon2/src/main.rs:2:" in box.call("search", pattern="println", path=".")
    for path in (".git/config", "../outside.txt"):
        with pytest.raises(ToolRefused):
            box.call("read_file", path=path)


def test_submissions_are_typed(repo: Path) -> None:
    contract = toolbox(repo, Role.CONTRACT_AUTHOR)
    contract.call("submit_contract")
    assert contract.submission == ContractSubmitted()

    tests = toolbox(repo, Role.TEST_AUTHOR)
    control = {
        "name": "wrong-greeting",
        "path": "features/f1/negative-controls/wrong-greeting",
        "target_test": "features/f1/tests/test_hello.py::test_greets",
        "expected_failure": "wrong greeting",
    }
    tests.call("submit_tests", negative_controls=[control])
    assert tests.submission == TestsSubmitted((NegativeControl(**control),))

    review = toolbox(repo, Role.REVIEWER)
    review.call("submit_review", findings=[{"severity": "blocking", "summary": "no exit code"}])
    assert review.submission == ReviewSubmitted((Finding(Severity.BLOCKING, "no exit code"),))

    implementer = toolbox(repo, Role.IMPLEMENTER)
    assert implementer.submission is None
    implementer.call("submit_implementation")
    assert implementer.submission == ImplementationSubmitted()


def test_malformed_submissions_are_refused(repo: Path) -> None:
    review = toolbox(repo, Role.REVIEWER)
    with pytest.raises(ToolRefused):
        review.call("submit_review", findings=[{"severity": "urgent", "summary": "x"}])
    tests = toolbox(repo, Role.TEST_AUTHOR)
    with pytest.raises(ToolRefused):
        tests.call(
            "submit_tests",
            negative_controls=[
                {"name": "x", "path": "climon2", "target_test": "t", "expected_failure": "e"}
            ],
        )
    assert review.submission is None
    assert tests.submission is None


def test_acceptance_runs_are_summarised_and_budgeted(repo: Path) -> None:
    runs = iter(
        [
            TestReport(
                "local-container",
                (
                    TestResult("t::a", Outcome.PASSED),
                    TestResult("t::b", Outcome.FAILED, "assert 1 == 2"),
                ),
            )
        ]
    )

    def run_tests() -> TestReport:
        report = next(runs, None)
        if report is None:
            raise RunBudgetExhausted("10 runs used in this attempt")
        return report

    box = toolbox(repo, Role.IMPLEMENTER, run_tests=run_tests)

    summary = box.call("run_acceptance_tests")
    assert "1 passed, 1 failed" in summary
    assert "FAILED t::b" in summary
    assert "assert 1 == 2" in summary
    assert "10 runs used" in box.call("run_acceptance_tests")


def test_lockfiles_are_updated_by_the_factory_within_scope(repo: Path) -> None:
    locks = Locks()
    write(repo, "climon2/Cargo.toml", "[package]\n")
    box = toolbox(repo, Role.IMPLEMENTER, locks=locks)

    assert box.call("update_lockfile", project="climon2") == "Cargo.lock updated"
    assert locks.calls == [(repo, "climon2")]
    with pytest.raises(ToolRefused):
        box.call("update_lockfile", project="features/f1/negative-controls/x")
