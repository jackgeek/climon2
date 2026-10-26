"""The local-container Executor against a real dockerd. Run with `pytest -m container`."""

import shutil
from pathlib import Path

import pytest

from climon2_factory.container_executor import (
    CONTAINER_LABEL,
    ContainerExecutor,
    Docker,
    build_image,
)
from climon2_factory.ports import Outcome, TestResult

pytestmark = pytest.mark.container

FIXTURE = Path(__file__).parent / "fixtures" / "workspace"
P = Outcome.PASSED
F = Outcome.FAILED
GREETS = "features/f1/tests/test_hello.py::test_greets"
EXITS = "features/f1/tests/test_hello.py::test_exits_cleanly"


@pytest.fixture(scope="session")
def docker() -> Docker:
    return Docker.rancher()


@pytest.fixture(scope="session")
def image(docker: Docker) -> str:
    return build_image(docker)


@pytest.fixture
def workspace(tmp_path: Path) -> Path:
    return shutil.copytree(FIXTURE, tmp_path / "work space ü")


@pytest.fixture
def executor(docker: Docker, image: str) -> ContainerExecutor:
    return ContainerExecutor(docker, image, time_limit_s=300)


def test_builds_climon2_with_its_locked_dependencies_offline_and_reports_each_test(
    executor: ContainerExecutor, workspace: Path
) -> None:
    report = executor.run_acceptance(workspace, ("f1",), None)

    assert report.label == "local-container"
    assert report.results == (TestResult(GREETS, P), TestResult(EXITS, P))


def test_tests_cannot_reach_the_network(executor: ContainerExecutor, workspace: Path) -> None:
    report = executor.run_acceptance(workspace, ("net",), None)

    [result] = report.results
    assert result.test_id == "features/net/tests/test_net.py::test_reaches_the_network"
    assert result.outcome is F


def test_a_negative_control_replaces_climon2_as_the_subject(
    executor: ContainerExecutor, workspace: Path
) -> None:
    report = executor.run_acceptance(
        workspace, ("f1",), "features/f1/negative-controls/wrong-greeting"
    )

    greets, exits = report.results
    assert (greets.test_id, greets.outcome) == (GREETS, F)
    assert "wrong greeting" in greets.message
    assert exits == TestResult(EXITS, P)


def test_a_subject_that_does_not_build_fails_every_test_with_the_build_log(
    executor: ContainerExecutor, workspace: Path
) -> None:
    report = executor.run_acceptance(
        workspace, ("f1",), "features/f1/negative-controls/does-not-compile"
    )

    build, *rest = report.results
    assert (build.test_id, build.outcome) == ("build", F)
    assert "error" in build.message
    assert rest == [
        TestResult(GREETS, F, "not run: build failed"),
        TestResult(EXITS, F, "not run: build failed"),
    ]


def test_a_run_over_its_time_limit_fails_and_leaves_no_container(
    docker: Docker, image: str, workspace: Path
) -> None:
    executor = ContainerExecutor(docker, image, time_limit_s=20)

    report = executor.run_acceptance(workspace, ("hang",), None)

    assert report.results == (
        TestResult(
            "features/hang/tests/test_hang.py::test_hangs", F, "acceptance run timed out after 20s"
        ),
    )
    leftovers = docker("ps", "-aq", "--filter", f"label={CONTAINER_LABEL}").stdout
    assert leftovers == b""


def test_the_host_workspace_is_never_modified(executor: ContainerExecutor, workspace: Path) -> None:
    before = snapshot(workspace)

    executor.run_acceptance(workspace, ("f1",), None)
    executor.run_acceptance(workspace, ("f1",), "features/f1/negative-controls/wrong-greeting")

    assert snapshot(workspace) == before


def snapshot(root: Path) -> dict[str, bytes]:
    return {str(p.relative_to(root)): p.read_bytes() for p in root.rglob("*") if p.is_file()}


def test_a_lockfile_is_resolved_for_a_project_without_building_it(
    executor: ContainerExecutor, workspace: Path
) -> None:
    lock = workspace / "climon2" / "Cargo.lock"
    lock.unlink()

    message = executor.lock_crates(workspace, "climon2")

    assert 'name = "itoa"' in lock.read_text()
    assert "Cargo.lock" in message
    assert not (workspace / "climon2" / "target").exists()


def test_a_lockfile_that_cannot_be_resolved_is_reported(
    executor: ContainerExecutor, workspace: Path
) -> None:
    manifest = workspace / "climon2" / "Cargo.toml"
    manifest.write_text(manifest.read_text() + 'no-such-crate-climon2 = "9.9"\n')

    message = executor.lock_crates(workspace, "climon2")

    assert "failed" in message
    assert "no-such-crate-climon2" in message


def test_every_test_is_collected_however_long_the_collection_output(
    executor: ContainerExecutor, workspace: Path
) -> None:
    tests = workspace / "features/many/tests"
    tests.mkdir(parents=True)
    (tests / "test_many.py").write_text(
        "import pytest\n\n\n"
        "@pytest.mark.parametrize('n', range(300), ids=lambda n: f'case-{n:04}-' + 'x' * 60)\n"
        "def test_many(n):\n    pass\n"
    )

    report = executor.run_acceptance(workspace, ("many",), None)

    assert len(report.results) == 300
    assert all(r.outcome is P for r in report.results)
