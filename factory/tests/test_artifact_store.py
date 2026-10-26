from pathlib import Path

import pytest

from climon2_factory.artifacts import ApprovalRefused, ArtifactStore, State, UnsafePath


def write(repo: Path, rel: str, text: str) -> None:
    path = repo / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


@pytest.fixture
def repo(tmp_path: Path) -> Path:
    write(tmp_path, "features/f1/contract.md", "# Contract\n")
    return tmp_path


def test_recorded_artifact_is_unapproved_at_its_recorded_hash(repo: Path) -> None:
    store = ArtifactStore(repo)

    version = store.record("f1/contract", paths=["features/f1/contract.md"])

    status = store.status("f1/contract")
    assert status.state is State.UNAPPROVED
    assert status.hash == version.hash


def test_approving_current_hash_makes_artifact_approved(repo: Path) -> None:
    store = ArtifactStore(repo)
    version = store.record("f1/contract", paths=["features/f1/contract.md"])

    store.approve("f1/contract", version.hash)

    assert store.status("f1/contract").state is State.APPROVED


def test_approving_a_hash_that_is_not_current_is_refused(repo: Path) -> None:
    store = ArtifactStore(repo)
    store.record("f1/contract", paths=["features/f1/contract.md"])

    with pytest.raises(ApprovalRefused):
        store.approve("f1/contract", "sha256:" + "0" * 64)

    assert store.status("f1/contract").state is State.UNAPPROVED


def test_editing_an_approved_artifact_withdraws_its_approval(repo: Path) -> None:
    store = ArtifactStore(repo)
    version = store.record("f1/contract", paths=["features/f1/contract.md"])
    store.approve("f1/contract", version.hash)

    write(repo, "features/f1/contract.md", "# Contract (edited)\n")

    status = store.status("f1/contract")
    assert status.state is State.UNRECORDED
    assert status.hash != version.hash
    with pytest.raises(ApprovalRefused):
        store.approve("f1/contract", version.hash)


def test_editing_a_path_input_makes_the_artifact_stale(repo: Path) -> None:
    write(repo, "CONTEXT.md", "glossary v1\n")
    store = ArtifactStore(repo)
    store.record("f1/contract", paths=["features/f1/contract.md"], path_inputs=["CONTEXT.md"])

    write(repo, "CONTEXT.md", "glossary v2\n")

    status = store.status("f1/contract")
    assert status.state is State.STALE
    assert status.changed_inputs == ("path:CONTEXT.md",)


def test_editing_an_input_of_an_input_makes_the_artifact_stale(repo: Path) -> None:
    write(repo, "features/f1/brief.md", "brief v1\n")
    write(repo, "features/f1/tests/test_exec.py", "def test_exec(): ...\n")
    store = ArtifactStore(repo)
    store.record(
        "f1/contract", paths=["features/f1/contract.md"], path_inputs=["features/f1/brief.md"]
    )
    store.record("f1/tests", paths=["features/f1/tests"], artifact_inputs=["f1/contract"])

    write(repo, "features/f1/brief.md", "brief v2\n")

    status = store.status("f1/tests")
    assert status.state is State.STALE
    assert status.changed_inputs == ("artifact:f1/contract",)


def test_stale_artifact_cannot_be_approved(repo: Path) -> None:
    write(repo, "CONTEXT.md", "glossary v1\n")
    store = ArtifactStore(repo)
    version = store.record(
        "f1/contract", paths=["features/f1/contract.md"], path_inputs=["CONTEXT.md"]
    )
    write(repo, "CONTEXT.md", "glossary v2\n")

    with pytest.raises(ApprovalRefused):
        store.approve("f1/contract", version.hash)


def test_rejected_artifact_reports_rejection(repo: Path) -> None:
    store = ArtifactStore(repo)
    version = store.record("f1/contract", paths=["features/f1/contract.md"])

    store.reject("f1/contract", version.hash, reason="missing resize scenario")

    assert store.status("f1/contract").state is State.REJECTED


def test_re_recording_keeps_earlier_versions_and_decisions_in_history(repo: Path) -> None:
    store = ArtifactStore(repo)
    v1 = store.record("f1/contract", paths=["features/f1/contract.md"])
    store.reject("f1/contract", v1.hash, reason="too vague")
    write(repo, "features/f1/contract.md", "# Contract v2\n")
    v2 = store.record("f1/contract", paths=["features/f1/contract.md"])
    store.approve("f1/contract", v2.hash)

    history = store.history("f1/contract")

    assert [(v.hash, [(d.decision, d.reason) for d in v.decisions]) for v in history] == [
        (v1.hash, [("rejected", "too vague")]),
        (v2.hash, [("approved", None)]),
    ]
    assert store.status("f1/contract").state is State.APPROVED


def test_identical_content_hashes_identically_regardless_of_location_or_write_order(
    tmp_path: Path,
) -> None:
    a, b = tmp_path / "a", tmp_path / "b"
    write(a, "features/f1/tests/one.py", "1\n")
    write(a, "features/f1/tests/sub/two.py", "2\n")
    write(b, "features/f1/tests/sub/two.py", "2\n")
    write(b, "features/f1/tests/one.py", "1\n")

    hash_a = ArtifactStore(a).record("f1/tests", paths=["features/f1/tests"]).hash
    hash_b = ArtifactStore(b).record("f1/tests", paths=["features/f1/tests"]).hash

    assert hash_a == hash_b


def test_renaming_a_file_inside_an_artifact_changes_its_hash(repo: Path) -> None:
    write(repo, "features/f1/tests/one.py", "1\n")
    store = ArtifactStore(repo)
    version = store.record("f1/tests", paths=["features/f1/tests"])

    (repo / "features/f1/tests/one.py").rename(repo / "features/f1/tests/uno.py")

    status = store.status("f1/tests")
    assert status.state is State.UNRECORDED
    assert status.hash != version.hash


def test_moving_bytes_between_files_changes_the_hash(tmp_path: Path) -> None:
    a, b = tmp_path / "a", tmp_path / "b"
    write(a, "t/x", "ab")
    write(a, "t/y", "c")
    write(b, "t/x", "a")
    write(b, "t/y", "bc")

    assert (
        ArtifactStore(a).record("t", paths=["t"]).hash
        != ArtifactStore(b).record("t", paths=["t"]).hash
    )


def test_never_recorded_artifact_is_unrecorded_without_a_hash(repo: Path) -> None:
    status = ArtifactStore(repo).status("f1/contract")

    assert status.state is State.UNRECORDED
    assert status.hash is None


def test_deleting_a_recorded_artifact_makes_it_missing(repo: Path) -> None:
    store = ArtifactStore(repo)
    store.record("f1/contract", paths=["features/f1/contract.md"])

    (repo / "features/f1/contract.md").unlink()

    assert store.status("f1/contract").state is State.MISSING


def test_artifact_that_depends_on_a_missing_artifact_is_stale(repo: Path) -> None:
    write(repo, "features/f1/tests/t.py", "t\n")
    store = ArtifactStore(repo)
    store.record("f1/contract", paths=["features/f1/contract.md"])
    store.record("f1/tests", paths=["features/f1/tests"], artifact_inputs=["f1/contract"])

    (repo / "features/f1/contract.md").unlink()

    assert store.status("f1/tests").changed_inputs == ("artifact:f1/contract",)


@pytest.mark.parametrize("path", ["../outside.md", "/etc/hosts"])
def test_paths_outside_the_repo_are_refused(repo: Path, path: str) -> None:
    with pytest.raises(UnsafePath):
        ArtifactStore(repo).record("x", paths=[path])


def test_symlinks_inside_an_artifact_are_refused(
    repo: Path, tmp_path_factory: pytest.TempPathFactory
) -> None:
    outside = tmp_path_factory.mktemp("outside")
    write(outside, "secret.md", "secret\n")
    (repo / "features/f1/link.md").symlink_to(outside / "secret.md")

    with pytest.raises(UnsafePath):
        ArtifactStore(repo).record("f1/all", paths=["features/f1"])


def test_symlinked_directory_leading_outside_the_repo_is_refused(
    repo: Path, tmp_path_factory: pytest.TempPathFactory
) -> None:
    outside = tmp_path_factory.mktemp("outside")
    write(outside, "secret.md", "secret\n")
    (repo / "features/escape").symlink_to(outside, target_is_directory=True)

    with pytest.raises(UnsafePath):
        ArtifactStore(repo).record("x", paths=["features/escape/secret.md"])


@pytest.mark.parametrize("name", ["", "../x", "/abs", "a/../b", "a//b"])
def test_invalid_artifact_names_are_refused(repo: Path, name: str) -> None:
    with pytest.raises(ValueError):
        ArtifactStore(repo).record(name, paths=["features/f1/contract.md"])


def test_recording_against_an_input_that_is_not_current_is_refused(repo: Path) -> None:
    write(repo, "CONTEXT.md", "v1\n")
    write(repo, "features/f1/tests/t.py", "t\n")
    store = ArtifactStore(repo)
    store.record("f1/contract", paths=["features/f1/contract.md"], path_inputs=["CONTEXT.md"])
    write(repo, "CONTEXT.md", "v2\n")

    with pytest.raises(ValueError):
        store.record("f1/tests", paths=["features/f1/tests"], artifact_inputs=["f1/contract"])
    with pytest.raises(ValueError):
        store.record("f1/tests", paths=["features/f1/tests"], artifact_inputs=["never/recorded"])


def test_dependency_cycles_are_refused(repo: Path) -> None:
    write(repo, "features/f1/tests/t.py", "t\n")
    store = ArtifactStore(repo)
    store.record("f1/contract", paths=["features/f1/contract.md"])
    store.record("f1/tests", paths=["features/f1/tests"], artifact_inputs=["f1/contract"])

    with pytest.raises(ValueError):
        store.record("f1/contract", paths=["features/f1/contract.md"], artifact_inputs=["f1/tests"])
    with pytest.raises(ValueError):
        store.record(
            "f1/contract", paths=["features/f1/contract.md"], artifact_inputs=["f1/contract"]
        )
