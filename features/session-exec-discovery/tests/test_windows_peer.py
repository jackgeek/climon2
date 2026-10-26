import os

import pytest
from afunix import LocalSocket, frame, receive_frame
from native import WINDOWS
from support import HELLO, assert_hello, cleaned, ready, record_for
from test_exec import redirected, terminal_error


if WINDOWS:
    def test_windows_peer_pid_verification_is_available_or_startup_fails_closed(climon2, base_env, short_dir, tmp_path, terminal, fixture_program):
        """S14 (B13, B14, B15): native peer-PID support is mandatory, not replaced by ACLs."""
        path = short_dir / "identity.sock"
        unsupported = None
        with LocalSocket() as listener, LocalSocket() as sender:
            listener.bind(path)
            listener.listen()
            sender.connect(path)
            with listener.accept() as receiver:
                try:
                    assert receiver.peer_pid() == os.getpid(), "accepted peer PID must identify the connecting process"
                    assert sender.peer_pid() == os.getpid()
                except OSError as exc:
                    unsupported = exc
        path.unlink()
        if unsupported is not None:
            err = tmp_path / "stderr"
            term = redirected(terminal, tmp_path, [climon2.launcher, "exec", *fixture_program], base_env, stderr=err)
            terminal_error(term, err, "peer identity verification unavailable")
            pytest.fail(f"Windows peer identity verification unavailable ({unsupported}); contract amendment required, not ACL-only delivery")
        term = terminal([str(climon2.launcher), "exec", *fixture_program], env=base_env)
        info = ready(term)
        metadata, record = record_for(base_env)
        with LocalSocket() as conn:
            conn.connect(record["socket_path"])
            assert conn.peer_pid() == record["daemon_pid"], "socket peer must be the actual session daemon"
        assert_hello(record)
        term.send_line("exit 0")
        assert term.wait() == 0
        cleaned(metadata, record, info["pid"])
