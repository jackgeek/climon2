"""Keep fresh compilation separate from bounded mutant test execution."""

import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys


def main():
    sources = [Path("probe/src") / name for name in ("input.rs", "lib.rs", "timing.rs")]
    digest = hashlib.sha256(b"".join(source.read_bytes() for source in sources)).hexdigest()
    directory = Path(".cache/legacy-mutation-evidence-checked")
    directory.mkdir(parents=True, exist_ok=True)
    command = [
        "cargo", "test", "--quiet", "--offline", "--locked",
        "--manifest-path", "probe/Cargo.toml", "--target-dir", "target/probe-tests",
    ]
    with (directory / f"{digest}.txt").open("w") as log:
        print("MUTATION_PHASE=compile", file=log, flush=True)
        process = subprocess.Popen(
            command + ["--no-run", "--message-format=json"],
            stdout=subprocess.PIPE, stderr=log, start_new_session=True,
        )
        try:
            output, _ = process.communicate(timeout=120)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            output, _ = process.communicate()
            log.write(output.decode())
            print("MUTATION_TIMEOUT_PHASE=compile", file=log, flush=True)
            return 124
        log.write(output.decode())
        print(f"MUTATION_EXIT={process.returncode}", file=log, flush=True)
        if process.returncode:
            return process.returncode
        executables = [
            message["executable"]
            for message in map(json.loads, output.splitlines())
            if message["reason"] == "compiler-artifact"
            and message["profile"]["test"] and message["executable"]
        ]
        if not executables:
            raise RuntimeError("Cargo reported no compiled test executables")
        for executable in executables:
            print(f"MUTATION_PHASE=runtime EXECUTABLE={executable}", file=log, flush=True)
            process = subprocess.Popen(
                [executable, "--quiet"], cwd="probe", stdout=log,
                stderr=subprocess.STDOUT, start_new_session=True,
            )
            try:
                result = process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
                print("MUTATION_TIMEOUT_PHASE=runtime", file=log, flush=True)
                return 124
            print(f"MUTATION_EXIT={result}", file=log, flush=True)
            if result:
                return result
    return 0


if __name__ == "__main__":
    sys.exit(main())
