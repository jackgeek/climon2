"""The local Windows VM Executor against the real VirtualBox VM. Run with `pytest -m windows_vm`.

Needs the VM built by `windows-vm/create-vm.sh` and a running dockerd for the crate cache."""

import shutil
from pathlib import Path

import pytest

from climon2_factory.container_executor import ContainerExecutor, Docker, build_image
from climon2_factory.ports import Outcome, TestResult
from climon2_factory.windows_vm import WindowsVM, WindowsVMExecutor

pytestmark = pytest.mark.windows_vm

FIXTURE = Path(__file__).parent / "fixtures" / "workspace"
P = Outcome.PASSED
F = Outcome.FAILED
GREETS = "features/f1/tests/test_hello.py::test_greets"
EXITS = "features/f1/tests/test_hello.py::test_exits_cleanly"


@pytest.fixture(scope="session")
def containers() -> ContainerExecutor:
    docker = Docker.rancher()
    return ContainerExecutor(docker, build_image(docker))


@pytest.fixture(scope="session")
def vm() -> WindowsVM:
    vm = WindowsVM.default()
    assert vm.available(), "build the VM with windows-vm/create-vm.sh first"
    return vm


@pytest.fixture
def workspace(tmp_path: Path) -> Path:
    return shutil.copytree(FIXTURE, tmp_path / "work space ü")


@pytest.fixture
def executor(vm: WindowsVM, containers: ContainerExecutor) -> WindowsVMExecutor:
    return WindowsVMExecutor(vm, containers.vendored_crates, time_limit_s=900)


def test_builds_climon2_from_cached_crates_and_reports_each_test(
    executor: WindowsVMExecutor, workspace: Path
) -> None:
    report = executor.run_acceptance(workspace, ("f1",), None)

    assert report.label == "windows-vm"
    assert report.results == (TestResult(GREETS, P), TestResult(EXITS, P))


def test_tests_cannot_reach_the_network(executor: WindowsVMExecutor, workspace: Path) -> None:
    report = executor.run_acceptance(workspace, ("net",), None)

    [result] = report.results
    assert result.test_id == "features/net/tests/test_net.py::test_reaches_the_network"
    assert result.outcome is F


def test_a_negative_control_replaces_climon2_as_the_subject(
    executor: WindowsVMExecutor, workspace: Path
) -> None:
    report = executor.run_acceptance(
        workspace, ("f1",), "features/f1/negative-controls/wrong-greeting"
    )

    greets, exits = report.results
    assert (greets.test_id, greets.outcome) == (GREETS, F)
    assert "wrong greeting" in greets.message
    assert exits == TestResult(EXITS, P)


def test_a_subject_that_does_not_build_fails_every_test_with_the_build_log(
    executor: WindowsVMExecutor, workspace: Path
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


def test_a_run_over_its_time_limit_fails_and_leaves_the_vm_off(
    vm: WindowsVM, containers: ContainerExecutor, workspace: Path
) -> None:
    executor = WindowsVMExecutor(vm, containers.vendored_crates, time_limit_s=240)

    report = executor.run_acceptance(workspace, ("hang",), None)

    assert report.results == (
        TestResult(
            "features/hang/tests/test_hang.py::test_hangs", F, "acceptance run timed out after 240s"
        ),
    )
    assert not vm.running()


def test_each_run_starts_from_the_clean_snapshot(
    executor: WindowsVMExecutor, workspace: Path
) -> None:
    tests = workspace / "features/leak/tests"
    tests.mkdir(parents=True)
    (tests / "test_leak.py").write_text(
        "from pathlib import Path\n\n"
        "MARK = Path('C:/leaked.txt')\n\n\n"
        "def test_nothing_leaked_from_an_earlier_run():\n"
        "    assert not MARK.exists()\n"
        "    MARK.write_text('x')\n"
    )

    first = executor.run_acceptance(workspace, ("leak",), None)
    second = executor.run_acceptance(workspace, ("leak",), None)

    assert [r.outcome for r in first.results + second.results] == [P, P]


def test_the_host_workspace_is_never_modified(executor: WindowsVMExecutor, workspace: Path) -> None:
    before = snapshot(workspace)

    executor.run_acceptance(workspace, ("f1",), None)

    assert snapshot(workspace) == before


def snapshot(root: Path) -> dict[str, bytes]:
    return {str(p.relative_to(root)): p.read_bytes() for p in root.rglob("*") if p.is_file()}
