"""The installed `factory` command, wired to real dependencies."""

import os
import subprocess
import sys
from pathlib import Path

FACTORY = Path(sys.executable).parent / "factory"


def factory(cwd: Path, home: Path, *args: str) -> subprocess.CompletedProcess[str]:
    env = {"PATH": "/usr/bin:/bin", "HOME": str(home)}
    return subprocess.run(
        [str(FACTORY), *args], cwd=cwd, env=env, capture_output=True, text=True, check=False
    )


def test_status_works_from_a_subdirectory_without_starting_docker(tmp_path: Path) -> None:
    repo, home = tmp_path / "repo", tmp_path / "home"
    (repo / "features").mkdir(parents=True)
    home.mkdir()
    subprocess.run(["git", "init", "-q", str(repo)], check=True)

    result = factory(repo / "features", home, "status")

    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == "No active feature run."
    assert not (home / ".climon2-factory" / "toolchain").exists()


def test_the_factory_refuses_to_run_outside_a_git_repository(tmp_path: Path) -> None:
    result = factory(tmp_path, tmp_path, "status")

    assert result.returncode == 2
    assert "not inside a git repository" in result.stderr


def test_start_without_a_brief_is_refused_by_the_real_command(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    subprocess.run(["git", "init", "-q", str(repo)], check=True)

    result = factory(repo, tmp_path, "start", "nope")

    assert result.returncode == 2
    assert "features/nope/brief.md not found" in result.stderr
    runs = tmp_path / ".climon2-factory" / "runs"
    assert not runs.exists() or os.listdir(runs) == []
