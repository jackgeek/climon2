import os
import pwd


def test_cannot_see_factory_secrets() -> None:
    assert "CLIMON2_FACTORY_CANARY" not in os.environ


def test_home_is_not_the_users_home() -> None:
    assert os.environ["HOME"] != pwd.getpwuid(os.getuid()).pw_dir
