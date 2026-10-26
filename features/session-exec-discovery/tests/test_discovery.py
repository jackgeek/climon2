import contextlib
import datetime as dt
import json
import os
from pathlib import Path
import re
import subprocess
import threading
import time
import uuid

import pytest
from harness import DEFAULT_TIMEOUT, wait_until
from native import WINDOWS, make_private
from support import FIELDS, OBSERVATION_GRACE, endpoint, error, listed, run, synthetic


def empty_output(json_mode):
    return b"[]\n" if json_mode else b"No climon2 sessions running.\n"


def listing_args(json_mode):
    return ["list", "--json"] if json_mode else ["list"]


@pytest.mark.parametrize("json_mode", [False, True])
@pytest.mark.parametrize("missing", ["sessions", "home"])
def test_missing_state_directories_list_as_empty(climon2, base_env, climon2_home, json_mode, missing):
    """S17, S18, S19 (B16, B17, B19): empty listing has exact output and no heading."""
    if missing == "home":
        climon2_home.rmdir()
    result = run(climon2, listing_args(json_mode), base_env)
    assert result.returncode == 0, "listing absent state directories must succeed"
    assert result.stdout == empty_output(json_mode), "empty listing must have the exact specified output"
    assert result.stderr == b""


@pytest.mark.parametrize("json_mode", [False, True])
def test_live_pid_without_matching_hello_is_not_a_live_session(climon2, base_env, climon2_home, short_dir, json_mode):
    """S17 (B11, B16): an existing unrelated PID never substitutes for a live hello."""
    with endpoint(short_dir, mode="mismatch") as ep:
        path, _ = synthetic(climon2_home, ep)
        before = path.read_bytes()
        result = run(climon2, listing_args(json_mode), base_env)
        assert result.returncode == 0, "stale-record discovery must succeed"
        assert result.stdout == empty_output(json_mode), "a live PID without matching hello must be hidden"
        assert result.stderr == b""
        assert path.read_bytes() == before, "stale records must remain byte-for-byte intact"
        assert ep.path.exists(), "discovery must not delete stale sockets"


@pytest.mark.parametrize("json_mode", [False, True])
def test_filename_guid_mismatch_is_silently_stale_even_with_matching_socket(climon2, base_env, climon2_home, short_dir, json_mode):
    """S17 (B16): record/socket agreement is insufficient when filename GUID differs."""
    with endpoint(short_dir) as ep:
        path, _ = synthetic(climon2_home, ep)
        wrong_name = path.with_name(str(uuid.uuid4()) + ".pty")
        path.rename(wrong_name)
        before = wrong_name.read_bytes()
        result = run(climon2, listing_args(json_mode), base_env)
        assert result.returncode == 0
        assert result.stdout == empty_output(json_mode), "filename GUID mismatch must be hidden even if hello matches record"
        assert result.stderr == b"", "filename GUID mismatch is stale, not malformed metadata"
        assert wrong_name.read_bytes() == before and ep.path.exists()


@pytest.mark.parametrize("mode", ["silent", "partial", "drip", "protocol", "malformed", "mismatch"])
@pytest.mark.parametrize("json_mode", [False, True])
def test_discovery_uses_one_bounded_handshake_and_preserves_stale_artifacts(climon2, base_env, climon2_home, short_dir, mode, json_mode):
    """S16, S17 (B16): silent/partial/invalid/mismatched endpoints are bounded and stale."""
    with endpoint(short_dir, mode=mode) as ep:
        path, _ = synthetic(climon2_home, ep)
        before = path.read_bytes()
        child = subprocess.Popen([str(climon2.launcher), *listing_args(json_mode)], env=base_env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            # Startup is not part of the per-connection deadline.
            deadline = time.monotonic() + DEFAULT_TIMEOUT
            while not ep.accepted.is_set() and child.poll() is None and time.monotonic() < deadline:
                ep.accepted.wait(0.01)
            if ep.accepted.is_set():
                started = time.monotonic()
                try:
                    out, err = child.communicate(timeout=2 + OBSERVATION_GRACE)
                except subprocess.TimeoutExpired:
                    pytest.fail("discovery handshake must finish within one two-second deadline")
                assert time.monotonic() - started <= 2 + OBSERVATION_GRACE, "discovery handshake must finish within one two-second deadline"
            else:
                out, err = child.communicate(timeout=DEFAULT_TIMEOUT)
            assert child.returncode == 0, f"discovery must succeed while hiding stale records: {err!r}"
            assert ep.accepted.is_set(), "discovery must attempt a hello rather than trust PID or metadata"
            assert out == empty_output(json_mode), "failed handshake must not be listed as live"
            assert err == b""
            assert path.read_bytes() == before and ep.path.exists()
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=DEFAULT_TIMEOUT)


@pytest.mark.parametrize("json_mode", [False, True])
def test_discovery_lists_valid_reply_delayed_by_one_point_two_seconds(climon2, base_env, climon2_home, short_dir, json_mode):
    """S16, S17 (B16): a valid reply delayed 1.2 seconds still qualifies as live."""
    with endpoint(short_dir, mode="delayed") as ep:
        path, record = synthetic(climon2_home, ep)
        before = path.read_bytes()
        result = run(climon2, listing_args(json_mode), base_env)
        assert result.returncode == 0, "discovery must succeed with a valid delayed hello"
        assert ep.accepted.is_set(), "discovery must probe the delayed endpoint"
        if json_mode:
            assert [{k: obj[k] for k in FIELDS} for obj in json.loads(result.stdout)] == [{k: record[k] for k in FIELDS}], "a valid hello reply delayed 1.2 seconds must still be listed"
        else:
            lines = result.stdout.decode("utf-8").splitlines()
            assert len(lines) == 2 and lines[0].split() == ["SESSION", "CREATED", "COMMAND", "ARGS"] and lines[1].split()[0] == ep.session_id, "a valid hello reply delayed 1.2 seconds must still be listed"
        assert result.stderr == b""
        assert path.read_bytes() == before and ep.path.exists()


@pytest.mark.parametrize("json_mode", [False, True])
def test_listing_mixes_live_stale_and_invalid_metadata_without_deleting_it(climon2, base_env, climon2_home, short_dir, json_mode):
    """S17 (B11, B16): warn on invalid records, hide stale records, retain all artifacts."""
    with contextlib.ExitStack() as stack:
        live = stack.enter_context(endpoint(short_dir))
        live_path, live_record = synthetic(climon2_home, live)
        dead = stack.enter_context(endpoint(short_dir))
        dead_path, dead_record = synthetic(climon2_home, dead)
        dead_record["socket_path"] = str(short_dir / "absent.sock")
        dead_path.write_text(json.dumps(dead_record))
        bad_paths = []
        for data in (b"{", b"\xff", b"[]", b"{}", {"format_version": 2}, {"args": "wrong type"}):
            path = climon2_home / "sessions" / (str(uuid.uuid4()) + ".pty")
            if isinstance(data, dict):
                data = json.dumps({**live_record, "session_id": path.stem, **data}).encode()
            path.write_bytes(data)
            bad_paths.append(path)
        unreadable = climon2_home / "sessions" / (str(uuid.uuid4()) + ".pty")
        unreadable.write_text(json.dumps({**live_record, "session_id": unreadable.stem}))
        if WINDOWS:
            result = subprocess.run(["icacls", str(unreadable), "/deny", "*S-1-1-0:(R)"], capture_output=True, timeout=DEFAULT_TIMEOUT)
            assert result.returncode == 0
        else:
            unreadable.chmod(0)
        try:
            result = run(climon2, listing_args(json_mode), base_env)
        finally:
            make_private(unreadable)
        assert result.returncode == 0, "individual invalid records must not fail listing"
        if json_mode:
            assert [{k: obj[k] for k in FIELDS} for obj in json.loads(result.stdout)] == [{k: live_record[k] for k in FIELDS}], "listing must include only hello-verified records"
        else:
            lines = result.stdout.decode().splitlines()
            assert len(lines) == 2 and live.session_id in lines[1]
        warnings = result.stderr.decode().splitlines()
        assert len(warnings) == len(bad_paths) + 1
        for path in [*bad_paths, unreadable]:
            assert any(line.startswith("climon2: ignoring session metadata: ") and str(path) in line for line in warnings), f"invalid metadata warning must name {path}"
        assert not any(str(dead_path) in line for line in warnings)
        assert live_path.exists() and dead_path.exists() and all(path.exists() for path in bad_paths)
        assert dead.path.exists()


@pytest.mark.parametrize("json_mode", [False, True])
def test_session_directory_enumeration_errors_are_not_empty_success(climon2, base_env, climon2_home, json_mode):
    """S17 (B3, B16): nonabsence directory errors return 125 and name the path."""
    directory = climon2_home / "sessions"
    directory.mkdir()
    if WINDOWS:
        result = subprocess.run(["icacls", str(directory), "/deny", "*S-1-1-0:(RD)"], capture_output=True, timeout=DEFAULT_TIMEOUT)
        assert result.returncode == 0
    else:
        directory.chmod(0)
    try:
        error(run(climon2, listing_args(json_mode), base_env), str(directory), directory)
    finally:
        make_private(directory)


@pytest.mark.parametrize("json_mode", [False, True])
def test_record_disappearing_during_enumeration_is_silently_skipped(climon2, base_env, climon2_home, short_dir, json_mode):
    """S17 (B16): a second record removed while another probe is pending causes no warning."""
    with contextlib.ExitStack() as stack:
        endpoints = [stack.enter_context(endpoint(short_dir, mode="silent")) for _ in range(2)]
        paths = [synthetic(climon2_home, ep)[0] for ep in endpoints]
        before = [path.read_bytes() for path in paths]
        child = subprocess.Popen([str(climon2.launcher), *listing_args(json_mode)], env=base_env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            wait_until(lambda: any(ep.accepted.is_set() for ep in endpoints) or child.poll() is not None,
                       "listing must reach a discovery probe before record removal")
            pending = next((i for i, ep in enumerate(endpoints) if ep.accepted.is_set()), None)
            assert pending is not None, "listing must reach a discovery probe before record removal"
            assert child.poll() is None, "listing must still be in progress when its record disappears"
            removed = 1 - pending
            # Delete the other record, which a serial reader may have enumerated but not read.
            # Prefetching/parallel readers may already hold both; silent endpoints keep either order stale.
            paths[removed].unlink()
            out, err = child.communicate(timeout=DEFAULT_TIMEOUT)
            assert child.returncode == 0 and out == empty_output(json_mode)
            assert err == b"", "a disappearing record must be silently skipped"
            assert not paths[removed].exists(), "listing must not recreate disappearing metadata"
            assert paths[pending].read_bytes() == before[pending], "listing must retain stale metadata unchanged"
            assert all(ep.path.exists() for ep in endpoints), "listing must not delete stale sockets"
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=DEFAULT_TIMEOUT)


@pytest.mark.skipif(WINDOWS, reason="Unix dangling-symlink disappearance case")
@pytest.mark.parametrize("json_mode", [False, True])
def test_dangling_metadata_symlink_is_skipped_without_modification(climon2, base_env, climon2_home, json_mode):
    """S17 (B16): a dangling GUID symlink is skipped, optionally warned about, and retained."""
    directory = climon2_home / "sessions"
    directory.mkdir()
    path = directory / (str(uuid.uuid4()) + ".pty")
    target = "absent-metadata"
    path.symlink_to(target)
    assert path.is_symlink() and not path.exists()
    result = run(climon2, listing_args(json_mode), base_env)
    assert result.returncode == 0, "a dangling metadata symlink must not fail listing"
    assert result.stdout == empty_output(json_mode), "a dangling metadata symlink must not be listed"
    warnings = result.stderr.decode("utf-8").splitlines()
    assert result.stderr == b"" or (len(warnings) == 1 and warnings[0].startswith("climon2: ignoring session metadata: ") and str(path) in warnings[0]), "a dangling metadata symlink permits only silence or one metadata warning naming its path"
    assert path.is_symlink() and os.readlink(path) == target, "discovery must not delete or replace dangling metadata symlinks"
    assert not path.exists(), "discovery must not create a missing metadata target"


def local_stamp(value):
    instant = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    if WINDOWS:
        return instant.astimezone().strftime("%Y-%m-%d %H:%M:%S")
    # EST5EDT: second March Sunday at 07:00 UTC through first November Sunday at 06:00 UTC.
    march = dt.datetime(instant.year, 3, 1, 7, tzinfo=dt.timezone.utc)
    november = dt.datetime(instant.year, 11, 1, 6, tzinfo=dt.timezone.utc)
    start = march + dt.timedelta(days=(6 - march.weekday()) % 7 + 7)
    end = november + dt.timedelta(days=(6 - november.weekday()) % 7)
    zone = dt.timezone(dt.timedelta(hours=-4 if start <= instant < end else -5))
    return instant.astimezone(zone).strftime("%Y-%m-%d %H:%M:%S")


def test_table_sorts_instants_and_guid_ties_and_uses_historical_local_offsets(climon2, base_env, climon2_home, short_dir, monkeypatch):
    """S18 (B16, B17): instant-local time with tzdata-free Unix rules or the Windows OS zone."""
    env = dict(base_env)
    if WINDOWS:
        env.pop("TZ", None)
        monkeypatch.delenv("TZ", raising=False)
        import ctypes
        ctypes.CDLL("ucrtbase")._tzset()
    else:
        env["TZ"] = "EST5EDT,M3.2.0,M11.1.0"
    with contextlib.ExitStack() as stack:
        records = []
        values = [("00000000-0000-4000-8000-000000000003", "2024-01-15T12:34:56Z"),
                  ("00000000-0000-4000-8000-000000000002", "2024-07-15T12:34:56.250Z"),
                  ("00000000-0000-4000-8000-000000000001", "2024-07-15T12:34:56.250Z")]
        for sid, created in values:
            ep = stack.enter_context(endpoint(short_dir, sid))
            _, record = synthetic(climon2_home, ep, created_at=created, args=[])
            records.append(record)
        expected = [records[2], records[1], records[0]]
        result = run(climon2, ["list"], env)
        assert result.returncode == 0 and result.stderr == b""
        text = result.stdout.decode("utf-8")
        assert "\x1b" not in text, "table must not contain ANSI styling"
        lines = text.splitlines()
        assert lines[0].split() == ["SESSION", "CREATED", "COMMAND", "ARGS"]
        assert len(lines) == 4
        for line, record in zip(lines[1:], expected):
            assert line.split() == [record["session_id"], *local_stamp(record["created_at"]).split(), record["command"]], "table must sort newest first, break ties by GUID and convert each instant locally"
        assert [{k: obj[k] for k in FIELDS} for obj in listed(climon2, env)] == [{k: record[k] for k in FIELDS} for record in expected], "JSON and table must share session ordering"


def escaped(value):
    return "".join("\\\\" if c == "\\" else f"\\u{ord(c):04x}" if ord(c) < 32 or 127 <= ord(c) <= 159 else c for c in value)


def args_cell(args):
    value = " ".join("'" + escaped(arg).replace("'", "'\\''") + "'" for arg in args)
    return value if len(value) <= 80 else value[:79] + "\u2026"


def test_table_escapes_controls_and_json_round_trips_every_metadata_field(climon2, base_env, climon2_home, short_dir):
    """S19 (B17, B18, B19): all C0/DEL/C1 controls, quoting and Unicode-scalar truncation."""
    controls = "".join(chr(i) for i in [*range(32), *range(127, 160)])
    cases = [("cmd with space\\" + controls, ["", "a b", "a'b", "back\\slash", "\u03bb"]),
             ("control-args", [controls]), ("boundary79", ["\U0001f642" * 77]),
             ("boundary80", ["\U0001f642" * 78]), ("boundary81", ["\U0001f642" * 79]),
             ("no-args", [])]
    with contextlib.ExitStack() as stack:
        records = []
        for index, (command, args) in enumerate(cases):
            sid = f"00000000-0000-4000-8000-{index:012x}"
            ep = stack.enter_context(endpoint(short_dir, sid))
            _, record = synthetic(climon2_home, ep, command=command, args=args, depth=index + 1)
            records.append(record)
        result = run(climon2, ["list"], base_env)
        assert result.returncode == 0 and result.stderr == b""
        text = result.stdout.decode("utf-8")
        assert all(ord(c) >= 32 and not 127 <= ord(c) <= 159 for c in text.replace("\n", "").replace("\r", "")), "table must escape all record-origin control characters"
        lines = text.splitlines()
        assert len(lines) == len(records) + 1, "record controls must not inject table rows"
        assert lines[0].split() == ["SESSION", "CREATED", "COMMAND", "ARGS"]
        for line, record in zip(lines[1:], records):
            match = re.fullmatch(r"(\S+)\s+(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d)\s+(.*)", line)
            assert match and match[1] == record["session_id"]
            tail = match[3]
            command = escaped(record["command"])
            assert tail.startswith(command), "COMMAND must be escaped, complete and untruncated"
            assert tail[len(command):].strip() == args_cell(record["args"]), "ARGS must use exact single-quote escaping and 80-scalar truncation"
        # Literal worked examples guard the display oracle's boundary and quoting rules.
        assert args_cell(cases[0][1]) == "'' 'a b' 'a'\\''b' 'back\\\\slash' '\u03bb'"
        assert len(args_cell(cases[2][1])) == 79 and not args_cell(cases[2][1]).endswith("\u2026")
        assert len(args_cell(cases[3][1])) == 80 and not args_cell(cases[3][1]).endswith("\u2026")
        assert args_cell(cases[4][1]) == "'" + "\U0001f642" * 78 + "\u2026"
        result = run(climon2, ["list", "--json"], base_env)
        assert result.returncode == 0 and result.stderr == b""
        assert result.stdout.endswith(b"\n")
        raw = result.stdout.decode("utf-8")
        assert [{k: obj[k] for k in FIELDS} for obj in json.loads(raw)] == [{k: record[k] for k in FIELDS} for record in records], "JSON must preserve full values, metadata types, depth and ordering"
        strings = re.findall(r'"(?:[^"\\]|\\.)*"', raw)
        assert all(not (ord(c) < 32 or 127 <= ord(c) <= 159) for value in strings for c in value), "JSON strings must escape every C0, DEL and C1 control code point"
