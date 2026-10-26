"""Persistent feature-run state, stored outside the repo under the factory state directory."""

import json
from dataclasses import asdict, dataclass
from pathlib import Path


@dataclass
class Run:
    run_id: str
    feature: str
    stage: str
    review_round: int = 0


class RunStore:
    def __init__(self, state_dir: Path) -> None:
        self._runs = state_dir / "runs"
        self._active = state_dir / "active"

    def active(self) -> Run | None:
        if not self._active.exists():
            return None
        return self.load(self._active.read_text().strip())

    def create(self, run: Run) -> None:
        self.save(run)
        self._active.write_text(run.run_id + "\n")

    def load(self, run_id: str) -> Run:
        return Run(**json.loads((self._runs / run_id / "run.json").read_text()))

    def save(self, run: Run) -> None:
        directory = self._runs / run.run_id
        directory.mkdir(parents=True, exist_ok=True)
        tmp = directory / "run.json.tmp"
        tmp.write_text(json.dumps(asdict(run), indent=2) + "\n")
        tmp.replace(directory / "run.json")
