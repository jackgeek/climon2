import json
import os
from pathlib import Path
import shutil

import pytest
from harness import pid_alive, wait_until
from harness.install import EXE
from native import WINDOWS, TRACKED_PIDS, console_processes, processes, terminate
from support import cleaned, no_state, record_for, wait_value
from test_exec import redirected, terminal_error
from test_lifecycle import finish_fixture, start_fixture, survivor_is_usable


def probe_report(term):
    report = json.loads(term.expect(r"PROBE (\{[\s\S]*?\}) PROBE_END\n").group(1).replace("\n", ""))
    TRACKED_PIDS.add(report["pid"])
    return report


@pytest.mark.parametrize("kind", ["bare", "relative", "absolute", "supplied-extension"] if WINDOWS else ["bare", "relative", "absolute"])
def test_command_resolution_preserves_path_order_and_supplied_filename(climon2, base_env, tmp_path, terminal, probe_binary, kind):
    """S5, S6 (B5, B11): PATH search is ordered; filenames resolve against invoking cwd."""
    first, second, cwd = [tmp_path / name for name in ("first", "second", "cwd")]
    for directory in (first, second, cwd):
        directory.mkdir()
        shutil.copy2(probe_binary, directory / ("probe" + EXE))
    env = {**base_env, "PATH": os.pathsep.join([str(first), str(second)]), "PATHEXT": ".EXE"}
    if kind == "bare":
        supplied, expected = "probe", first / ("probe" + EXE)
    elif kind == "supplied-extension":
        supplied, expected = "probe.exe", first / "probe.exe"
        env["PATHEXT"] = ".CMD"
    elif kind == "relative":
        supplied, expected = "./probe" + EXE, cwd / ("probe" + EXE)
    else:
        supplied, expected = str(second / ("probe" + EXE)), second / ("probe" + EXE)
    term = terminal([str(climon2.launcher), "exec", supplied, "--resume", ""], env=env, cwd=cwd)
    report = probe_report(term)
    assert Path(report["executable"]).resolve() == expected.resolve(), "bare names must honor PATH order, supplied paths must bypass PATH"
    assert report["argv"] == ["--resume", ""]
    assert Path(report["cwd"]).resolve() == cwd.resolve()
    path, record = record_for(env)
    assert record["command"] == supplied and record["args"] == ["--resume", ""]
    term.send_line("exit 0")
    assert term.wait() == 0
    cleaned(path, record, report["pid"])


def test_exec_relays_stdout_and_stderr_while_the_command_is_running(climon2, base_env, terminal, probe_binary):
    """S6 (B5, B6): startup output and both output streams arrive before command exit."""
    env = {**base_env, "PROBE_STARTUP_STDERR": "1"}
    term = terminal([str(climon2.launcher), "exec", str(probe_binary)], env=env)
    report = probe_report(term)
    term.expect("PROBE_STARTUP_ERROR_STREAM")
    expected_env = {**env, "CLIMON2_SESSION_ID": report["env"]["CLIMON2_SESSION_ID"], "CLIMON2_SESSION_DEPTH": "1"}
    expected_env.setdefault("TERM", "xterm-256color")
    assert report["env"] == expected_env, "command environment must be unchanged except session ID, depth and absent TERM"
    assert term.alive
    term.send_line("stream-check")
    wait_until(lambda: 'PROBE_OUT "stream-check"' in term.text and 'PROBE_ERR "stream-check"' in term.text,
               "both stdout and stderr must be relayed while command is running")
    path, record = record_for(env)
    term.send_line("exit 0")
    assert term.wait() == 0
    cleaned(path, record, report["pid"])


if WINDOWS:
    @pytest.mark.parametrize("pathext", [".EXE", ".COM", ""])
    def test_windows_has_no_implicit_cwd_search_or_extra_extensions(climon2, base_env, tmp_path, terminal, probe_binary, pathext):
        """S5 (B3, B5): CWD is not an implicit PATH entry and PATHEXT has no defaults."""
        cwd, path_dir = tmp_path / "cwd", tmp_path / "path"
        cwd.mkdir()
        path_dir.mkdir()
        shutil.copy2(probe_binary, cwd / "probe.exe")
        if pathext != ".EXE":
            shutil.copy2(probe_binary, path_dir / "probe.exe")
        env = {**base_env, "PATH": str(path_dir), "PATHEXT": pathext}
        err = tmp_path / "stderr"
        term = redirected(terminal, tmp_path, [climon2.launcher, "exec", "probe"], env, cwd=cwd, stderr=err)
        terminal_error(term, err, "cannot launch", "probe")
        no_state(env, climon2)


    @pytest.mark.parametrize("pathext,expected", [(".EXE;.CMD", "exe"), (".CMD;.EXE", "batch")])
    def test_windows_pathext_order_selects_exe_or_batch_in_each_path_directory(climon2, base_env, tmp_path, terminal, probe_binary, pathext, expected):
        """S5 (B5): PATHEXT ordering determines which same-directory target runs."""
        directory = tmp_path / "path"
        directory.mkdir()
        shutil.copy2(probe_binary, directory / "probe.exe")
        (directory / "probe.cmd").write_text('@echo BATCH_EXECUTED\n@"' + str(probe_binary) + '" %*\n', encoding="utf-8")
        env = {**base_env, "PATH": str(directory), "PATHEXT": pathext}
        term = terminal([str(climon2.launcher), "exec", "probe", "--resume"], env=env)
        report = probe_report(term)
        assert ("BATCH_EXECUTED" in term.text) == (expected == "batch"), "Windows command lookup must follow inherited PATHEXT order"
        expected_path = directory / "probe.exe" if expected == "exe" else probe_binary
        assert Path(report["executable"]).resolve() == expected_path.resolve()
        path, record = record_for(env)
        term.send_line("exit 0")
        assert term.wait() == 0
        cleaned(path, record, report["pid"])


    @pytest.mark.parametrize("extension", [".com", ".cmd", ".bat", ".BAT"])
    def test_windows_com_and_batch_targets_preserve_arguments_and_metadata(climon2, base_env, tmp_path, terminal, probe_binary, extension):
        """S6 (B3, B5, B6, B7, B11): direct COM and inherited-ComSpec CMD/BAT shims."""
        directory = tmp_path / "path"
        directory.mkdir()
        target = directory / ("shim" + extension)
        if extension == ".com":
            shutil.copy2(probe_binary, target)
        else:
            target.write_text('@echo BATCH_EXECUTED\n@"' + str(probe_binary) + '" %*\n', encoding="utf-8")
        comspec = shutil.which("cmd.exe")
        assert comspec, "Windows batch test requires cmd.exe"
        inherited_comspec = tmp_path / "chosen-comspec.exe"
        shutil.copy2(comspec, inherited_comspec)
        env = {**base_env, "PATH": str(directory), "PATHEXT": extension.upper(), "COMSPEC": str(inherited_comspec)}
        args = ["--resume", "argument with spaces", "Unicode-\u03bb"]
        supplied = str(target) if extension in (".com", ".BAT") else "shim"
        term = terminal([str(climon2.launcher), "exec", supplied, *args], env=env)
        report = probe_report(term)
        assert report["argv"] == args, "COM/CMD/BAT commands must receive exact argument boundaries and Unicode"
        assert report["tty"] == [True, True, True]
        path, record = record_for(env)
        assert record["command"] == supplied and record["args"] == args
        assert record["session_id"] == report["env"]["CLIMON2_SESSION_ID"]
        if extension != ".com":
            assert any(Path(info["exe"]).resolve() == inherited_comspec.resolve() for info in processes().values()), "batch targets must run through inherited ComSpec, not an implicit system shell"
        term.send_line("exit 0")
        assert term.wait() == 0
        cleaned(path, record, report["pid"])


    @pytest.mark.parametrize("extension", [".cmd", ".bat"])
    @pytest.mark.parametrize("unsafe", ["embedded\rreturn", "embedded\nnewline"])
    def test_windows_unsafe_batch_arguments_fail_without_executing_batch(climon2, base_env, tmp_path, terminal, probe_binary, extension, unsafe):
        """S6 (B3, B5): unsafe CR/LF arguments are refused before batch execution."""
        directory = tmp_path / "path"
        directory.mkdir()
        marker = tmp_path / "batch-ran"
        target = directory / ("shim" + extension)
        target.write_text('@echo ran>"' + str(marker) + '"\n@"' + str(probe_binary) + '" %*\n', encoding="utf-8")
        env = {**base_env, "PATH": str(directory), "PATHEXT": extension.upper(), "COMSPEC": shutil.which("cmd.exe")}
        err = tmp_path / "stderr"
        term = redirected(terminal, tmp_path, [climon2.launcher, "exec", "shim", unsafe], env, stderr=err)
        terminal_error(term, err, "cannot launch", target)
        assert not marker.exists(), "unsafe batch arguments must be rejected without executing the batch"
        no_state(env, climon2)


    def test_windows_launcher_job_excludes_daemon_and_command_and_closes_client(climon2, base_env, tmp_path, terminal, fixture_program, probe_binary):
        """S3 (B2, B6, B9): CLI/client share kill-on-close job; detached cleanup lives outside."""
        logs = tmp_path / "logs"
        logs.mkdir()
        for name in ("climon2-cli", "climon2-client"):
            original = climon2.companion(name)
            original.rename(original.with_name(name + ".real.exe"))
            shutil.copy2(probe_binary, original)
        env = {**base_env, "PROBE_LOG": str(logs)}
        survivor = start_fixture(climon2, env, terminal, fixture_program, "survivor")
        for log in logs.glob("*.json"):
            log.unlink()
        term = terminal([str(climon2.launcher), "exec", str(probe_binary)], env=env)
        command = probe_report(term)
        path, record = record_for(env, command["env"]["CLIMON2_SESSION_ID"])
        reports = {}
        for name in ("climon2-cli", "climon2-client"):
            log = wait_value(lambda: next(iter(logs.glob(name + "-*.json")), None), f"{name} did not launch")
            reports[name] = json.loads(log.read_text())
        cli, client = [reports[name] for name in ("climon2-cli", "climon2-client")]
        assert cli["pid"] in client["job"] and client["pid"] in client["job"], "CLI and controlling client must share launcher's job"
        assert record["daemon_pid"] not in client["job"] and command["pid"] not in client["job"], "daemon and command must be outside launcher's kill-on-close job"
        assert command["pid"] in command["job"] and cli["pid"] not in command["job"], "command requires its own separate job"
        daemon = processes()[record["daemon_pid"]]
        assert Path(daemon["exe"]).resolve() == climon2.companion("climon2-daemon").resolve(), "daemon_pid must identify the actual unwrapped session daemon"
        daemon_console = console_processes(record["daemon_pid"])
        assert all(pid not in daemon_console for pid in (term.pid, cli["pid"], client["pid"])), "Windows session daemon must be detached from invoking console"
        assert term.pid in cli["console"] and term.pid in client["console"]
        term.send("\x03")
        term.expect(f"PROBE_CTRL_C_{command['pid']}")
        assert term.alive
        terminate(term.pid)
        wait_until(lambda: not pid_alive(cli["pid"]) and not pid_alive(client["pid"]), "closing launcher must kill CLI/client job")
        cleaned(path, record, command["pid"])
        survivor_is_usable(climon2, base_env, survivor)
        finish_fixture(survivor)
