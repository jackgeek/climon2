import json
import os
from pathlib import Path
import re
import shlex
import signal
import subprocess
import sys

import pytest
from harness import (controlled_env, controlled_shell, pid_alive, required_shells,
                     wait_until)
from native import WINDOWS, descendants, processes, terminate
from support import answer, cleaned, listed, ready, record_for, run, wait_value


def start_fixture(climon2, env, terminal, fixture_program, marker):
    term = terminal([str(climon2.launcher), "exec", *fixture_program, "--marker", marker], env=env)
    info = ready(term)
    sid = answer(term, "env CLIMON2_SESSION_ID", "ENV")["CLIMON2_SESSION_ID"]
    path, record = record_for(env, sid)
    return term, info, path, record


def survivor_is_usable(climon2, env, survivor):
    term, _, _, record = survivor
    assert answer(term, "echo independent-session", "ECHO") == "independent-session"
    assert record["session_id"] in {r["session_id"] for r in listed(climon2, env)}, "ending one session must not disturb another"
    table = run(climon2, ["list"], env)
    assert table.returncode == 0 and record["session_id"].encode() in table.stdout


def finish_fixture(session):
    term, info, path, record = session
    term.send_line("exit 0")
    assert term.wait() == 0
    cleaned(path, record, info["pid"])


def outer_shell(terminal, env):
    name = "cmd.exe" if WINDOWS else "bash"
    argv = controlled_shell(name)
    if WINDOWS:
        argv.append("/v:on")
    term = terminal(argv, env=controlled_env(name, env), cols=180, rows=40)
    if WINDOWS:
        term.send_line("echo OUTER_%COMSPEC:~0,0%READY")
        term.expect(r"(?:^|\n)OUTER_READY\n")
        before = None
    else:
        term.send_line("printf 'MODES_BEFORE_%s\\n' \"$(stty -g)\"")
        before = term.expect(r"(?:^|\n)MODES_BEFORE_([^\n]+)\n").group(1)
    return term, before


def invoke_in_outer(term, argv, stderr):
    if WINDOWS:
        command = subprocess.list2cmdline([str(arg) for arg in argv])
        term.send_line(command + " 2>" + subprocess.list2cmdline([str(stderr)]) + " & echo EXEC_STATUS_!errorlevel!")
    else:
        command = shlex.join([str(arg) for arg in argv])
        term.send_line(command + " 2>" + shlex.quote(str(stderr)) + "; __c2=$?; printf '\\nEXEC_STATUS_%s\\n' \"$__c2\"; printf 'MODES_AFTER_%s\\n' \"$(stty -g)\"")


def assert_restored(term, before, status):
    actual = int(term.expect(r"(?:^|\n)EXEC_STATUS_(\d+)\n").group(1))
    assert actual == status, "outer shell must receive the command/client-loss exit status unchanged"
    if WINDOWS:
        line = "echo RESTORED_%COMSPEC:~0,0%42"
        term.send(line)
        term.expect(re.escape(line))
        term.send("\r")
        term.expect(r"(?:^|\n)RESTORED_42\n")
    else:
        after = term.expect(r"(?:^|\n)MODES_AFTER_([^\n]+)\n").group(1)
        assert after == before, "exec must restore exact stty modes before the next outer-shell command"
    term.send_line("exit 0")
    assert term.wait() == 0


def group_fixture(tmp_path, fixture_program):
    script = tmp_path / "command-with-descendant.py"
    script.write_text("import os, runpy, subprocess, sys\n"
                      "child = subprocess.Popen([sys.executable, '-c', 'import threading; threading.Event().wait(120)'])\n"
                      "print('DESCENDANT_PID_' + str(child.pid), flush=True)\n"
                      "sys.argv = sys.argv[1:]\n"
                      "runpy.run_path(sys.argv[0], run_name='__main__')\n")
    return [sys.executable, str(script), *fixture_program[1:]]


@pytest.mark.parametrize("status", [0, 37, 125])
def test_command_exit_drains_output_restores_outer_terminal_and_cleans_group(climon2, base_env, terminal, fixture_program, tmp_path, status):
    """S9 (B6, B9): exact child status, terminal restoration, descendant cleanup and isolation."""
    survivor = start_fixture(climon2, base_env, terminal, fixture_program, "survivor")
    outer, before = outer_shell(terminal, base_env)
    err = tmp_path / "stderr"
    invoke_in_outer(outer, [climon2.launcher, "exec", *group_fixture(tmp_path, fixture_program)], err)
    descendant = int(outer.expect(r"DESCENDANT_PID_(\d+)\n").group(1))
    info = ready(outer)
    sid = answer(outer, "env CLIMON2_SESSION_ID", "ENV")["CLIMON2_SESSION_ID"]
    path, record = record_for(base_env, sid)
    assert answer(outer, "echo final-output-before-exit", "ECHO") == "final-output-before-exit"
    outer.send_line(f"exit {status}")
    assert_restored(outer, before, status)
    assert err.read_bytes() == b"", "normal command status, including 125, must not produce climon2 errors"
    cleaned(path, record, info["pid"], [descendant])
    survivor_is_usable(climon2, base_env, survivor)
    finish_fixture(survivor)


if not WINDOWS:
    @pytest.mark.parametrize("event", ["sigterm", "terminal-ctrl-c"])
    def test_signal_status_and_terminal_ctrl_c_restore_outer_modes(climon2, base_env, terminal, fixture_program, tmp_path, event):
        """S9 (B6, B9): Unix terminal-byte SIGINT and signal exit status are not client loss."""
        survivor = start_fixture(climon2, base_env, terminal, fixture_program, "survivor")
        outer, before = outer_shell(terminal, base_env)
        script = tmp_path / "signal-command.py"
        script.write_text("import json, os, signal, sys, time\n"
                          "def interrupt(signum, frame):\n print('INTERRUPT_RECEIVED', flush=True)\n sys.exit(130)\n"
                          "signal.signal(signal.SIGINT, interrupt)\n"
                          "print('SIGNAL_READY ' + json.dumps({'pid': os.getpid(), 'sid': os.environ['CLIMON2_SESSION_ID']}), flush=True)\n"
                          "deadline = time.monotonic() + 120\n"
                          "while time.monotonic() < deadline:\n time.sleep(0.05)\n")
        err = tmp_path / "stderr"
        invoke_in_outer(outer, [climon2.launcher, "exec", sys.executable, script], err)
        report = json.loads(outer.expect(r"SIGNAL_READY (\{[^\n]+\})\n").group(1))
        path, record = record_for(base_env, report["sid"])
        if event == "sigterm":
            os.kill(report["pid"], signal.SIGTERM)
            status = 143
        else:
            outer.send("\x03")
            outer.expect("INTERRUPT_RECEIVED")
            status = 130
        assert_restored(outer, before, status)
        assert err.read_bytes() == b""
        cleaned(path, record, report["pid"])
        survivor_is_usable(climon2, base_env, survivor)
        finish_fixture(survivor)


def test_killed_controlling_client_reports_loss_and_restores_outer_terminal(climon2, base_env, terminal, fixture_program, tmp_path):
    """S10 (B6, B9): unexpected client death is handled failure, not detach or success."""
    survivor = start_fixture(climon2, base_env, terminal, fixture_program, "survivor")
    outer, before = outer_shell(terminal, base_env)
    err = tmp_path / "stderr"
    invoke_in_outer(outer, [climon2.launcher, "exec", *group_fixture(tmp_path, fixture_program)], err)
    descendant = int(outer.expect(r"DESCENDANT_PID_(\d+)\n").group(1))
    info = ready(outer)
    sid = answer(outer, "env CLIMON2_SESSION_ID", "ENV")["CLIMON2_SESSION_ID"]
    path, record = record_for(base_env, sid)
    def client_pid():
        table = processes()
        clients = [pid for pid in descendants(outer.pid, table) if "climon2-client" in Path(table[pid]["exe"]).name]
        assert len(clients) <= 1, "one session must have exactly one controlling client"
        return clients[0] if clients else None
    client = wait_value(client_pid, "controlling client process was not found")
    assert record["daemon_pid"] not in (client, info["pid"])
    terminate(client)
    assert_restored(outer, before, 125)
    diagnostic = err.read_text()
    assert diagnostic.startswith("climon2: ") and "client disconnected" in diagnostic, "unexpected client death must report climon2: client disconnected"
    cleaned(path, record, info["pid"], [client, descendant])
    survivor_is_usable(climon2, base_env, survivor)
    finish_fixture(survivor)


def test_losing_invoking_terminal_ends_only_its_session_and_process_group(climon2, base_env, terminal, fixture_program, tmp_path):
    """S10 (B6, B9): terminal loss still permits detached daemon cleanup."""
    survivor = start_fixture(climon2, base_env, terminal, fixture_program, "survivor")
    term = terminal([str(climon2.launcher), "exec", *group_fixture(tmp_path, fixture_program)], env=base_env)
    descendant = int(term.expect(r"DESCENDANT_PID_(\d+)\n").group(1))
    info = ready(term)
    sid = answer(term, "env CLIMON2_SESSION_ID", "ENV")["CLIMON2_SESSION_ID"]
    path, record = record_for(base_env, sid)
    term.close()
    cleaned(path, record, info["pid"], [descendant])
    survivor_is_usable(climon2, base_env, survivor)
    finish_fixture(survivor)


COMPUTED = {"bash": "echo SHELL_$((6*7))", "sh": "echo SHELL_$((6*7))", "zsh": "echo SHELL_$((6*7))",
            "powershell.exe": "echo ('SHELL_' + 6*7)", "pwsh.exe": "echo ('SHELL_' + 6*7)",
            "cmd.exe": "echo SHELL_%COMSPEC:~0,0%42"}


@pytest.mark.parametrize("shell", required_shells())
def test_required_interactive_shell_is_discoverable_and_survives_terminal_ctrl_c(climon2, base_env, terminal, shell):
    """S20 (B5, B6, B9, B16): every required real shell supports interaction and discovery."""
    prompt_env = {"PROMPT": "C2_PROMPT$G"} if WINDOWS else {"PS1": "C2_PROMPT> "}
    env = controlled_env(shell, {**base_env, **prompt_env})
    term = terminal([str(climon2.launcher), "exec", *controlled_shell(shell)], env=env)
    if shell != "cmd.exe":
        term.expect(r"PS [^\n]*>" if shell in ("powershell.exe", "pwsh.exe") else r"C2_PROMPT>")
    term.send_line(COMPUTED[shell])
    term.expect(r"(?:^|\n)SHELL_42\n")
    path, record = record_for(env)
    for _ in range(2 if not WINDOWS else 1):
        assert [r["session_id"] for r in listed(climon2, env)] == [record["session_id"]]
        table = run(climon2, ["list"], env)
        assert table.returncode == 0 and record["session_id"].encode() in table.stdout
        if not WINDOWS:
            term.expect(r"C2_PROMPT> ")
            term.send("\x03")
            term.expect(r"C2_PROMPT> ")
            term.send_line("echo AFTER_$((20+3))")
            term.expect(r"(?:^|\n)AFTER_23\n")
            assert term.alive
    term.send_line("exit 37")
    assert term.wait() == 37
    cleaned(path, record)
    assert listed(climon2, env) == []
    result = run(climon2, ["list"], env)
    assert result.returncode == 0 and result.stdout.replace(b"\r\n", b"\n") == b"No climon2 sessions running.\n"
