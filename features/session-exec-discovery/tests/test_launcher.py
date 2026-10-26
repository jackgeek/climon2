import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import pytest
from harness import DEFAULT_TIMEOUT, install_layout, pid_alive, wait_until
from harness.install import EXE
from native import WINDOWS, terminate
from support import error, no_state, run


@pytest.mark.parametrize("version,ending", [("0.1.0", b""), ("0.1.0", b"\n"), ("0.1.0", b"\r\n"), ("1.2.3-rc.1+build.007", b"\n"), ("0.0.0-a-b.0+001", b"")], ids=["plain", "lf", "crlf", "prerelease-build", "semver-zero"])
def test_launcher_preserves_boundaries_environment_directory_and_handles(climon2, bin_dir, base_env, tmp_path, probe_binary, version, ending):
    """S1 (B1, B2): real-executable-relative selection and transparent forwarding."""
    install = install_layout(bin_dir, tmp_path / "package space \u03bb", version)
    shutil.copy2(probe_binary, install.companion("climon2-cli"))
    install.current.write_bytes(version.encode() + ending)
    cwd = tmp_path / "foreign cwd \u03bb"
    cwd.mkdir()
    (cwd / "current").write_text("9.9.9")
    args = ["", "two words", 'quo"te', "a'b", "back\\slash\\", "Unicode-\u03bb", "$(echo nope); & | > * %PATH%"]
    env = {**base_env, "PATH": str(install.root) + os.pathsep + base_env["PATH"], "FORWARD_MARKER": "space \u03bb", "PROBE_EXIT": "125"}
    executable = shutil.which("climon2" + EXE, path=env["PATH"])
    assert executable is not None, "adding the extracted install to PATH must find the launcher"
    child = subprocess.Popen(["climon2" + EXE, *args], executable=executable, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        out, err = child.communicate(b"input with spaces\n", timeout=DEFAULT_TIMEOUT)
    finally:
        if child.poll() is None:
            child.kill()
            child.wait(timeout=DEFAULT_TIMEOUT)
    assert child.returncode == 125, "launcher must preserve child status, including 125"
    lines = out.decode().splitlines()
    assert lines and lines[0].startswith("PROBE "), "selected runtime CLI must receive the invocation"
    report = json.loads(lines[0][6:].removesuffix(" PROBE_END"))
    assert report["argv"] == args, "launcher must preserve every argument boundary and value"
    assert report["env"] == env, "launcher must preserve the complete invoking environment"
    assert Path(report["cwd"]).resolve() == cwd.resolve()
    assert report["tty"] == [False, False, False]
    assert lines[1:] == ['PROBE_OUT "input with spaces"']
    assert err.decode().splitlines() == ['PROBE_ERR "input with spaces"']
    if not WINDOWS:
        assert report["pid"] == child.pid, "Unix CLI must replace launcher with the same PID"


@pytest.mark.parametrize("contents", [b"", b"\n", b"0.1.0\nextra", b" 0.1.0\n", b"0.1.0 \n", b"0.1.0\t", b"0.1.0\n\n", b"0.1.0\r", b"../0.1.0", b"/0.1.0", b"v0.1.0", b"1.2", b"01.2.3", b"1.2.3-01", b"1.2.3-", b"1.2.3+", b"1.2.3-a..b", b"1.2.3+bad!", b"\xff", b"0.1.0\x00"], ids=["empty", "blank", "multiline", "leading-space", "trailing-space", "tab", "extra-lf", "bare-cr", "traversal", "absolute-path", "v-prefix", "missing-patch", "leading-zero", "numeric-prerelease-zero", "empty-prerelease", "empty-build", "empty-identifier", "invalid-build-char", "invalid-utf8", "nul"])
def test_invalid_current_is_rejected_without_startup(climon2, base_env, contents):
    """S2 (B1, B3): current is strict SemVer, not a trimmed path or loose version."""
    climon2.current.write_bytes(contents)
    error(run(climon2, ["list"], base_env), "current is invalid", climon2.current)
    no_state(base_env, climon2)


@pytest.mark.parametrize("missing,fragment", [("current", "current is missing"), ("directory", "runtime directory is missing"), ("cli", "cannot launch")])
def test_missing_install_components_name_the_affected_path(climon2, base_env, missing, fragment):
    """S2 (B1, B3): missing current, runtime directory and CLI have precise errors."""
    if missing == "current":
        path = climon2.current
        path.unlink()
    elif missing == "directory":
        path = climon2.root / "versions" / "7.8.9"
        climon2.current.write_text("7.8.9\n")
    else:
        path = climon2.companion("climon2-cli")
        path.unlink()
    error(run(climon2, ["list"], base_env), fragment, path)
    no_state(base_env, climon2)


def test_unlaunchable_runtime_cli_is_a_climon2_error(climon2, base_env):
    """S2 (B1, B3): a selected CLI that exists but cannot run names its path."""
    target = climon2.companion("climon2-cli")
    target.write_bytes(b"not an executable\n")
    target.chmod(0o600)
    error(run(climon2, ["list"], base_env), "cannot launch", target)
    no_state(base_env, climon2)


@pytest.mark.parametrize("args", [[], ["unknown-subcommand"], ["list", "extra"], ["list", "--unknown"]])
def test_invalid_invocation_reports_usage_without_state(climon2, base_env, args):
    """S2, S5 (B3): invalid invocations return 125, not child/shell errors."""
    error(run(climon2, args, base_env), "usage:")
    no_state(base_env, climon2)


if not WINDOWS:
    def test_launcher_symlink_uses_current_beside_its_real_executable(climon2, base_env, tmp_path, probe_binary):
        """S1 (B1, B2): a symlink on PATH does not relocate the launcher install root."""
        directory = tmp_path / "links"
        directory.mkdir()
        (directory / "climon2").symlink_to(climon2.launcher)
        (directory / "current").write_text("9.9.9\n")
        shutil.copy2(probe_binary, climon2.companion("climon2-cli"))
        env = {**base_env, "PATH": str(directory) + os.pathsep + base_env["PATH"]}
        result = subprocess.run(["climon2", "", "symlink argument"], env=env, cwd=directory, input=b"", capture_output=True, timeout=DEFAULT_TIMEOUT)
        assert result.returncode == 37, "launcher symlink must resolve current beside the real executable"
        report = json.loads(result.stdout.decode().splitlines()[0][6:].removesuffix(" PROBE_END"))
        assert report["argv"] == ["", "symlink argument"]
        assert result.stderr == b""


@pytest.mark.parametrize("case", ["identity"] if not WINDOWS else ["console-and-job"])
def test_launcher_process_identity_and_console_lifetime(climon2, base_env, terminal, probe_binary, case):
    """S3 (B2, B6, B9): exec PID on Unix; shared console and kill-on-close on Windows."""
    shutil.copy2(probe_binary, climon2.companion("climon2-cli"))
    term = terminal([str(climon2.launcher)], env=base_env)
    report = json.loads(term.expect(r"PROBE (\{[\s\S]*?\}) PROBE_END\n").group(1).replace("\n", ""))
    if not WINDOWS:
        assert report["pid"] == term.pid, "Unix CLI must retain launcher PID"
        term.send_line("exit 37")
        assert term.wait() == 37
    else:
        assert term.pid in report["console"] and report["pid"] in report["console"], "Windows CLI must share launcher's console"
        assert report["pid"] in report["job"], "Windows CLI must be in a job"
        term.send("\x03")
        term.expect(f"PROBE_CTRL_C_{report['pid']}")
        assert term.alive and pid_alive(report["pid"]), "launcher must ignore Ctrl+C while CLI handles it"
        terminate(term.pid)
        wait_until(lambda: not pid_alive(report["pid"]), "closing launcher must terminate its CLI job")
