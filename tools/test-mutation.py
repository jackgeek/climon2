"""Fail fast on relevant tests; only the full workspace can certify a survivor."""

import os
from pathlib import Path
import subprocess
import sys


def mutated_source(workspace):
    if not (
        workspace.name.startswith("worker-")
        and workspace.parent.name.startswith("run-")
        and workspace.parent.parent.name == "mutation-workers"
    ):
        return None
    sources = []
    for directory, children, files in os.walk(workspace, followlinks=False):
        children[:] = [
            name for name in children
            if name not in ("target", ".git", ".metrics")
            and not (Path(directory) / name).is_symlink()
        ]
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


def stages(workspace):
    source = mutated_source(workspace)
    if source is not None:
        seed_build(workspace)
        yield ["--lib"]
        package = next(
            parent for parent in source.parents
            if (parent / "Cargo.toml").is_file()
        )
        targets = sorted((package / "tests").glob("*.rs"))
        if targets:
            yield [item for target in targets for item in ("--test", target.stem)]
    yield []


def seed_build(workspace):
    baseline = workspace.parents[3] / "target/mutation-tests"
    destination = workspace / "target/mutation-tests"
    if destination.exists():
        return
    if not baseline.is_dir():
        print("No baseline artifact cache; building a cold mutation worker.", flush=True)
        return
    destination.mkdir(parents=True)
    options = ["-cRp"] if sys.platform == "darwin" else ["-a", "--reflink=auto"]
    subprocess.run(
        ["cp", *options, str(baseline) + "/.", str(destination)],
        check=True,
    )


def main():
    # The rustup proxy pins rustc for nested builds too; tests may substitute Cargo.
    command = [
        os.environ.get("CLIMON2_MUTATION_CARGO", "cargo"),
        "test", "--workspace", "--locked", "--target-dir", "target/mutation-tests",
    ]
    for selection in stages(Path.cwd()):
        result = subprocess.run(command + selection)
        if result.returncode:
            return result.returncode
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
