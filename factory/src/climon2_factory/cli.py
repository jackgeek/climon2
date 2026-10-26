"""The `factory` command line."""

import argparse
import re
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

from climon2_factory.artifacts import ApprovalRefused, ArtifactStore
from climon2_factory.ports import Agents, Clock, Executor, Notifier
from climon2_factory.runs import Run, RunStore
from climon2_factory.workflow import DRAFT_CONTRACT, Pause, Workflow

EXIT_OK = 0
EXIT_REFUSED = 2
_FEATURE_ID = re.compile(r"[a-z0-9][a-z0-9-]*")


@dataclass(frozen=True)
class Deps:
    repo_root: Path
    state_dir: Path
    agents: Agents
    executor: Executor
    notifier: Notifier
    clock: Clock


class Refused(Exception):
    pass


def main(argv: Sequence[str], deps: Deps) -> int:
    parser = argparse.ArgumentParser(prog="factory")
    commands = parser.add_subparsers(dest="command", required=True)
    start = commands.add_parser("start", help="start a feature run")
    start.add_argument("feature")
    commands.add_parser("status", help="show the active feature run")
    approve = commands.add_parser("approve", help="approve one artifact hash and continue")
    approve.add_argument("artifact")
    approve.add_argument("hash")
    reject = commands.add_parser("reject", help="reject the awaited artifact and revise it")
    reject.add_argument("artifact")
    reject.add_argument("--reason", required=True)
    try:
        args = parser.parse_args(argv)
    except SystemExit as exit:
        return EXIT_OK if exit.code == 0 else EXIT_REFUSED

    runs = RunStore(deps.state_dir)
    workflow = Workflow(deps.repo_root, runs, deps.agents, deps.executor)
    try:
        match args.command:
            case "start":
                _start(args.feature, deps, runs, workflow)
            case "approve":
                _approve(args.artifact, args.hash, deps, runs, workflow)
            case "reject":
                _reject(args.artifact, args.reason, deps, runs, workflow)
            case _:
                _status(runs, workflow)
    except Refused as refusal:
        print(f"factory: {refusal}", file=sys.stderr)
        return EXIT_REFUSED
    return EXIT_OK


def _start(feature: str, deps: Deps, runs: RunStore, workflow: Workflow) -> None:
    if not _FEATURE_ID.fullmatch(feature):
        raise Refused(f"invalid feature id {feature!r}")
    active = runs.active()
    if active is not None:
        raise Refused(f"feature run {active.run_id} ({active.feature}) is already active")
    brief = f"features/{feature}/brief.md"
    if not (deps.repo_root / brief).is_file():
        raise Refused(f"{brief} not found")
    run_id = f"{feature}-{deps.clock.now():%Y%m%dT%H%M%SZ}"
    run = Run(run_id=run_id, feature=feature, stage=DRAFT_CONTRACT)
    runs.create(run)
    _report(workflow.advance(run), deps)


def _approve(artifact: str, hash: str, deps: Deps, runs: RunStore, workflow: Workflow) -> None:
    run = _require_awaiting(runs, workflow, artifact)
    try:
        ArtifactStore(deps.repo_root).approve(artifact, hash)
    except ApprovalRefused as refusal:
        raise Refused(str(refusal)) from refusal
    _report(workflow.advance(run), deps)


def _reject(artifact: str, reason: str, deps: Deps, runs: RunStore, workflow: Workflow) -> None:
    run = _require_awaiting(runs, workflow, artifact)
    workflow.reject(run, reason)
    _report(workflow.advance(run), deps)


def _require_awaiting(runs: RunStore, workflow: Workflow, artifact: str) -> Run:
    run = _require_active(runs)
    awaiting = workflow.awaiting(run)
    if awaiting != artifact:
        raise Refused(f"feature run {run.run_id} is waiting for {awaiting}, not {artifact}")
    return run


def _require_active(runs: RunStore) -> Run:
    run = runs.active()
    if run is None:
        raise Refused("no active feature run")
    return run


def _status(runs: RunStore, workflow: Workflow) -> None:
    run = runs.active()
    if run is None:
        print("No active feature run.")
        return
    print(f"{run.feature} ({run.run_id})")
    pause = workflow.pause(run)
    print(pause.reason)
    for detail in pause.details:
        print(f"  {detail}")


def _report(pause: Pause, deps: Deps) -> None:
    print(pause.reason)
    for detail in pause.details:
        print(f"  {detail}")
    print(f"Next: {pause.next_command}")
    deps.notifier.notify(pause.reason)
