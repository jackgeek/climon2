import os
import subprocess


def test_lists_its_own_process() -> None:
    output = subprocess.run(
        ["ps", "-axo", "pid="], capture_output=True, text=True, check=True
    ).stdout
    assert str(os.getpid()) in output.split()
