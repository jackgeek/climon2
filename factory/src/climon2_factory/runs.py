"""Persistent feature-run state, stored outside the repo under the factory state directory."""

import json
from dataclasses import asdict, dataclass, field
from pathlib import Path


@dataclass
class Run:
    run_id: str
    feature: str
    stage: str
    review_round: int = 0
    attempts: int = 0
    candidate_commit: str | None = None
    candidate_branch: str | None = None
    pr_number: int | None = None
    attempt_findings: list[tuple[str, str]] = field(default_factory=list[tuple[str, str]])


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

    def finish(self, run: Run) -> None:
        self.save(run)
        self._active.unlink()

    def load(self, run_id: str) -> Run:
        data = json.loads((self._runs / run_id / "run.json").read_text())
        data["attempt_findings"] = [tuple(f) for f in data.get("attempt_findings", [])]
        return Run(**data)

    def save(self, run: Run) -> None:
        directory = self._runs / run.run_id
        directory.mkdir(parents=True, exist_ok=True)
        tmp = directory / "run.json.tmp"
        tmp.write_text(json.dumps(asdict(run), indent=2) + "\n")
        tmp.replace(directory / "run.json")
