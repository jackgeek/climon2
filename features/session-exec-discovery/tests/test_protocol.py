import json
from pathlib import Path
import struct
import threading
import time

import pytest
from harness import wait_until
from afunix import LocalSocket, frame, receive_frame
from support import (HELLO, OBSERVATION_GRACE, answer, assert_hello, cleaned,
                     ready, record_for)


def assert_closed_without_hello(sock, deadline, message="invalid or incomplete handshake must close within the fixed two-second deadline"):
    received = bytearray()
    while True:
        remaining = deadline - time.monotonic()
        assert remaining > 0, message
        sock.settimeout(remaining)
        try:
            chunk = sock.recv(65536)
        except TimeoutError:
            pytest.fail(message)
        except OSError as exc:
            assert getattr(exc, "errno", None) in (32, 54, 104, 10053, 10054), f"unexpected socket failure: {exc}"
            break
        if not chunk:
            break
        received.extend(chunk)
    if len(received) >= 4:
        size = struct.unpack(">I", received[:4])[0]
        if len(received) >= size + 4:
            try:
                value = json.loads(received[4:4 + size])
            except (ValueError, UnicodeError):
                value = None
            assert not (isinstance(value, dict) and value.get("type") == "hello"), "invalid handshake must not receive a successful hello"


@pytest.mark.parametrize("wire", [frame(b""), frame(b"\xff"), frame(b"{bad json"), frame(b'{"type":"hello","protocol":2}'), frame(b'{"type":"not-a-climon2-message","protocol":1}'), struct.pack(">I", 1048577)])
def test_invalid_protocol_connections_close_without_harming_session(climon2, base_env, terminal, fixture_program, wire):
    """S15 (B15): reject bad framing, UTF-8, JSON, versions, types and oversized prefixes."""
    term = terminal([str(climon2.launcher), "exec", *fixture_program], env=base_env)
    info = ready(term)
    path, record = record_for(base_env)
    with LocalSocket() as sock:
        sock.connect(record["socket_path"])
        started = time.monotonic()
        sock.sendall(wire)
        if wire == struct.pack(">I", 1048577):
            assert_closed_without_hello(
                sock, started + 0.75,
                "oversized prefix must close within 0.75 seconds without waiting for its payload")
        else:
            assert_closed_without_hello(sock, started + 2 + OBSERVATION_GRACE)
    assert_hello(record)
    assert answer(term, "echo still-authorized", "ECHO") == "still-authorized"
    term.send_line("exit 0")
    assert term.wait() == 0
    cleaned(path, record, info["pid"])


@pytest.mark.parametrize("mode", ["fragmented", "maximum"])
def test_fragmented_and_maximum_size_hello_do_not_take_control(climon2, base_env, terminal, fixture_program, mode):
    """S14, S15, S16 (B14, B15, B16): hello-only probes are noncontrolling and reusable."""
    term = terminal([str(climon2.launcher), "exec", *fixture_program], env=base_env)
    info = ready(term)
    path, record = record_for(base_env)
    payload = HELLO + b" " * (1048576 - len(HELLO)) if mode == "maximum" else HELLO
    assert_hello(record, payload=payload, fragmented=mode == "fragmented")
    assert_hello(record)
    assert answer(term, "echo control-was-not-stolen", "ECHO") == "control-was-not-stolen"
    term.send_line("exit 0")
    assert term.wait() == 0
    cleaned(path, record, info["pid"])


def test_fragmented_hello_completed_after_one_and_a_half_seconds_is_accepted(climon2, base_env, terminal, fixture_program):
    """S15, S16 (B15): a valid fragmented hello may use most of the two-second deadline."""
    term = terminal([str(climon2.launcher), "exec", *fixture_program], env=base_env)
    info = ready(term)
    path, record = record_for(base_env)
    with LocalSocket() as sock:
        sock.connect(record["socket_path"])
        started = time.monotonic()
        data = frame(HELLO)
        timer = threading.Event()
        # Deliberately schedule wire fragments, rather than wait for session readiness.
        try:
            for offset, part in zip((0, 0.5, 1.0, 1.5), (data[:1], data[1:4], data[4:-1], data[-1:])):
                timer.wait(max(0, started + offset - time.monotonic()))
                sock.sendall(part)
            reply = json.loads(receive_frame(sock, started + 2 + OBSERVATION_GRACE))
        except (OSError, EOFError, TimeoutError) as exc:
            pytest.fail(f"fragmented hello completed at 1.5 seconds must receive a matching reply: {exc}")
        assert reply == {"type": "hello", "protocol": 1, "session_id": record["session_id"]}, "fragmented hello completed at 1.5 seconds must receive a matching reply"
    assert_hello(record)
    assert answer(term, "echo delayed-hello-survived", "ECHO") == "delayed-hello-survived"
    term.send_line("exit 0")
    assert term.wait() == 0
    cleaned(path, record, info["pid"])


@pytest.mark.parametrize("mode", ["silent", "drip"])
def test_handshake_deadline_is_total_not_reset_by_fragments(climon2, base_env, terminal, fixture_program, mode):
    """S16 (B15): acceptance starts one two-second deadline, not one per fragment."""
    term = terminal([str(climon2.launcher), "exec", *fixture_program], env=base_env)
    info = ready(term)
    path, record = record_for(base_env)
    with LocalSocket() as sock:
        sock.connect(record["socket_path"])
        started = time.monotonic()
        if mode == "drip":
            data = frame(HELLO)
            timer = threading.Event()
            for index, offset in enumerate((0, 0.6, 1.2, 1.8)):
                timer.wait(max(0, started + offset - time.monotonic()))
                sock.sendall(data[index:index + 1])
        assert_closed_without_hello(sock, started + 2 + OBSERVATION_GRACE)
    assert_hello(record)
    assert answer(term, "echo deadline-survived", "ECHO") == "deadline-survived"
    term.send_line("exit 0")
    assert term.wait() == 0
    cleaned(path, record, info["pid"])
