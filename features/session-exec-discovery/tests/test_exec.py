import contextlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import threading
import time

import pytest
from harness import DEFAULT_TIMEOUT, pid_alive, wait_until
from harness.install import EXE
from native import WINDOWS, TRACKED_PIDS, assert_private, processes
from support import (FIELDS, GUID, answer, assert_hello, cleaned, error, listed, no_state,
                     ready, record_for, run, validate_record, wait_value)


def redirected(terminal, tmp_path, argv, env, cwd=None, **streams):
    request = tmp_path / ("request-" + str(time.monotonic_ns()) + ".json")
    request.write_text(json.dumps({"argv": [str(a) for a in argv], **{k: str(v) for k, v in streams.items()}}), encoding="utf-8")
    return terminal([sys.executable, str(Path(__file__).with_name("stdio_driver.py")), str(request)], env=env, cwd=cwd)


def terminal_error(term, stderr, fragment, path=None):
    result = subprocess.CompletedProcess([], term.wait(), term.text.encode(), stderr.read_bytes())
    error(result, fragment, path)


def test_exec_relays_a_real_terminal_with_exact_process_context(climon2, base_env, tmp_path, terminal, fixture_program):
    """S6 (B3, B5, B6, B7, B11): genuine terminal, argv, environment, cwd and resize."""
    cwd = tmp_path / "working directory \u03bb"
    cwd.mkdir()
    args = ["", "two words", 'quote"', "a'b", "back\\slash", "Unicode-\u03bb", "--resume", "$(echo forbidden); & | > * %PATH%"]
    env = {**base_env, "UNCHANGED_VALUE": "a b \u03bb", "UNCHANGED_EMPTY": "", "TERM": "custom-term"}
    term = terminal([str(climon2.launcher), "exec", *fixture_program, "--marker", "context", *args], env=env, cwd=cwd, cols=103, rows=31)
    info = ready(term)
    assert info["tty"] == {"stdin": True, "stdout": True, "stderr": True}, "exec must give the command three terminal handles"
    assert info["size"] == "103x31", "command startup must see invoking terminal's initial dimensions"
    assert answer(term, "ctty", "CTTY") is True
    assert answer(term, "argv", "ARGV") == args
    assert Path(answer(term, "cwd", "CWD")).resolve() == cwd.resolve()
    for key in ("UNCHANGED_VALUE", "UNCHANGED_EMPTY", "TERM", "PATH"):
        assert answer(term, "env " + key, "ENV") == {key: env[key]}
    sid = answer(term, "env CLIMON2_SESSION_ID", "ENV")["CLIMON2_SESSION_ID"]
    assert GUID.fullmatch(sid)
    assert answer(term, "env CLIMON2_SESSION_DEPTH", "ENV") == {"CLIMON2_SESSION_DEPTH": "1"}
    path, record = record_for(env, sid)
    validate_record(path, record, climon2, fixture_program[0], [*fixture_program[1:], "--marker", "context", *args], cwd)
    assert record["daemon_pid"] not in (term.pid, info["pid"]), "session daemon must be a distinct process"
    assert answer(term, "echo interactive-before-exit", "ECHO") == "interactive-before-exit"
    term.resize(127, 39)
    def resized():
        term.send_line("size")
        return term.expect(r"SIZE (\d+x\d+)\n").group(1) == "127x39"
    wait_until(resized, "terminal resize did not reach command")
    assert_hello(record)
    assert [item["session_id"] for item in listed(climon2, env)] == [sid]
    term.send_line("exit 37")
    assert term.wait() == 37
    cleaned(path, record, info["pid"])


@pytest.mark.parametrize("kind", ["no-command", "missing", "not-executable"])
def test_failed_exec_startup_leaves_no_processes_or_state(climon2, base_env, tmp_path, terminal, kind):
    """S5 (B3, B5, B9): startup errors are diagnosed and rolled back."""
    err = tmp_path / "stderr"
    if kind == "no-command":
        args, fragment, target = [], "usage:", None
    elif kind == "missing":
        args, fragment, target = ["climon2-nonexistent-command-475cd"], "cannot launch", "climon2-nonexistent-command-475cd"
    else:
        target = tmp_path / ("not executable" + EXE)
        target.write_bytes(b"this is not an executable\n")
        target.chmod(0o600)
        args, fragment = [str(target)], "cannot launch"
    term = redirected(terminal, tmp_path, [climon2.launcher, "exec", *args], base_env, stderr=err)
    terminal_error(term, err, fragment, target)
    no_state(base_env, climon2)


@pytest.mark.parametrize("companion", ["climon2-daemon", "climon2-client"])
@pytest.mark.parametrize("defect", ["missing", "unlaunchable"])
def test_missing_companion_never_uses_path_or_cwd_decoys(climon2, base_env, tmp_path, terminal, fixture_program, probe_binary, companion, defect):
    """S4 (B3, B4): missing same-runtime companions fail without fallback or leaks."""
    decoys = tmp_path / "decoys"
    decoys.mkdir()
    for name in ("climon2-daemon", "climon2-client"):
        shutil.copy2(probe_binary, decoys / (name + EXE))
    pid_file = tmp_path / "startup-command.pid"
    env = {**base_env, "PATH": str(decoys) + os.pathsep + base_env["PATH"], "PROBE_PID_FILE": str(pid_file)}
    target = climon2.companion(companion)
    target.unlink()
    if defect == "unlaunchable":
        target.write_bytes(b"not an executable\n")
        target.chmod(0o600)
    err = tmp_path / "stderr"
    term = redirected(terminal, tmp_path, [climon2.launcher, "exec", probe_binary], env, cwd=decoys, stderr=err)
    terminal_error(term, err, "cannot launch", target)
    if pid_file.exists():
        command_pid = int(pid_file.read_text())
        TRACKED_PIDS.add(command_pid)
        wait_until(lambda: not pid_alive(command_pid), "failed startup must terminate any already-started command")
    no_state(env, climon2)


@pytest.mark.parametrize("through_symlink", [False] if WINDOWS else [False, True])
def test_companions_are_resolved_beside_real_cli_despite_current_switch(climon2, base_env, tmp_path, terminal, fixture_program, probe_binary, through_symlink):
    """S4 (B4): exact client launch interface and immutable runtime selection."""
    decoys, logs = tmp_path / "decoys", tmp_path / "logs"
    decoys.mkdir()
    logs.mkdir()
    for name in ("climon2-daemon", "climon2-client"):
        shutil.copy2(probe_binary, decoys / (name + EXE))
    # Windows cannot exec-replace a daemon probe; switch current at the selected CLI instead.
    switcher = "climon2-cli" if WINDOWS else "climon2-daemon"
    for name in (switcher, "climon2-client"):
        original = climon2.companion(name)
        original.rename(original.with_name(name + ".real" + EXE))
        shutil.copy2(probe_binary, original)
    env = {**base_env, "PATH": str(decoys) + os.pathsep + base_env["PATH"], "PROBE_LOG": str(logs), "PROBE_CURRENT": str(climon2.current)}
    cli = climon2.companion("climon2-cli")
    if through_symlink:
        link = decoys / "cli-link"
        link.symlink_to(cli)
        cli = link
    term = terminal([str(cli), "exec", *fixture_program], env=env, cwd=decoys)
    info = ready(term)
    path, record = record_for(env)
    client_log = wait_value(lambda: next(iter(logs.glob("climon2-client-*.json")), None), "same-runtime client did not launch")
    report = json.loads(client_log.read_text())
    assert len(report["argv"]) == 5 and report["argv"][:4] == ["--protocol", "1", "attach", "--metadata"], "client launch interface must preserve exact protocol-1 argument boundaries"
    client_metadata = Path(report["argv"][4])
    assert client_metadata.is_absolute() and client_metadata.resolve() == path.resolve(), "client must receive the session's absolute metadata path as one argument"
    assert len(list(logs.glob(switcher + "-*.json"))) == 1
    daemon = processes()[record["daemon_pid"]]
    expected_daemon = climon2.companion("climon2-daemon")
    if not WINDOWS:
        expected_daemon = expected_daemon.with_name("climon2-daemon.real" + EXE)
    assert Path(daemon["exe"]).resolve() == expected_daemon.resolve(), "daemon_pid must identify the daemon beside the real CLI, not a decoy or wrapper"
    assert climon2.current.read_text() == "9.9.9\n"
    assert record["climon2_version"] == climon2.version
    assert answer(term, "echo same-version", "ECHO") == "same-version"
    term.send_line("exit 0")
    assert term.wait() == 0
    cleaned(path, record, info["pid"])


@pytest.mark.parametrize("term_value", [None, "", "custom-256color"])
@pytest.mark.parametrize("inherited_id", [None, ""])
def test_top_level_depth_and_term_defaults_preserve_environment(climon2, base_env, terminal, fixture_program, tmp_path, term_value, inherited_id):
    """S7 (B7, B8): empty ID is top-level; TERM defaults only when absent."""
    env = {**base_env, "CLIMON2_SESSION_DEPTH": "4294967295", "OTHER_VALUE": "unchanged \u03bb"}
    if inherited_id is not None:
        env["CLIMON2_SESSION_ID"] = inherited_id
    env.pop("TERM", None)
    if term_value is not None:
        env["TERM"] = term_value
    err = tmp_path / "stderr"
    term = redirected(terminal, tmp_path, [climon2.launcher, "exec", *fixture_program], env, stderr=err)
    info = ready(term)
    assert answer(term, "env CLIMON2_SESSION_DEPTH", "ENV") == {"CLIMON2_SESSION_DEPTH": "1"}
    assert answer(term, "env TERM", "ENV") == {"TERM": "xterm-256color" if term_value is None else term_value}
    assert answer(term, "env OTHER_VALUE", "ENV") == {"OTHER_VALUE": "unchanged \u03bb"}
    path, record = record_for(env)
    assert record["depth"] == 1
    term.send_line("exit 0")
    assert term.wait() == 0
    assert err.read_bytes() == b"", "top-level exec must not print a nesting notice"
    cleaned(path, record, info["pid"])


@pytest.mark.parametrize("depth,new_depth,ordinal", [("1", 2, "1st"), ("2", 3, "2nd"), ("3", 4, "3rd"), ("10", 11, "10th"), ("11", 12, "11th"), ("12", 13, "12th"), ("13", 14, "13th"), ("20", 21, "20th"), ("21", 22, "21st"), ("00021", 22, "21st"), ("4294967294", 4294967295, "4294967294th"), (None, 2, "1st"), ("", 2, "1st"), ("\u0662", 2, "1st"), ("0", 2, "1st"), ("-1", 2, "1st"), ("abc", 2, "1st"), ("1.0", 2, "1st"), ("+2", 2, "1st"), (" 2", 2, "1st"), ("4294967296", 2, "1st")])
def test_nested_depth_validation_and_english_ordinals(climon2, base_env, terminal, fixture_program, tmp_path, depth, new_depth, ordinal):
    """S8 (B7, B8, B11): bounded decimal depths and exactly one ordinal notice."""
    env = {**base_env, "CLIMON2_SESSION_ID": "enclosing-session"}
    if depth is not None:
        env["CLIMON2_SESSION_DEPTH"] = depth
    err = tmp_path / "stderr"
    term = redirected(terminal, tmp_path, [climon2.launcher, "exec", *fixture_program], env, stderr=err)
    info = ready(term)
    sid = answer(term, "env CLIMON2_SESSION_ID", "ENV")["CLIMON2_SESSION_ID"]
    assert GUID.fullmatch(sid) and sid != env["CLIMON2_SESSION_ID"]
    assert answer(term, "env CLIMON2_SESSION_DEPTH", "ENV") == {"CLIMON2_SESSION_DEPTH": str(new_depth)}
    path, record = record_for(env, sid)
    assert record["depth"] == new_depth
    term.send_line("exit 0")
    assert term.wait() == 0
    assert err.read_text().splitlines() == [f"Starting climon2 session nested for the {ordinal} time"], "nested start must emit exactly one correct ordinal stderr line"
    cleaned(path, record, info["pid"])


def test_maximum_nested_depth_fails_before_startup(climon2, base_env, terminal, fixture_program, tmp_path):
    """S8 (B8): uint32 depth increment must not wrap or clamp."""
    env = {**base_env, "CLIMON2_SESSION_ID": "outer", "CLIMON2_SESSION_DEPTH": "4294967295"}
    err = tmp_path / "stderr"
    term = redirected(terminal, tmp_path, [climon2.launcher, "exec", *fixture_program], env, stderr=err)
    terminal_error(term, err, "session depth overflow")
    no_state(env, climon2)


def test_an_actual_enclosing_session_starts_a_distinct_inner_session(climon2, base_env, terminal, fixture_program, tmp_path):
    """S8 (B7, B8, B10, B11): actual nested exec agrees in environment, stderr and both records."""
    script = tmp_path / "nested.py"
    script.write_text("import json, os, subprocess, sys\n"
                      "print('OUTER ' + json.dumps({'id': os.environ['CLIMON2_SESSION_ID'], 'depth': os.environ['CLIMON2_SESSION_DEPTH']}), flush=True)\n"
                      "with open(sys.argv[1], 'wb') as stderr:\n"
                      " status = subprocess.call(sys.argv[2:], stderr=stderr)\n"
                      "sys.exit(status)\n")
    err = tmp_path / "stderr"
    inner_err = tmp_path / "inner-stderr"
    argv = [climon2.launcher, "exec", sys.executable, script, inner_err, climon2.launcher, "exec", *fixture_program]
    term = redirected(terminal, tmp_path, argv, base_env, stderr=err)
    outer = json.loads(term.expect(r"OUTER (\{[^\n]+\})\n").group(1))
    info = ready(term)
    inner = answer(term, "env CLIMON2_SESSION_ID", "ENV")["CLIMON2_SESSION_ID"]
    assert inner != outer["id"] and outer["depth"] == "1"
    assert answer(term, "env CLIMON2_SESSION_DEPTH", "ENV") == {"CLIMON2_SESSION_DEPTH": "2"}
    outer_path, outer_record = record_for(base_env, outer["id"])
    inner_path, inner_record = record_for(base_env, inner)
    assert outer_record["depth"] == 1 and inner_record["depth"] == 2
    assert {r["session_id"] for r in listed(climon2, base_env)} == {inner, outer["id"]}
    term.send_line("exit 0")
    assert term.wait() == 0
    notice = "Starting climon2 session nested for the 1st time"
    assert inner_err.read_text(encoding="utf-8").splitlines() == [notice], "inner exec must emit exactly one nesting notice on stderr"
    assert err.read_bytes() == b"", "outer top-level exec must not print a nesting notice"
    cleaned(inner_path, inner_record, info["pid"])
    cleaned(outer_path, outer_record)


@pytest.mark.parametrize("redirect", ["stdin", "stdout"])
def test_exec_rejects_each_nonterminal_required_handle(climon2, base_env, terminal, fixture_program, tmp_path, redirect):
    """S11 (B3, B10): either nonterminal stdin or stdout prevents session startup."""
    err, redirected_file = tmp_path / "stderr", tmp_path / "redirected"
    redirected_file.write_bytes(b"")
    term = redirected(terminal, tmp_path, [climon2.launcher, "exec", *fixture_program], base_env, stderr=err, **{redirect: "PIPE" if redirect == "stdin" else redirected_file})
    terminal_error(term, err, "exec requires a terminal")
    assert redirected_file.read_bytes() == b""
    no_state(base_env, climon2)


def test_stderr_may_be_redirected_and_list_needs_no_terminal(climon2, base_env, terminal, fixture_program, tmp_path):
    """S11 (B10): stderr is optional as a terminal; discovery uses redirected stdio."""
    err = tmp_path / "stderr"
    term = redirected(terminal, tmp_path, [climon2.launcher, "exec", *fixture_program], base_env, stderr=err)
    info = ready(term)
    assert info["tty"] == {"stdin": True, "stdout": True, "stderr": True}
    path, record = record_for(base_env)
    for args in (["list"], ["list", "--json"]):
        result = run(climon2, args, base_env)
        assert result.returncode == 0 and record["session_id"].encode() in result.stdout
        assert result.stderr == b""
    term.send_line("exit 0")
    assert term.wait() == 0 and err.read_bytes() == b""
    cleaned(path, record, info["pid"])


@pytest.mark.parametrize("status", [0, 37, 125])
def test_immediate_exit_drains_final_output_and_preserves_status(climon2, base_env, terminal, probe_binary, status):
    """S5, S9 (B5, B6, B9): exit during initial attachment still drains output."""
    env = {**base_env, "PROBE_MODE": "immediate", "PROBE_EXIT": str(status), "PROBE_MARKER": "attached-too-late"}
    term = terminal([str(climon2.launcher), "exec", str(probe_binary)], env=env)
    assert term.wait() == status, "exec must return the immediate command's exact exit status"
    assert "FINAL_DRAIN_attached-too-late" in term.text, "exec must drain output even when command exits during attachment"
    no_state(env, climon2)
