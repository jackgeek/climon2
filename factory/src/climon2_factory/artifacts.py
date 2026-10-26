"""Content-addressed artifact store (see docs/adr/0001-content-addressed-artifacts.md).

An artifact is a named set of repo paths. Recording it writes an append-only manifest of
its content hash and the hashes of its inputs. Status is always recomputed from disk.
"""

import hashlib
import json
import re
from collections.abc import Sequence
from dataclasses import dataclass
from datetime import UTC, datetime
from enum import StrEnum
from pathlib import Path
from typing import Any

RECORDS_DIR = ".factory/records"
_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]*(/[A-Za-z0-9][A-Za-z0-9._-]*)*")


class State(StrEnum):
    MISSING = "missing"
    UNRECORDED = "unrecorded"
    STALE = "stale"
    UNAPPROVED = "unapproved"
    APPROVED = "approved"
    REJECTED = "rejected"


_NOT_CURRENT = (State.MISSING, State.UNRECORDED, State.STALE)


class ApprovalRefused(Exception):
    pass


class UnsafePath(ValueError):
    """A path that escapes the repository or passes through a symlink."""


@dataclass(frozen=True)
class ArtifactVersion:
    name: str
    hash: str
    paths: tuple[str, ...]


@dataclass(frozen=True)
class Status:
    state: State
    hash: str | None
    changed_inputs: tuple[str, ...] = ()


@dataclass(frozen=True)
class Decision:
    decision: str
    reason: str | None
    decided_at: str


@dataclass(frozen=True)
class HistoryEntry:
    hash: str
    paths: tuple[str, ...]
    inputs: dict[str, str]
    decisions: tuple[Decision, ...]


class ArtifactStore:
    def __init__(self, repo_root: Path) -> None:
        self._root = repo_root
        self._records = repo_root / RECORDS_DIR

    def record(
        self,
        name: str,
        paths: Sequence[str],
        path_inputs: Sequence[str] = (),
        artifact_inputs: Sequence[str] = (),
    ) -> ArtifactVersion:
        _validate_name(name)
        for ref in artifact_inputs:
            if ref == name or self._depends_on(ref, name):
                raise ValueError(f"{name}: input {ref} would create a dependency cycle")
            if self.status(ref).state in _NOT_CURRENT:
                raise ValueError(f"{name}: input {ref} is not current")
        version = ArtifactVersion(name=name, hash=self._hash_paths(paths), paths=tuple(paths))
        inputs = {f"path:{p}": self._hash_paths([p]) for p in path_inputs}
        inputs |= {f"artifact:{a}": self.status(a).hash for a in artifact_inputs}
        manifest = {
            "name": name,
            "hash": version.hash,
            "paths": list(version.paths),
            "inputs": inputs,
            "recorded_at": _now(),
        }
        _append(self._records / name / "manifests", manifest)
        return version

    def status(self, name: str) -> Status:
        _validate_name(name)
        manifest = self._latest_manifest(name)
        if manifest is None:
            return Status(state=State.UNRECORDED, hash=None)
        if not all(self._safe(p).exists() for p in manifest["paths"]):
            return Status(state=State.MISSING, hash=None)
        current = self._hash_paths(manifest["paths"])
        if current != manifest["hash"]:
            return Status(state=State.UNRECORDED, hash=current)
        changed = tuple(k for k, h in manifest["inputs"].items() if not self._input_current(k, h))
        if changed:
            return Status(state=State.STALE, hash=current, changed_inputs=changed)
        latest = len(_read_all(self._records / name / "manifests")) - 1
        decisions = [d for d in self._decisions(name) if d["manifest"] == latest]
        if not decisions:
            return Status(state=State.UNAPPROVED, hash=current)
        return Status(state=State(decisions[-1]["decision"]), hash=current)

    def approve(self, name: str, hash: str) -> None:
        self._decide(name, hash, State.APPROVED, reason=None)

    def reject(self, name: str, hash: str, reason: str) -> None:
        self._decide(name, hash, State.REJECTED, reason=reason)

    def history(self, name: str) -> list[HistoryEntry]:
        decisions = self._decisions(name)
        return [
            HistoryEntry(
                hash=m["hash"],
                paths=tuple(m["paths"]),
                inputs=m["inputs"],
                decisions=tuple(
                    Decision(d["decision"], d["reason"], d["decided_at"])
                    for d in decisions
                    if d["manifest"] == seq
                ),
            )
            for seq, m in enumerate(_read_all(self._records / name / "manifests"))
        ]

    def _decide(self, name: str, hash: str, decision: State, reason: str | None) -> None:
        status = self.status(name)
        if status.state in _NOT_CURRENT or hash != status.hash:
            raise ApprovalRefused(f"{name}: {hash} is not the current recorded hash")
        record = {
            "hash": hash,
            "manifest": len(_read_all(self._records / name / "manifests")) - 1,
            "decision": decision.value,
            "reason": reason,
            "decided_at": _now(),
        }
        _append(self._records / name / "decisions", record)

    def _input_current(self, key: str, recorded_hash: str) -> bool:
        kind, _, ref = key.partition(":")
        if kind == "path":
            return self._safe(ref).exists() and self._hash_paths([ref]) == recorded_hash
        status = self.status(ref)
        return status.state not in _NOT_CURRENT and status.hash == recorded_hash

    def _depends_on(self, ref: str, name: str) -> bool:
        manifest = self._latest_manifest(ref)
        if manifest is None:
            return False
        for key in manifest["inputs"]:
            kind, _, dep = key.partition(":")
            if kind == "artifact" and (dep == name or self._depends_on(dep, name)):
                return True
        return False

    def _latest_manifest(self, name: str) -> dict[str, Any] | None:
        manifests = _read_all(self._records / name / "manifests")
        return manifests[-1] if manifests else None

    def _decisions(self, name: str) -> list[dict[str, Any]]:
        return _read_all(self._records / name / "decisions")

    def _hash_paths(self, paths: Sequence[str]) -> str:
        digest = hashlib.sha256()
        for rel in sorted(self._files(paths)):
            data = (self._root / rel).read_bytes()
            digest.update(f"{rel}\0{len(data)}\0".encode())
            digest.update(data)
        return f"sha256:{digest.hexdigest()}"

    def _files(self, paths: Sequence[str]) -> set[str]:
        files: set[str] = set()
        for rel in paths:
            path = self._safe(rel)
            if path.is_dir():
                for f in path.rglob("*"):
                    if f.is_symlink():
                        raise UnsafePath(f"{f} is a symlink")
                    if f.is_file():
                        files.add(f.relative_to(self._root).as_posix())
            else:
                files.add(path.relative_to(self._root).as_posix())
        return files

    def _safe(self, rel: str) -> Path:
        """Resolve a repo-relative path, refusing escapes and any symlinked component."""
        if Path(rel).is_absolute():
            raise UnsafePath(f"{rel} is absolute")
        path = self._root
        for part in Path(rel).parts:
            if part in ("..", "."):
                raise UnsafePath(f"{rel} contains {part!r}")
            path = path / part
            if path.is_symlink():
                raise UnsafePath(f"{rel} passes through symlink {path}")
        return path


def _validate_name(name: str) -> None:
    if not _NAME.fullmatch(name):
        raise ValueError(f"invalid artifact name {name!r}")


def _now() -> str:
    return datetime.now(UTC).isoformat()


def _append(directory: Path, record: dict[str, Any]) -> None:
    """Append-only record log: one zero-padded sequence-numbered JSON file per entry."""
    directory.mkdir(parents=True, exist_ok=True)
    seq = len(list(directory.glob("*.json")))
    (directory / f"{seq:06d}.json").write_text(json.dumps(record, indent=2) + "\n")


def _read_all(directory: Path) -> list[dict[str, Any]]:
    if not directory.exists():
        return []
    return [json.loads(p.read_text()) for p in sorted(directory.glob("*.json"))]
