import os
import subprocess
from pathlib import Path


def climon2() -> str:
    return str(Path(os.environ["CLIMON2_BIN_DIR"]) / "climon2")


def test_greets() -> None:
    out = subprocess.run([climon2()], capture_output=True, text=True, check=True).stdout
    assert out == "hello from climon2 42\n", "wrong greeting"


def test_exits_cleanly() -> None:
    assert subprocess.run([climon2()], capture_output=True, check=False).returncode == 0
