import os
import subprocess
import sys
from pathlib import Path


def climon2() -> str:
    name = "climon2.exe" if sys.platform == "win32" else "climon2"
    return str(Path(os.environ["CLIMON2_BIN_DIR"]) / name)


def test_greets() -> None:
    out = subprocess.run([climon2()], capture_output=True, text=True, check=True).stdout
    assert out == "hello from climon2 42\n", "wrong greeting"


def test_exits_cleanly() -> None:
    assert subprocess.run([climon2()], capture_output=True, check=False).returncode == 0
