"""The path-scoped tools each agent role is given. Deterministic: no model is involved."""

import os
import re
from pathlib import Path

import pytest
from fakes import write

from climon2_factory.agent_tools import MAX_OUTPUT_BYTES, Toolbox, ToolRefused
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

READ = ("factory_read_file", "factory_list_files", "factory_search")
EDIT = ("factory_write_file", "factory_replace_text", "factory_delete_file")


class Locks:
    def __init__(self) -> None:
        self.calls: list[tuple[Path, str]] = []

    def __call__(self, workspace: Path, project: str) -> str:
        self.calls.append((workspace, project))
        return "Cargo.lock updated"


def toolbox(
    repo: Path,
    role: Role,
    run_tests: object = None,
    locks: Locks | None = None,
    run_windows_tests: object = None,
    run_macos_tests: object = None,
) -> Toolbox:
    request = AgentRequest(
        role,
        "f1",
        "task",
        repo,
        run_tests=run_tests,  # type: ignore[arg-type]
        run_windows_tests=run_windows_tests,  # type: ignore[arg-type]
        run_macos_tests=run_macos_tests,  # type: ignore[arg-type]
    )
    return Toolbox(request, locks or Locks())


@pytest.mark.parametrize(
    ("role", "names"),
    [
        (Role.CONTRACT_AUTHOR, (*READ, *EDIT, "factory_submit_contract")),
        (Role.TEST_AUTHOR, (*READ, *EDIT, "factory_update_lockfile", "factory_submit_tests")),
        (
            Role.IMPLEMENTER,
            (*READ, *EDIT, "factory_update_lockfile", "factory_submit_implementation"),
        ),
        (Role.REVIEWER, (*READ, "factory_submit_review")),
    ],
)
def test_each_role_gets_only_its_tools(repo: Path, role: Role, names: tuple[str, ...]) -> None:
    assert toolbox(repo, role).tool_names == names


def test_the_implementer_can_run_the_acceptance_tests_when_the_factory_allows(repo: Path) -> None:
    box = toolbox(repo, Role.IMPLEMENTER, run_tests=lambda: TestReport("x", ()))

    assert "factory_run_acceptance_tests" in box.tool_names
    assert "factory_run_windows_tests" not in box.tool_names
    assert "factory_run_macos_tests" not in box.tool_names


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
        box.call("factory_write_file", path=path, content="new")
        assert (repo / path).read_text() == "new"
    else:
        with pytest.raises(ToolRefused):
            box.call("factory_write_file", path=path, content="new")
        assert not (repo / path).exists() or (repo / path).read_text() != "new"


@pytest.mark.parametrize(
    "path", ["climon2/../features/f1/tests/x.py", "../outside.txt", "/etc/passwd", "climon2/link/x"]
)
def test_paths_cannot_escape_their_scope(repo: Path, path: str) -> None:
    (repo / "climon2").mkdir()
    os.symlink(repo / "features", repo / "climon2" / "link")
    box = toolbox(repo, Role.IMPLEMENTER)

    with pytest.raises(ToolRefused):
        box.call("factory_write_file", path=path, content="x")
    with pytest.raises(ToolRefused):
        box.call("factory_delete_file", path=path)


def test_files_can_be_deleted_within_scope(repo: Path) -> None:
    write(repo, "climon2/src/old.rs", "x")
    toolbox(repo, Role.IMPLEMENTER).call("factory_delete_file", path="climon2/src/old.rs")

    assert not (repo / "climon2/src/old.rs").exists()


def test_the_repo_can_be_read_listed_and_searched_but_not_git_or_beyond(repo: Path) -> None:
    write(repo, "climon2/src/main.rs", 'fn main() {\n    println!("hi");\n}\n')
    write(repo, ".git/config", "secret")
    write(repo.parent, "outside.txt", "secret")
    box = toolbox(repo, Role.REVIEWER)

    assert "println" in box.call("factory_read_file", path="climon2/src/main.rs")
    listing = box.call("factory_list_files", path=".")
    assert "climon2/src/main.rs" in listing
    assert "features/f1/brief.md" in listing
    assert ".git" not in listing
    assert "climon2/src/main.rs:2:" in box.call("factory_search", pattern="println", path=".")
    for path in (".git/config", "../outside.txt"):
        with pytest.raises(ToolRefused):
            box.call("factory_read_file", path=path)


def test_submissions_are_typed(repo: Path) -> None:
    contract = toolbox(repo, Role.CONTRACT_AUTHOR)
    contract.call("factory_submit_contract")
    assert contract.submission == ContractSubmitted()

    tests = toolbox(repo, Role.TEST_AUTHOR)
    control = {
        "name": "wrong-greeting",
        "path": "features/f1/negative-controls/wrong-greeting",
        "target_test": "features/f1/tests/test_hello.py::test_greets",
        "expected_failure": "wrong greeting",
    }
    tests.call("factory_submit_tests", negative_controls=[control])
    assert tests.submission == TestsSubmitted((NegativeControl(**control),))

    review = toolbox(repo, Role.REVIEWER)
    review.call(
        "factory_submit_review", findings=[{"severity": "blocking", "summary": "no exit code"}]
    )
    assert review.submission == ReviewSubmitted((Finding(Severity.BLOCKING, "no exit code"),))

    implementer = toolbox(repo, Role.IMPLEMENTER)
    assert implementer.submission is None
    implementer.call("factory_submit_implementation")
    assert implementer.submission == ImplementationSubmitted()


def test_malformed_submissions_are_refused(repo: Path) -> None:
    review = toolbox(repo, Role.REVIEWER)
    with pytest.raises(ToolRefused):
        review.call("factory_submit_review", findings=[{"severity": "urgent", "summary": "x"}])
    tests = toolbox(repo, Role.TEST_AUTHOR)
    with pytest.raises(ToolRefused):
        tests.call(
            "factory_submit_tests",
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

    summary = box.call("factory_run_acceptance_tests")
    assert "1 passed, 1 failed" in summary
    assert "FAILED t::b" in summary
    assert "assert 1 == 2" in summary
    assert "10 runs used" in box.call("factory_run_acceptance_tests")


def test_windows_runs_are_summarised_and_budgeted(repo: Path) -> None:
    runs = iter([TestReport("github", (TestResult("t::w", Outcome.FAILED, "ConPTY closed"),))])

    def run_windows_tests() -> TestReport:
        report = next(runs, None)
        if report is None:
            raise RunBudgetExhausted("3 Windows runs used in this attempt")
        return report

    box = toolbox(repo, Role.IMPLEMENTER, run_windows_tests=run_windows_tests)

    summary = box.call("factory_run_windows_tests")
    assert "0 passed, 1 failed (github)" in summary
    assert "FAILED t::w" in summary
    assert "ConPTY closed" in summary
    assert "3 Windows runs used" in box.call("factory_run_windows_tests")


def test_macos_runs_are_summarised_and_budgeted(repo: Path) -> None:
    runs = iter([TestReport("macos", (TestResult("t::m", Outcome.FAILED, "EINVAL"),))])

    def run_macos_tests() -> TestReport:
        report = next(runs, None)
        if report is None:
            raise RunBudgetExhausted("10 macOS runs used in this attempt")
        return report

    box = toolbox(repo, Role.IMPLEMENTER, run_macos_tests=run_macos_tests)

    summary = box.call("factory_run_macos_tests")
    assert "0 passed, 1 failed (macos)" in summary
    assert "FAILED t::m" in summary
    assert "EINVAL" in summary
    assert "10 macOS runs used" in box.call("factory_run_macos_tests")


def test_a_run_with_many_long_failures_fits_the_hosts_output_limit(repo: Path) -> None:
    failures = tuple(TestResult(f"t::case_{n}", Outcome.FAILED, "x" * 6000) for n in range(100))
    box = toolbox(repo, Role.IMPLEMENTER, run_tests=lambda: TestReport("github", failures))

    summary = box.call("factory_run_acceptance_tests")

    assert len(summary.encode()) <= MAX_OUTPUT_BYTES
    assert summary.startswith("0 passed, 100 failed (github)")
    assert "FAILED t::case_0\n" in summary
    assert "t::case_99" in summary


def test_lockfiles_are_updated_by_the_factory_within_scope(repo: Path) -> None:
    locks = Locks()
    write(repo, "climon2/Cargo.toml", "[package]\n")
    box = toolbox(repo, Role.IMPLEMENTER, locks=locks)

    assert box.call("factory_update_lockfile", project="climon2") == "Cargo.lock updated"
    assert locks.calls == [(repo, "climon2")]
    with pytest.raises(ToolRefused):
        box.call("factory_update_lockfile", project="features/f1/negative-controls/x")


def test_a_large_file_is_read_in_pages_that_fit_the_hosts_output_limit(repo: Path) -> None:
    lines = [f"line {n}: " + "x" * 400 for n in range(1, 101)]
    write(repo, "features/f1/contract.md", "\n".join(lines) + "\n")
    box = toolbox(repo, Role.REVIEWER)

    pages = [box.call("factory_read_file", path="features/f1/contract.md")]
    while found := re.search(r"start_line=(\d+)", pages[-1]):
        pages.append(
            box.call("factory_read_file", path="features/f1/contract.md", start_line=int(found[1]))
        )

    assert len(pages) > 1
    assert all(len(page.encode()) < 20_000 for page in pages)
    assert (
        "".join(re.sub(r"\[lines .*\]\n?$", "", page) for page in pages) == "\n".join(lines) + "\n"
    )


def test_a_line_range_can_be_read(repo: Path) -> None:
    write(repo, "climon2/src/main.rs", "one\ntwo\nthree\nfour\n")

    page = toolbox(repo, Role.REVIEWER).call(
        "factory_read_file", path="climon2/src/main.rs", start_line=2, end_line=3
    )

    assert page.startswith("two\nthree\n")
    assert "four" not in page
    assert "of 4" in page


def test_search_and_listing_output_fit_the_hosts_output_limit(repo: Path) -> None:
    for n in range(200):
        write(repo, f"climon2/src/module_number_{n:04}_with_a_long_name.rs", "y" * 290 + "\n")
    box = toolbox(repo, Role.REVIEWER)

    hits = box.call("factory_search", pattern="y", path="climon2")
    listing = box.call("factory_list_files", path="climon2")

    assert len(hits.encode()) < 20_000 and "more" in hits
    assert "module_number_0000" in listing


def test_a_unique_passage_can_be_replaced_without_rewriting_the_file(repo: Path) -> None:
    write(repo, "features/f1/contract.md", "- **B1.** exits 1.\n- **B2.** exits 2.\n")
    box = toolbox(repo, Role.CONTRACT_AUTHOR)

    box.call("factory_replace_text", path="features/f1/contract.md", old="exits 2", new="exits 3")

    assert (repo / "features/f1/contract.md").read_text() == (
        "- **B1.** exits 1.\n- **B2.** exits 3.\n"
    )


@pytest.mark.parametrize(
    ("path", "old"),
    [
        ("features/f1/contract.md", "exits"),
        ("features/f1/contract.md", "exits 9"),
        ("features/f1/brief.md", "Run"),
    ],
)
def test_replacement_needs_exactly_one_match_within_scope(repo: Path, path: str, old: str) -> None:
    write(repo, "features/f1/contract.md", "- **B1.** exits 1.\n- **B2.** exits 2.\n")
    before = (repo / path).read_text()

    with pytest.raises(ToolRefused):
        toolbox(repo, Role.CONTRACT_AUTHOR).call(
            "factory_replace_text", path=path, old=old, new="changed"
        )
    assert (repo / path).read_text() == before
