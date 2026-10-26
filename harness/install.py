"""The climon2 install layout, built from the binaries the gate compiled."""

import shutil
import sys
from dataclasses import dataclass
from pathlib import Path

EXE = ".exe" if sys.platform == "win32" else ""
COMPANIONS = ("climon2-cli", "climon2-daemon", "climon2-client")


@dataclass(frozen=True)
class Install:
    root: Path
    version: str

    @property
    def launcher(self) -> Path:
        return self.root / f"climon2{EXE}"

    @property
    def current(self) -> Path:
        return self.root / "current"

    @property
    def version_dir(self) -> Path:
        return self.root / "versions" / self.version

    def companion(self, name: str) -> Path:
        return self.version_dir / f"{name}{EXE}"


def install_layout(bin_dir: Path, root: Path, version: str = "0.1.0") -> Install:
    """Lay out `climon2` (launcher), `current` and `versions/<version>/{climon2-cli,
    climon2-daemon, climon2-client}` under `root`, copying the binaries from `bin_dir`."""
    install = Install(root, version)
    install.version_dir.mkdir(parents=True, exist_ok=True)
    missing = [
        name for name in ("climon2", *COMPANIONS) if not (bin_dir / f"{name}{EXE}").is_file()
    ]
    if missing:
        raise AssertionError(f"climon2 build in {bin_dir} has no {', '.join(missing)}")
    shutil.copy2(bin_dir / f"climon2{EXE}", install.launcher)
    for name in COMPANIONS:
        shutil.copy2(bin_dir / f"{name}{EXE}", install.companion(name))
    install.current.write_text(version + "\n")
    return install
