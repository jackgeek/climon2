"""The harness itself, on whichever platform runs it. No climon2 is involved."""

import json
import os
from pathlib import Path

import pytest
from harness import (
    TerminalTimeout,
    clean,
    controlled_env,
    controlled_shell,
    install_layout,
    pid_alive,
    required_shells,
    wait_until,
)
from harness.install import EXE
from harness.plugin import Spawn


def ready(term) -> dict:
    return json.loads(term.expect(r"FIXTURE READY \S+ (\{.*\})\n").group(1))


def test_the_fixture_runs_in_a_real_terminal_of_the_requested_size(
    terminal: Spawn, fixture_program: list[str]
) -> None:
    term = terminal(fixture_program, cols=100, rows=30)

    info = ready(term)

    assert info["tty"] == {"stdin": True, "stdout": True, "stderr": True}
    assert info["size"] == "100x30"
    assert pid_alive(info["pid"])
    term.send_line("ctty")
    assert term.expect(r"CTTY (\w+)").group(1) == "true"


def test_resizing_the_terminal_is_seen_by_the_program(
    terminal: Spawn, fixture_program: list[str]
) -> None:
    term = terminal(fixture_program, cols=80, rows=24)
    ready(term)

    term.resize(132, 43)
    term.send_line("size")

    assert term.expect(r"SIZE (\d+x\d+)").group(1) == "132x43"


def test_arguments_cwd_and_environment_arrive_exactly(
    terminal: Spawn, fixture_program: list[str], tmp_path: Path
) -> None:
    args = ["two words", "ünïcødé ✓", 'quo"te', "", "back\\slash"]
    cwd = tmp_path / "dir with space ü"
    cwd.mkdir()
    env = {**os.environ, "HARNESS_VALUE": "a b ü"}
    term = terminal([*fixture_program, *args], env=env, cwd=cwd)
    ready(term)

    term.send_line("argv")
    assert json.loads(term.expect(r"ARGV (.*)\n").group(1)) == args
    term.send_line("cwd")
    assert Path(json.loads(term.expect(r"CWD (.*)\n").group(1))).resolve() == cwd.resolve()
    term.send_line("env HARNESS_VALUE")
    assert json.loads(term.expect(r"ENV (.*)\n").group(1)) == {"HARNESS_VALUE": "a b ü"}


def test_the_exit_code_is_reported(terminal: Spawn, fixture_program: list[str]) -> None:
    term = terminal(fixture_program)
    ready(term)

    term.send_line("exit 42")

    assert term.wait() == 42
    assert not term.alive


def test_expect_fails_with_the_output_when_the_pattern_never_appears(
    terminal: Spawn, fixture_program: list[str]
) -> None:
    term = terminal(fixture_program)
    ready(term)
    term.send_line("echo something else")

    with pytest.raises(TerminalTimeout, match="something else"):
        term.expect("never printed", timeout=1)


def test_closing_kills_a_running_program(terminal: Spawn, fixture_program: list[str]) -> None:
    term = terminal(fixture_program)
    pid = ready(term)["pid"]

    term.close()

    assert not term.alive
    wait_until(lambda: not pid_alive(pid), f"fixture {pid} still alive", timeout=5)


# The terminal echoes what is typed, so each command prints text that was never typed.
COMPUTED_ECHO = {
    "bash": "echo HARNESS_$((6*7))",
    "sh": "echo HARNESS_$((6*7))",
    "zsh": "echo HARNESS_$((6*7))",
    "powershell.exe": "echo ('HARNESS_' + 6*7)",
    "pwsh.exe": "echo ('HARNESS_' + 6*7)",
    "cmd.exe": "echo HARNESS_%COMSPEC:~0,0%42",
}


@pytest.mark.parametrize("shell", required_shells())
def test_each_required_shell_runs_interactively_with_controlled_startup(
    terminal: Spawn, shell: str
) -> None:
    term = terminal(controlled_shell(shell), env=controlled_env(shell, dict(os.environ)))

    term.send_line(COMPUTED_ECHO[shell])
    term.expect(r"HARNESS_42")
    term.send_line("exit")

    assert term.wait() == 0


def test_the_install_layout_is_built_from_the_binaries(tmp_path: Path) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    for name in ("climon2", "climon2-cli", "climon2-daemon", "climon2-client"):
        (bin_dir / f"{name}{EXE}").write_text(name)

    install = install_layout(bin_dir, tmp_path / "in stall ü", version="1.2.3")

    assert install.launcher.read_text() == "climon2"
    assert install.current.read_text() == "1.2.3\n"
    assert (install.root / "versions/1.2.3" / f"climon2-daemon{EXE}").read_text() == (
        "climon2-daemon"
    )


def test_a_missing_binary_is_named(tmp_path: Path) -> None:
    with pytest.raises(AssertionError, match="climon2-client"):
        install_layout(tmp_path, tmp_path / "x")


def test_clean_turns_terminal_output_into_text() -> None:
    raw = "\x1b[?25l\x1b[2;1Hhello\x1b[3Cworld\r\n\x1b]0;title\x07done\x1b[0m"

    assert clean(raw) == "hello   world\ndone"
