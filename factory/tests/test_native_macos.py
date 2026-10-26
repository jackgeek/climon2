"""The native macOS gate on this Mac. Run with `pytest -m macos`."""

import shutil
import subprocess
import time
from pathlib import Path

import pytest

from climon2_factory.native_macos import NativeMacOS, Toolchain
from climon2_factory.ports import Outcome, TestResult

pytestmark = pytest.mark.macos

FIXTURE = Path(__file__).parent / "fixtures" / "workspace"
P = Outcome.PASSED
F = Outcome.FAILED
GREETS = "features/f1/tests/test_hello.py::test_greets"
EXITS = "features/f1/tests/test_hello.py::test_exits_cleanly"


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=repo, capture_output=True, text=True, check=True
    ).stdout.strip()


def commit_all(repo: Path) -> str:
    git(repo, "add", "-A")
    git(repo, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "x")
    return git(repo, "rev-parse", "HEAD")


@pytest.fixture(scope="session")
def toolchain() -> Toolchain:
    return Toolchain.ensure()


@pytest.fixture
def repo(tmp_path: Path) -> Path:
    repo = shutil.copytree(FIXTURE, tmp_path / "source repo ü")
    git(repo, "init", "-q")
    return repo


@pytest.fixture
def runs(tmp_path: Path) -> Path:
    return tmp_path / "runs"


def test_runs_the_requested_features_at_the_commit_and_reports_each_test(
    toolchain: Toolchain, repo: Path, runs: Path
) -> None:
    commit = commit_all(repo)
    shutil.copy(  # uncommitted: must not be what is tested
        FIXTURE / "features/f1/negative-controls/wrong-greeting/src/main.rs",
        repo / "climon2/src/main.rs",
    )

    report = NativeMacOS(repo, toolchain, runs).run_acceptance(commit, ("f1",))

    assert report.label == "macos-native"
    assert report.results == (TestResult(GREETS, P), TestResult(EXITS, P))
    assert list(runs.iterdir()) == []


def test_a_commit_that_does_not_build_fails_every_test_with_the_build_log(
    toolchain: Toolchain, repo: Path, runs: Path
) -> None:
    broken = FIXTURE / "features/f1/negative-controls/does-not-compile/src/main.rs"
    shutil.copy(broken, repo / "climon2/src/main.rs")
    commit = commit_all(repo)

    report = NativeMacOS(repo, toolchain, runs).run_acceptance(commit, ("f1",))

    build, *rest = report.results
    assert (build.test_id, build.outcome) == ("build", F)
    assert "error" in build.message
    assert rest == [
        TestResult(GREETS, F, "not run: build failed"),
        TestResult(EXITS, F, "not run: build failed"),
    ]


def test_tests_run_with_a_temporary_home_and_none_of_the_factorys_environment(
    toolchain: Toolchain, repo: Path, runs: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("CLIMON2_FACTORY_CANARY", "secret")
    commit = commit_all(repo)

    report = NativeMacOS(repo, toolchain, runs).run_acceptance(commit, ("env",))

    assert report.results == (
        TestResult("features/env/tests/test_env.py::test_cannot_see_factory_secrets", P),
        TestResult("features/env/tests/test_env.py::test_home_is_not_the_users_home", P),
    )


def test_a_run_over_its_time_limit_is_killed_and_fails(
    toolchain: Toolchain, repo: Path, runs: Path
) -> None:
    commit = commit_all(repo)
    started = time.monotonic()

    report = NativeMacOS(repo, toolchain, runs, time_limit_s=20).run_acceptance(commit, ("hang",))

    assert time.monotonic() - started < 60
    assert report.results == (
        TestResult(
            "features/hang/tests/test_hang.py::test_hangs", F, "acceptance run timed out after 20s"
        ),
    )
    assert list(runs.iterdir()) == []


def test_processes_left_behind_by_the_tests_are_killed(
    toolchain: Toolchain, repo: Path, runs: Path
) -> None:
    commit = commit_all(repo)
    try:
        report = NativeMacOS(repo, toolchain, runs).run_acceptance(commit, ("orphan",))

        assert report.results == (
            TestResult(
                "features/orphan/tests/test_orphan.py::test_leaves_a_detached_process_behind", P
            ),
        )
        assert leftover_sleeps() == ""
    finally:
        subprocess.run(["pkill", "-f", "^sleep 2999$"], check=False)


def leftover_sleeps() -> str:
    return subprocess.run(
        ["pgrep", "-f", "^sleep 2999$"], capture_output=True, text=True, check=False
    ).stdout
