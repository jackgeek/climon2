"""The GitHub port against jackgeek/climon2 itself, on throwaway factory/test-* branches.

The `github` tests push, run real Windows CI and open PRs: run with `pytest -m github`."""

import shutil
import subprocess
import uuid
from collections.abc import Iterator
from pathlib import Path

import pytest

from climon2_factory.github_client import GitHubClient, windows_report
from climon2_factory.ports import Outcome, TestResult

REPO = "jackgeek/climon2"
REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURE = Path(__file__).parent / "fixtures" / "workspace"
GREETS = "features/f1/tests/test_hello.py::test_greets"
EXITS = "features/f1/tests/test_hello.py::test_exits_cleanly"
P = Outcome.PASSED
F = Outcome.FAILED


def git(workspace: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=workspace, capture_output=True, text=True, check=True
    ).stdout.strip()


@pytest.mark.parametrize("branch", ["main", "feature/x", "factory", "refs/heads/factory/x"])
def test_only_factory_branches_can_be_pushed(tmp_path: Path, branch: str) -> None:
    with pytest.raises(ValueError, match="factory/"):
        GitHubClient(REPO).push_candidate(tmp_path, branch)


class Candidates:
    passing: tuple[str, str]
    failing: tuple[str, str]

    def __init__(self, workspace: Path, client: GitHubClient) -> None:
        self.workspace = workspace
        self.client = client
        self.run = uuid.uuid4().hex[:8]
        self.branches: list[str] = []

    def push(self, name: str) -> tuple[str, str]:
        branch = f"factory/test-{self.run}-{name}"
        self.branches.append(branch)
        return branch, self.client.push_candidate(self.workspace, branch)


@pytest.fixture(scope="module")
def candidates(tmp_path_factory: pytest.TempPathFactory) -> Iterator[Candidates]:
    """Two candidates pushed up front so their Windows CI runs overlap: one passes, one fails."""
    workspace = tmp_path_factory.mktemp("github") / "climon2"
    git(REPO_ROOT, "clone", "-q", str(REPO_ROOT), str(workspace))
    git(workspace, "remote", "set-url", "origin", f"https://github.com/{REPO}.git")
    shutil.copytree(FIXTURE / "climon2", workspace / "climon2")
    shutil.copytree(FIXTURE / "features" / "f1", workspace / "features" / "f1")
    pushed = Candidates(workspace, GitHubClient(REPO, poll_s=15))
    pushed.passing = pushed.push("pass")
    wrong = FIXTURE / "features/f1/negative-controls/wrong-greeting/src/main.rs"
    shutil.copy(wrong, workspace / "climon2/src/main.rs")
    pushed.failing = pushed.push("fail")
    yield pushed
    for branch in pushed.branches:
        subprocess.run(
            ["git", "push", "-q", "origin", "--delete", branch], cwd=workspace, check=False
        )


def test_windows_results_cover_every_collected_test_however_long_the_list(
    tmp_path: Path,
) -> None:
    ids = [f"features/f1/tests/test_hello.py::test_{'x' * 200}_{n}" for n in range(300)]
    (tmp_path / "collected").mkdir()
    (tmp_path / "collected/collected.txt").write_text("\n".join(ids) + "\n\n300 tests\n")
    (tmp_path / "collected/exit-code.txt").write_text("0\n")
    (tmp_path / "results").mkdir()
    (tmp_path / "results/build-exit-code.txt").write_text("0\n")
    (tmp_path / "results/exit-code.txt").write_text("0\n")
    cases = "".join(
        f'<testcase classname="features.f1.tests.test_hello" name="{i.split("::")[1]}" />'
        for i in ids
    )
    (tmp_path / "results/report.xml").write_text(
        f"<testsuites><testsuite>{cases}</testsuite></testsuites>"
    )

    report = windows_report(tmp_path, "success")

    assert report.results == tuple(TestResult(i, P) for i in ids)


@pytest.mark.github
def test_a_candidate_is_the_workspace_committed_on_head_and_pushed(
    candidates: Candidates,
) -> None:
    workspace = candidates.workspace
    branch, commit = candidates.failing

    assert git(workspace, "ls-remote", "origin", f"refs/heads/{branch}").split()[0] == commit
    assert git(workspace, "rev-parse", f"{commit}^") == git(workspace, "rev-parse", "HEAD")
    assert "goodbye" in git(workspace, "show", f"{commit}:climon2/src/main.rs")
    assert git(workspace, "diff", "--cached", "--name-only") == ""
    assert "climon2/" in git(workspace, "status", "--porcelain")


@pytest.mark.github
def test_changed_files_are_relative_to_main(candidates: Candidates) -> None:
    _, commit = candidates.passing

    changed = candidates.client.changed_files(candidates.workspace, commit)

    assert {"climon2/Cargo.toml", "climon2/src/main.rs", "features/f1/tests/test_hello.py"} <= set(
        changed
    )
    assert "CONTEXT.md" not in changed


@pytest.mark.github
def test_windows_ci_reports_each_test_for_the_pushed_commit(candidates: Candidates) -> None:
    _, commit = candidates.passing

    result = candidates.client.windows_ci(commit)

    assert result.commit == commit
    assert result.report.label == "github"
    assert result.report.results == (TestResult(GREETS, P), TestResult(EXITS, P))


@pytest.mark.github
def test_windows_ci_reports_failures_with_their_messages(candidates: Candidates) -> None:
    _, commit = candidates.failing

    result = candidates.client.windows_ci(commit)

    greets, exits = result.report.results
    assert (greets.test_id, greets.outcome) == (GREETS, F)
    assert "wrong greeting" in greets.message
    assert exits == TestResult(EXITS, P)


@pytest.mark.github
def test_a_pull_request_can_be_opened_found_and_its_state_read(candidates: Candidates) -> None:
    branch, _ = candidates.passing
    client = candidates.client

    number = client.open_pr(
        branch, "factory test PR (closed automatically)", "Test of the factory."
    )
    try:
        assert client.find_pr(branch) == number
        assert client.pr_state(number) == "open"
    finally:
        subprocess.run(["gh", "pr", "close", str(number), "--repo", REPO], check=True)
    assert client.pr_state(number) == "closed"
    assert client.find_pr(f"factory/test-{candidates.run}-never-pushed") is None
