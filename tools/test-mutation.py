"""Fail fast on relevant tests; only the full workspace can certify a survivor.

Without arguments, runs a mutant's test stages: workspace library tests, then the mutated
crate's integration tests, then the full workspace. The first failing stage kills the mutant.

With `ROOT CARGO TIMEOUT_FACTOR STAGE_TIMEOUTS SOURCE...`, run from ROOT, times each
earlier stage of the given sources once, unloaded in the warm `target/mutation-tests`
build, and writes each stage's timeout (TIMEOUT_FACTOR times that run, at least two
seconds, as the mutator times the whole command) to the STAGE_TIMEOUTS file.

When `CLIMON2_MUTATION_STAGE_TIMEOUTS` names that file, a mutant's earlier stage is built
first and its tests then run under that stage's timeout. A stage that outlives it kills
the run's whole process group, as the mutator's own timeout does, so the mutator counts
the mutant as killed. The full workspace has no timeout of its own: the mutator's timeout
for the whole command still bounds it.
"""

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

MINIMUM_TIMEOUT = 2.0


def mutated_source(workspace):
    if not (
        workspace.name.startswith("worker-")
        and workspace.parent.name.startswith("run-")
        and workspace.parent.parent.name == "mutation-workers"
    ):
        return None
    sources = []
    # The walk never enters a symlinked directory, so only private copies are found.
    for directory, children, files in os.walk(workspace):
        children[:] = [name for name in children if name not in ("target", ".git", ".metrics")]
        for name in files:
            path = Path(directory) / name
            if (
                path.suffix == ".rs"
                and "src" in path.relative_to(workspace).parts
                and not path.is_symlink()
            ):
                sources.append(path)
    if len(sources) != 1:
        raise RuntimeError(f"expected one private mutated Rust source, found {sources}")
    return sources[0]


def early_stages(source):
    """The stages that can kill a mutant of `source` before the full workspace runs."""
    yield ["--lib"]
    package = next(
        parent for parent in source.parents
        if (parent / "Cargo.toml").is_file()
    )
    targets = sorted((package / "tests").glob("*.rs"))
    if targets:
        yield [item for target in targets for item in ("--test", target.stem)]


def stages(workspace):
    source = mutated_source(workspace)
    if source is not None:
        seed_build(workspace)
        yield from early_stages(source)
    yield []


def seed_build(workspace):
    baseline = workspace.parents[3] / "target/mutation-tests"
    destination = workspace / "target/mutation-tests"
    if destination.exists():
        return
    if not baseline.is_dir():
        print("No baseline artifact cache; building a cold mutation worker.", file=sys.stderr)
        return
    destination.mkdir(parents=True)
    options = ["-cRp"] if sys.platform == "darwin" else ["-a", "--reflink=auto"]
    subprocess.run(
        ["cp", *options, str(baseline) + "/.", str(destination)],
        check=True,
    )


def cargo_test(cargo):
    return [cargo, "test", "--workspace", "--locked", "--target-dir", "target/mutation-tests"]


def name(selection):
    """A stage's key in the stage timeouts file."""
    return " ".join(selection)


def run_stage(command, selection, timeout):
    if timeout is None:
        return subprocess.run(command + selection).returncode
    # Only running tests can hang; a mutant's rebuild stays within the mutator's timeout.
    built = subprocess.run(command + selection + ["--no-run"]).returncode
    if built:
        return built
    process = subprocess.Popen(command + selection)
    try:
        return process.wait(timeout)
    except subprocess.TimeoutExpired:
        print(
            f"Killing this run: `{name(selection)}` outlived its {timeout:.1f}s stage timeout",
            file=sys.stderr,
        )
        os.killpg(os.getpgrp(), signal.SIGKILL)


def stage_timeouts():
    path = os.environ.get("CLIMON2_MUTATION_STAGE_TIMEOUTS")
    if path is None:
        return {}
    return {
        selection: float(timeout)
        for selection, timeout in json.loads(Path(path).read_text()).items()
    }


def run():
    # The rustup proxy pins rustc for nested builds too; tests may substitute Cargo.
    command = cargo_test(os.environ.get("CLIMON2_MUTATION_CARGO", "cargo"))
    timeouts = stage_timeouts()
    for selection in stages(Path.cwd()):
        code = run_stage(command, selection, timeouts.get(name(selection)))
        if code:
            return code
    return 0


def measure(root, cargo, timeout_factor, output, sources):
    command = cargo_test(cargo)
    subprocess.run(command + ["--no-run"], check=True)
    timeouts = {}
    for source in sources:
        for selection in early_stages(root / source):
            if name(selection) in timeouts:
                continue
            started = time.monotonic()
            subprocess.run(command + selection, check=True)
            seconds = time.monotonic() - started
            timeouts[name(selection)] = max(MINIMUM_TIMEOUT, timeout_factor * seconds)
            print(f"Stage `{name(selection)}` took {seconds:.1f}s unloaded.", file=sys.stderr)
    output.write_text(json.dumps(timeouts))


def main(arguments):
    if not arguments:
        return run()
    root, cargo, timeout_factor, output, *sources = arguments
    measure(Path(root), cargo, float(timeout_factor), Path(output), sources)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
