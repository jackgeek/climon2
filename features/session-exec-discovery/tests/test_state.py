import json
import os
from pathlib import Path
import re
import subprocess
import sys
import threading

import pytest
from harness import DEFAULT_TIMEOUT, wait_until
from native import WINDOWS, assert_private, make_private
from support import (answer, assert_hello, cleaned, error, listed, no_state, ready,
                     record_for, run, validate_record)
from test_exec import redirected, terminal_error


class PublicationObserver:
    def __init__(self, directory):
        self.directory = directory
        self.stop = threading.Event()
        self.observed = set()
        self.failures = []
        self.thread = threading.Thread(target=self.observe, daemon=True)

    def observe(self):
        while not self.stop.is_set():
            try:
                for path in self.directory.glob("*"):
                    try:
                        if path.is_file():
                            assert_private(path, 0o600)
                            if path.suffix == ".pty":
                                value = json.loads(path.read_text(encoding="utf-8"))
                                assert isinstance(value, dict), "published metadata must be a JSON object"
                                from support import FIELDS
                                assert FIELDS <= value.keys(), "published metadata is incomplete"
                                if path.name not in self.observed:
                                    assert_hello(value)
                                self.observed.add(path.name)
                    except FileNotFoundError:
                        continue
            except (AssertionError, OSError, ValueError) as exc:
                self.failures.append(str(exc))
                return
            # Avoid continuously opening files being atomically renamed on Windows.
            self.stop.wait(0.02 if WINDOWS else 0.001)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.stop.set()
        self.thread.join(DEFAULT_TIMEOUT)
        assert not self.thread.is_alive(), "metadata observer did not stop"
        assert not self.failures, f"metadata must be atomic and private from first visibility: {self.failures}"


@pytest.mark.parametrize("home_mode", ["unset", "empty", "absolute", "relative", "override-missing-profile", "override-empty-profile"])
def test_concurrent_publication_is_atomic_private_and_uses_invoking_home(climon2, base_env, tmp_path, terminal, fixture_program, home_mode):
    """S12 (B3, B7, B11, B12): concurrent GUIDs, private atomic records, home precedence."""
    env = dict(base_env)
    cwd = tmp_path / "cwd"
    cwd.mkdir()
    profile_var = "USERPROFILE" if WINDOWS else "HOME"
    if home_mode in ("unset", "empty"):
        expected_home = Path(env[profile_var]) / ".climon2"
        env.pop("CLIMON2_HOME", None)
        if home_mode == "empty":
            env["CLIMON2_HOME"] = ""
    else:
        expected_home = tmp_path / "new state" / "home"
        if home_mode == "relative":
            env["CLIMON2_HOME"] = "../new state/home"
        else:
            env["CLIMON2_HOME"] = str(expected_home)
        if home_mode == "override-missing-profile":
            env.pop(profile_var, None)
        elif home_mode == "override-empty-profile":
            env[profile_var] = ""
    effective_env = {**env, "CLIMON2_HOME": str(expected_home)}
    assert not expected_home.exists()
    with PublicationObserver(expected_home / "sessions") as observer:
        terms = [terminal([str(climon2.launcher), "exec", *fixture_program, "--marker", f"parallel{i}"], env=env, cwd=cwd) for i in range(2)]
        infos = [ready(term) for term in terms]
        ids = [answer(term, "env CLIMON2_SESSION_ID", "ENV")["CLIMON2_SESSION_ID"] for term in terms]
        assert len(set(ids)) == 2, "simultaneous sessions must have distinct GUIDs"
        records = [record_for(effective_env, sid) for sid in ids]
        wait_until(lambda: len(observer.observed) == 2 or observer.failures, "publication observer did not see complete records")
        for i, (path, record) in enumerate(records):
            validate_record(path, record, climon2, fixture_program[0], [*fixture_program[1:], "--marker", f"parallel{i}"], cwd)
            assert record["daemon_pid"] not in {infos[0]["pid"], infos[1]["pid"], terms[0].pid, terms[1].pid}
            assert_hello(record)
        assert len({record["socket_path"] for _, record in records}) == 2
        assert len({record["daemon_pid"] for _, record in records}) == 2
        assert {r["session_id"] for r in listed(climon2, env, cwd)} == set(ids)
        table = run(climon2, ["list"], env, cwd)
        assert table.returncode == 0 and table.stderr == b""
        assert all(sid.encode() in table.stdout for sid in ids)
    for term, info, (path, record) in zip(terms, infos, records):
        term.send_line("exit 0")
        assert term.wait() == 0
        cleaned(path, record, info["pid"])
    assert listed(climon2, env, cwd) == []


@pytest.mark.parametrize("home_override", [None, ""])
@pytest.mark.parametrize("profile", [None, ""])
@pytest.mark.parametrize("invocation", ["exec", "list", "json"])
def test_missing_required_home_variable_has_no_fallback(climon2, base_env, tmp_path, terminal, fixture_program, home_override, profile, invocation):
    """S12 (B3, B11): missing/empty home variables fail explicitly before state creation."""
    env = dict(base_env)
    key = "USERPROFILE" if WINDOWS else "HOME"
    env.pop("CLIMON2_HOME", None)
    env.pop(key, None)
    if home_override is not None:
        env["CLIMON2_HOME"] = home_override
    if profile is not None:
        env[key] = profile
    fragment = f"cannot determine home directory: {key} is unset or empty"
    if invocation == "exec":
        err = tmp_path / "stderr"
        term = redirected(terminal, tmp_path, [climon2.launcher, "exec", *fixture_program], env, stderr=err)
        terminal_error(term, err, fragment)
    else:
        error(run(climon2, ["list"] + (["--json"] if invocation == "json" else []), env), fragment)
    no_state(base_env, climon2)


@pytest.mark.parametrize("mode", ["missing", "existing"] if WINDOWS else ["missing", "existing", "unset", "empty"])
def test_runtime_socket_location_ownership_and_authorized_peer(climon2, base_env, short_dir, terminal, fixture_program, mode):
    """S13, S14 (B13, B14, B15): native AF_UNIX in the prescribed private location."""
    env = dict(base_env)
    if WINDOWS:
        directory = short_dir / "climon2" / "run"
        if mode == "existing":
            directory.mkdir(parents=True)
            make_private(directory)
    elif mode in ("unset", "empty"):
        env.pop("XDG_RUNTIME_DIR", None)
        if mode == "empty":
            env["XDG_RUNTIME_DIR"] = ""
        directory = Path(f"/tmp/climon2-{os.getuid()}")
    else:
        directory = short_dir / "r"
        env["XDG_RUNTIME_DIR"] = str(directory)
        if mode == "existing":
            directory.mkdir(mode=0o700)
    term = terminal([str(climon2.launcher), "exec", *fixture_program], env=env)
    info = ready(term)
    path, record = record_for(env)
    socket_path = Path(record["socket_path"])
    assert socket_path.is_absolute() and socket_path.parent.resolve() == directory.resolve()
    if WINDOWS:
        assert re.fullmatch(r"[0-9a-f]+(?:\.sock)?", socket_path.name), "Windows socket basename must be lowercase hexadecimal hash"
    assert_private(directory, 0o700)
    assert_private(socket_path, 0o600)
    assert_hello(record)
    assert answer(term, "echo authorized-client-still-controls", "ECHO") == "authorized-client-still-controls"
    term.send_line("exit 0")
    assert term.wait() == 0
    cleaned(path, record, info["pid"])


def test_unsafe_runtime_directory_is_refused_without_repair_or_fallback(climon2, base_env, short_dir, tmp_path, terminal, fixture_program):
    """S13 (B3, B13): existing permissive runtime directory is never repaired or bypassed."""
    env = dict(base_env)
    directory = short_dir / "climon2" / "run" if WINDOWS else short_dir / "unsafe"
    directory.mkdir(parents=True)
    if WINDOWS:
        result = subprocess.run(["icacls", str(directory), "/grant", "*S-1-1-0:(F)"], capture_output=True, timeout=DEFAULT_TIMEOUT)
        assert result.returncode == 0
        from native import security
        before = security(directory)
    else:
        directory.chmod(0o755)
        before = directory.stat().st_mode
        env["XDG_RUNTIME_DIR"] = str(directory)
    err = tmp_path / "stderr"
    term = redirected(terminal, tmp_path, [climon2.launcher, "exec", *fixture_program], env, stderr=err)
    terminal_error(term, err, "unsafe runtime directory", directory)
    after = security(directory) if WINDOWS else directory.stat().st_mode
    assert before == after, "unsafe runtime permissions must not be silently repaired"
    no_state(env, climon2)


@pytest.mark.parametrize("case", ["long-socket", "missing-localappdata"] if WINDOWS else ["long-socket", "foreign-owner"])
def test_unusable_runtime_paths_fail_without_fallback(climon2, base_env, short_dir, tmp_path, terminal, fixture_program, case):
    """S13 (B3, B13): owner validation, missing LOCALAPPDATA and separate overlong socket case."""
    env = dict(base_env)
    target = None
    if case == "long-socket":
        directory = short_dir / ("x" * 100)
        directory.mkdir(mode=0o700)
        make_private(directory)
        if WINDOWS:
            env["LOCALAPPDATA"] = str(directory)
        else:
            env["XDG_RUNTIME_DIR"] = str(directory)
        fragment = "socket path"
    elif case == "foreign-owner":
        directory = Path("/var/root" if sys.platform == "darwin" else "/root")
        assert directory.stat().st_uid != os.getuid(), "owner-negative case requires the OS-owned root home"
        before = directory.stat()
        env["XDG_RUNTIME_DIR"] = str(directory)
        fragment, target = "unsafe runtime directory", directory
    else:
        env.pop("LOCALAPPDATA", None)
        fragment = "runtime directory"
    err = tmp_path / "stderr"
    term = redirected(terminal, tmp_path, [climon2.launcher, "exec", *fixture_program], env, stderr=err)
    terminal_error(term, err, fragment, target)
    if case == "foreign-owner":
        after = directory.stat()
        assert (before.st_uid, before.st_mode) == (after.st_uid, after.st_mode)
    elif case == "long-socket":
        assert not any(not path.is_dir() for path in directory.rglob("*")), "overlong socket failure must not leave socket files"
    no_state(base_env, climon2)
