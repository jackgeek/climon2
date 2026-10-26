"""Acceptance-test harness for climon2: real terminals, a terminal fixture program, shells and
the install layout. See harness/README.md."""

from pathlib import Path

from harness.install import Install, install_layout
from harness.shells import REQUIRED_SHELLS, controlled_env, controlled_shell, required_shells
from harness.terminal import DEFAULT_TIMEOUT, Terminal, TerminalTimeout, clean, spawn
from harness.waiting import pid_alive, wait_until

FIXTURE_PROGRAM = Path(__file__).with_name("fixture_program.py")

__all__ = [
    "DEFAULT_TIMEOUT",
    "FIXTURE_PROGRAM",
    "REQUIRED_SHELLS",
    "Install",
    "Terminal",
    "TerminalTimeout",
    "clean",
    "controlled_env",
    "controlled_shell",
    "install_layout",
    "pid_alive",
    "required_shells",
    "spawn",
    "wait_until",
]
