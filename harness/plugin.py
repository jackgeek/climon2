"""pytest fixtures for climon2 acceptance tests. Loaded by the repository's root conftest.py."""

import os
import sys
from collections.abc import Callable, Iterator, Mapping, Sequence
from pathlib import Path

import pytest

from harness import FIXTURE_PROGRAM
from harness.install import Install, install_layout
from harness.terminal import Terminal, spawn

Spawn = Callable[..., Terminal]


@pytest.fixture
def bin_dir() -> Path:
    """The directory holding the climon2 binaries under test (`$CLIMON2_BIN_DIR`)."""
    value = os.environ.get("CLIMON2_BIN_DIR")
    if not value:
        pytest.fail("CLIMON2_BIN_DIR is not set")
    return Path(value)


@pytest.fixture
def climon2(bin_dir: Path, tmp_path: Path) -> Install:
    """A fresh install layout in a path with a space and non-ASCII characters."""
    return install_layout(bin_dir, tmp_path / "climon2 install ü")


@pytest.fixture
def climon2_home(tmp_path: Path) -> Path:
    """An empty climon2 home for this test (pass it as CLIMON2_HOME)."""
    home = tmp_path / "home ü" / ".climon2"
    home.mkdir(parents=True)
    return home


@pytest.fixture
def base_env(climon2: Install, climon2_home: Path) -> dict[str, str]:
    """The test environment: this process's, minus inherited CLIMON2_* variables, with the
    install first on PATH and CLIMON2_HOME set to the test's home."""
    env = {k: v for k, v in os.environ.items() if not k.upper().startswith("CLIMON2_")}
    env["PATH"] = str(climon2.root) + os.pathsep + env.get("PATH", "")
    env["CLIMON2_HOME"] = str(climon2_home)
    return env


@pytest.fixture
def terminal() -> Iterator[Spawn]:
    """`terminal(argv, env=None, cwd=None, cols=80, rows=24)` starts `argv` in a real terminal.
    Every terminal is closed (its process killed if still running) after the test."""
    opened: list[Terminal] = []

    def start(
        argv: Sequence[str],
        *,
        env: Mapping[str, str] | None = None,
        cwd: "str | os.PathLike[str] | None" = None,
        cols: int = 80,
        rows: int = 24,
    ) -> Terminal:
        term = spawn(argv, env=env, cwd=cwd, cols=cols, rows=rows)
        opened.append(term)
        return term

    yield start
    for term in opened:
        term.close()


@pytest.fixture
def fixture_program() -> list[str]:
    """argv prefix that runs the generic terminal fixture program with this Python."""
    return [sys.executable, str(FIXTURE_PROGRAM)]
