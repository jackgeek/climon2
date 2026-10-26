import socket


def test_reaches_the_network() -> None:
    socket.create_connection(("1.1.1.1", 53), timeout=5).close()
