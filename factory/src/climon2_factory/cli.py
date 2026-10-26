"""The `factory` command line."""

import argparse
import re
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

from climon2_factory.artifacts import ApprovalRefused, ArtifactStore
from climon2_factory.ports import Agents, Clock, Executor, GitHub, MacOS, Notifier
from climon2_factory.runs import Run, RunStore
from climon2_factory.workflow import DRAFT_CONTRACT, Pause, StepFailed, Workflow

EXIT_OK = 0
EXIT_STEP_FAILED = 1
EXIT_REFUSED = 2
_FEATURE_ID = re.compile(r"[a-z0-9][a-z0-9-]*")


@dataclass(frozen=True)
class Deps:
    repo_root: Path
    state_dir: Path
    agents: Agents
    executor: Executor
    github: GitHub
    macos: MacOS
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
    commands.add_parser("resume", help="continue the active feature run")
    commands.add_parser("abort", help="end the active feature run")
    show = commands.add_parser("show", help="show an artifact's state and history")
    show.add_argument("artifact")
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
    workflow = Workflow(deps.repo_root, runs, deps.agents, deps.executor, deps.github, deps.macos)
    try:
        match args.command:
            case "start":
                _start(args.feature, deps, runs, workflow)
            case "approve":
                _approve(args.artifact, args.hash, deps, runs, workflow)
            case "resume":
                _report(workflow.resume(_require_active(runs)), deps)
            case "abort":
                _report(workflow.abort(_require_active(runs)), deps)
            case "show":
                _show(args.artifact, deps)
            case "reject":
                _reject(args.artifact, args.reason, deps, runs, workflow)
            case _:
                _status(runs, workflow)
    except Refused as refusal:
        print(f"factory: {refusal}", file=sys.stderr)
        return EXIT_REFUSED
    except StepFailed as failure:
        print(f"step failed: {failure}")
        print("Next: factory resume")
        deps.notifier.notify(f"step failed: {failure}")
        return EXIT_STEP_FAILED
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
    stale = workflow.stale(run)
    if stale is not None:
        raise Refused(f"{stale.describe()}; run factory resume")
    awaiting = workflow.awaiting(run)
    if awaiting != artifact:
        raise Refused(f"feature run {run.run_id} is waiting for {awaiting}, not {artifact}")
    return run


def _require_active(runs: RunStore) -> Run:
    run = runs.active()
    if run is None:
        raise Refused("no active feature run")
    return run


def _show(artifact: str, deps: Deps) -> None:
    store = ArtifactStore(deps.repo_root)
    try:
        status = store.status(artifact)
    except ValueError as invalid:
        raise Refused(str(invalid)) from invalid
    history = store.history(artifact)
    if not history:
        raise Refused(f"{artifact} has never been recorded")
    print(f"{artifact}: {status.state} {status.hash}")
    for path in history[-1].paths:
        print(f"  {path}")
    for changed in status.changed_inputs:
        print(f"  input changed: {changed}")
    print("history:")
    for number, entry in enumerate(history, start=1):
        decisions = "".join(
            f" {d.decision}: {d.reason}" if d.reason else f" {d.decision}" for d in entry.decisions
        )
        print(f"  {number} {entry.hash}{decisions}")


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
