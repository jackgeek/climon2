"""Reject infrastructure failures before they can count as killed mutants."""

from pathlib import Path
import subprocess
import sys

from mutator import workers


def main():
    root = Path(sys.argv[1]).resolve()
    cargo = sys.argv[2]
    sources = sys.argv[3:]
    if not sources:
        raise ValueError("the workspace proof needs shipped source paths")
    run = workers.new_run_dir(root)
    log = root / ".metrics" / "worker-baseline.log"
    try:
        worker = workers.create_workers(
            run, root, sources[0], (root / sources[0]).read_bytes(), 1
        )[0]
        # Expand every shipped crate, not just one worker's mutated source spine.
        for source in sources[1:]:
            workers._copy_importer(worker, root, source)
        with log.open("w") as output:
            result = subprocess.run(
                [
                    cargo,
                    "test",
                    "--workspace",
                    "--locked",
                    "--target-dir",
                    "target/mutation-tests",
                ],
                cwd=worker,
                stdout=output,
                stderr=subprocess.STDOUT,
            )
        if result.returncode:
            print(log.read_text(errors="replace"), file=sys.stderr)
            return result.returncode
        print("Verified unmutated workspace in a real mutation overlay.")
        return 0
    finally:
        workers.delete_tree(run)


if __name__ == "__main__":
    raise SystemExit(main())
