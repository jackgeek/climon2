"""The shells each platform must test, started with controlled (no user config) startup."""

import shutil
import sys

REQUIRED_SHELLS: dict[str, tuple[str, ...]] = {
    "win32": ("powershell.exe", "pwsh.exe", "cmd.exe"),
    "linux": ("bash", "sh"),
    "darwin": ("zsh", "bash", "sh"),
}

_CONTROLLED_ARGS: dict[str, tuple[str, ...]] = {
    "bash": ("--noprofile", "--norc", "-i"),
    "sh": ("-i",),
    "zsh": ("-f", "-i"),
    "powershell.exe": ("-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass"),
    "pwsh.exe": ("-NoLogo", "-NoProfile"),
    "cmd.exe": ("/d", "/q"),
}


def required_shells() -> tuple[str, ...]:
    return REQUIRED_SHELLS[sys.platform]


def controlled_shell(name: str) -> list[str]:
    """argv that starts `name` interactively without user or system startup files.
    A missing shell is an error, never a skip and never a substitute."""
    if name not in _CONTROLLED_ARGS:
        raise ValueError(f"unknown shell {name!r}")
    path = shutil.which(name)
    if path is None:
        raise AssertionError(f"required shell {name} is not installed")
    return [path, *_CONTROLLED_ARGS[name]]


def controlled_env(name: str, env: dict[str, str]) -> dict[str, str]:
    """`env` adjusted so `name` reads no startup files (sh's ENV, zsh's ZDOTDIR)."""
    env = dict(env)
    env.pop("ENV", None)
    env.pop("BASH_ENV", None)
    if name == "zsh":
        env["ZDOTDIR"] = "/nonexistent"
    return env
