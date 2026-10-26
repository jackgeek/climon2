import os
from pathlib import Path
import shutil
import subprocess
import tempfile

import pytest

from harness import DEFAULT_TIMEOUT
from native import WINDOWS, TRACKED_PIDS, descendants, make_private, processes, terminate


@pytest.fixture
def short_dir():
    # Leave room for hashed socket basenames, independent of the profile path length.
    parent = Path(tempfile.gettempdir()).anchor if WINDOWS else "/tmp"
    path = Path(tempfile.mkdtemp(prefix="c2-", dir=parent))
    try:
        make_private(path)
        yield path
    finally:
        shutil.rmtree(path)


@pytest.fixture
def base_env(base_env, short_dir, tmp_path):
    env = dict(base_env)
    profile = tmp_path / "isolated profile"
    profile.mkdir()
    env["USERPROFILE" if WINDOWS else "HOME"] = str(profile)
    if WINDOWS:
        env["LOCALAPPDATA"] = str(short_dir)
    else:
        env["XDG_RUNTIME_DIR"] = str(short_dir)
    env["PYTHONUTF8"] = "1"
    return env


@pytest.fixture(scope="session")
def probe_binary(tmp_path_factory):
    path = tmp_path_factory.mktemp("native-probe") / ("probe.exe" if WINDOWS else "probe")
    result = subprocess.run(["rustc", "--edition=2021", str(Path(__file__).with_name("probe.rs")), "-o", str(path)], capture_output=True, text=True, timeout=DEFAULT_TIMEOUT)
    assert result.returncode == 0, f"native test probe failed to compile: {result.stderr}"
    return path


@pytest.fixture(autouse=True)
def reap_test_processes(climon2, terminal, short_dir):
    before = set(processes())
    TRACKED_PIDS.clear()
    yield
    table = processes()
    owned = {pid for pid, info in table.items() if pid not in before and str(climon2.root).casefold() in info["exe"].casefold()}
    owned |= (TRACKED_PIDS & set(table)) - before
    for pid in tuple(owned):
        owned |= descendants(pid, table)
    for pid in sorted(owned, reverse=True):
        terminate(pid)
    from harness import wait_until
    wait_until(lambda: not (owned & set(processes())), "test-owned processes did not terminate during cleanup")
