import os
import pwd
from pathlib import Path

REAL_HOME = Path(pwd.getpwuid(os.getuid()).pw_dir)


def test_writes_into_the_users_home() -> None:
    (REAL_HOME / ".climon2-sandbox-canary").write_text("escaped")


def test_reads_the_users_home() -> None:
    os.listdir(REAL_HOME)
