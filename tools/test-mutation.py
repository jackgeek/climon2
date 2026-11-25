"""Fail fast on relevant tests; only the full workspace can certify a survivor.

Without arguments, runs a mutant's test stages: workspace library tests, then the mutated
crate's integration tests, then the suite, which is the full workspace. The first failing
stage kills the mutant.

Where only part of the workspace builds (Windows so far), `CLIMON2_MUTATION_SUITE` names
the `cargo test` runs that make up the suite, as a JSON list of argument lists. A mutant
then runs those alone, in order, with no earlier stages.

With `ROOT CARGO TIMEOUT_FACTOR STAGE_TIMEOUTS SOURCE...`, run from ROOT, times each
earlier stage of the given sources once, then the suite, unloaded in the warm
`target/mutation-tests` build. It writes each earlier stage's timeout, and the suite's
under the key `suite`, to the STAGE_TIMEOUTS file: TIMEOUT_FACTOR times that run, at least
two seconds, as the mutator times the whole command. Without sources nothing is mutated,
so nothing is timed.

When `CLIMON2_MUTATION_STAGE_TIMEOUTS` names that file, each stage of a mutant is built
first and its tests then run under a timeout: an earlier stage's own, and for the run as
a whole the suite's, so that a mutant's run ends before the mutator's timeout for the
whole command (the factor times its unloaded run of the suite) would kill it. A stage that
outlives its timeout fails, so the mutator counts the mutant as killed.

Each stage's tests, measured or not, write to a file rather than to the caller's pipe, and
get a temporary directory of their own. When they end or time out, every process they
started is killed:
their descendants, and any process whose executable lies in the stage's build or temporary
directory, with its descendants. That finds a daemon that left its session, which a
process group does not, and whose parent has exited (#401). On Linux this process also
adopts orphans, so descent alone finds them. It reaps them while the tests run, as an
orphan a test kills stays a zombie until reaped, and a test may wait for it to be gone.
"""

from collections import defaultdict
from contextlib import suppress
import ctypes
import functools
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from typing import NamedTuple, Optional

MINIMUM_TIMEOUT = 2.0
TARGET = "target/mutation-tests"
# The key of the suite's timeout in the stage timeouts file.
SUITE = "suite"
# A stage killed at its timeout exits as `timeout` does.
TIMED_OUT = 124
# A dying process can start another, so a sweep repeats until none is left.
SWEEP_ROUNDS = 100
SWEEP_PAUSE = 0.05
# How often a stage's wait reaps adopted orphans.
REAP_PAUSE = 0.02
PR_SET_CHILD_SUBREAPER = 36
WINDOWS_STILL_ACTIVE = 259


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
    """The stages that can kill a mutant of `source` before the suite runs."""
    yield ["--lib"]
    package = next(
        parent for parent in source.parents
        if (parent / "Cargo.toml").is_file()
    )
    targets = sorted((package / "tests").glob("*.rs"))
    if targets:
        yield [item for target in targets for item in ("--test", target.stem)]


def seed_build(workspace):
    baseline = workspace.parents[3] / TARGET
    destination = workspace / TARGET
    if destination.exists():
        return
    if not baseline.is_dir():
        print("No baseline artifact cache; building a cold mutation worker.", file=sys.stderr)
        return
    if sys.platform == "win32":
        # No cp. copy2 keeps the modification times Cargo's freshness checks read, and
        # extended-length paths pass the 260-character limit a worker's build reaches.
        shutil.copytree(
            "\\\\?\\" + str(baseline.resolve()),
            "\\\\?\\" + str(destination.resolve()),
            symlinks=True,
        )
        return
    destination.mkdir(parents=True)
    options = ["-cRp"] if sys.platform == "darwin" else ["-a", "--reflink=auto"]
    subprocess.run(
        ["cp", *options, str(baseline) + "/.", str(destination)],
        check=True,
    )


def name(selection):
    """A stage's key in the stage timeouts file."""
    return " ".join(selection)


def cargo_test(cargo, selection):
    return [cargo, "test", *selection, "--locked", "--target-dir", TARGET]


def suite(cargo):
    """The suite's `cargo test` runs, as `(name, command)` pairs."""
    text = os.environ.get("CLIMON2_MUTATION_SUITE")
    selections = [["--workspace"]] if text is None else json.loads(text)
    return [(name(selection), cargo_test(cargo, selection)) for selection in selections]


def earlier(cargo, sources):
    """The earlier stages of `sources`, each once, as `(name, command)` pairs; none where
    the suite is only part of the workspace."""
    if "CLIMON2_MUTATION_SUITE" in os.environ:
        return []
    workspace = cargo_test(cargo, ["--workspace"])
    stages = {}
    for source in sources:
        for selection in early_stages(source):
            stages.setdefault(name(selection), workspace + selection)
    return list(stages.items())


def stages(cargo, workspace):
    source = mutated_source(workspace)
    if source is None:
        return suite(cargo)
    seed_build(workspace)
    return earlier(cargo, [source]) + suite(cargo)


def stage_timeouts():
    path = os.environ.get("CLIMON2_MUTATION_STAGE_TIMEOUTS")
    if path is None:
        return {}
    return {
        selection: float(timeout)
        for selection, timeout in json.loads(Path(path).read_text()).items()
    }


def limit(own, deadline):
    """The sooner of a stage's own timeout and the run's deadline, in seconds from now."""
    limits = [] if own is None else [own]
    if deadline is not None:
        limits.append(max(0.0, deadline - time.monotonic()))
    return min(limits, default=None)


def run():
    started = time.monotonic()
    # The rustup proxy pins rustc for nested builds too; tests may substitute Cargo.
    cargo = os.environ.get("CLIMON2_MUTATION_CARGO", "cargo")
    timeouts = stage_timeouts()
    budget = timeouts.get(SUITE)
    deadline = None if budget is None else started + budget
    for stage, command in stages(cargo, Path.cwd()):
        code = run_stage(stage, command, limit(timeouts.get(stage), deadline))
        if code:
            return code
    return 0


def run_stage(stage, command, timeout):
    if timeout is not None:
        # Only running tests can hang; a mutant's rebuild stays within the mutator's timeout.
        built = subprocess.run(command + ["--no-run"]).returncode
        if built:
            return built
    code = contain(command, timeout)
    if code == TIMED_OUT:
        print(
            f"Killed this run: `{stage}` outlived its {timeout:.1f}s stage timeout",
            file=sys.stderr,
        )
    return code


def contain(command, timeout):
    """Runs `command` with its output in a file and a temporary directory of its own, and
    returns its status, or TIMED_OUT once it outlives `timeout` seconds. Either way, every
    process it started is gone before this returns, so none holds the mutator's pipe."""
    # Short, as some tests bind Unix sockets under the temporary directory.
    scratch = Path(tempfile.mkdtemp(prefix="m"))
    environment = dict(os.environ, TMPDIR=str(scratch), TMP=str(scratch), TEMP=str(scratch))
    try:
        with tempfile.TemporaryFile() as output:
            process = subprocess.Popen(
                command, stdout=output, stderr=subprocess.STDOUT, env=environment
            )
            try:
                code = wait(process, timeout)
            finally:
                sweep([Path(TARGET), scratch])
                process.wait()
            output.seek(0)
            sys.stdout.flush()
            shutil.copyfileobj(output, sys.stdout.buffer)
            sys.stdout.flush()
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    return code


def wait(process, timeout):
    """`process`'s status, or TIMED_OUT once it outlives `timeout` seconds. Meanwhile
    reaps the orphans this process adopted, so none that dies stays a zombie: a test that
    kills its orphan waits until it is gone."""
    deadline = None if timeout is None else time.monotonic() + timeout
    while True:
        code = process.poll()
        if code is not None:
            return code
        if deadline is not None and time.monotonic() >= deadline:
            return TIMED_OUT
        reap_adopted(process.pid)
        time.sleep(REAP_PAUSE)


def reap_adopted(spare):
    """Reaps every exited child but `spare`, whose status its Popen collects. Only Linux
    adopts orphans; elsewhere this process's only child is `spare`."""
    if sys.platform != "linux":
        return
    while True:
        try:
            exited = os.waitid(os.P_ALL, 0, os.WEXITED | os.WNOHANG | os.WNOWAIT)
        except ChildProcessError:
            return
        if exited is None or exited.si_pid == spare:
            return
        with suppress(ChildProcessError):
            os.waitpid(exited.si_pid, 0)


class Process(NamedTuple):
    parent: int
    # The executable's path, where it can be read.
    executable: Optional[str]
    # Where a dead parent's ID can be reused for a later process (Windows), the creation
    # time; elsewhere None, as an orphan gets a live parent.
    created: Optional[int]


def adopt_orphans():
    """On Linux, this process's orphaned descendants become its children."""
    if sys.platform == "linux":
        libc = ctypes.CDLL(None, use_errno=True)
        if libc.prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) != 0:
            raise OSError(ctypes.get_errno(), "prctl(PR_SET_CHILD_SUBREAPER)")


def sweep(directories):
    """Kills every process this one started, and every process whose executable lies under
    one of `directories`, with their descendants."""
    roots = owned(directories)
    for _ in range(SWEEP_ROUNDS):
        doomed = belonging(processes(), os.getpid(), roots)
        if not doomed:
            return
        for pid in doomed:
            kill(pid)
        reap()
        time.sleep(SWEEP_PAUSE)
    print(f"Processes {sorted(doomed)} outlived the stage's cleanup.", file=sys.stderr)


def owned(directories):
    """Each directory as given and resolved, normalized for comparison, ending in a
    separator."""
    return tuple({
        os.path.join(os.path.normcase(path), "")
        for directory in directories
        for path in (os.path.abspath(directory), os.path.realpath(directory))
    })


def belonging(table, me, roots):
    """The processes in `table`, other than `me` and its ancestors, that descend from `me`
    or whose executable lies under one of `roots`, with their descendants."""
    children = defaultdict(list)
    for pid, process in table.items():
        parent = table.get(process.parent)
        if parent is not None and pid != process.parent and started_after(parent, process):
            children[process.parent].append(pid)
    found = set()
    pending = [me] + [
        pid for pid, process in table.items()
        if process.executable is not None
        and os.path.normcase(process.executable).startswith(roots)
    ]
    while pending:
        pid = pending.pop()
        if pid not in found:
            found.add(pid)
            pending.extend(children[pid])
    ancestors = set()
    ancestor = me
    while ancestor is not None and ancestor not in ancestors:
        ancestors.add(ancestor)
        ancestor = table[ancestor].parent if ancestor in table else None
    return found - ancestors


def started_after(parent, child):
    """Whether `parent` can be `child`'s parent, rather than a later process that reused
    the ID of a parent that exited."""
    if parent.created is None or child.created is None:
        return parent.created is None and child.created is None
    return parent.created <= child.created


def processes():
    """Every live process, by ID."""
    if sys.platform == "linux":
        return linux_processes()
    if os.name == "nt":
        return windows_processes()
    return ps_processes()


def linux_processes():
    table = {}
    for entry in os.scandir("/proc"):
        if not entry.name.isdigit():
            continue
        try:
            fields = Path(entry.path, "stat").read_text().rsplit(")", 1)[1].split()
        except OSError:
            continue
        state, parent = fields[:2]
        if state == "Z":
            continue
        try:
            executable = os.readlink(Path(entry.path, "exe"))
        except OSError:
            executable = None
        table[int(entry.name)] = Process(int(parent), executable, None)
    return table


def ps_processes():
    """macOS, where `ps` gives the executable's path as `comm`."""
    listing = subprocess.run(
        ["ps", "-axww", "-o", "pid=,ppid=,stat=,comm="],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    table = {}
    for line in listing.splitlines():
        pid, parent, state, executable = line.split(None, 3)
        if not state.startswith("Z"):
            table[int(pid)] = Process(int(parent), executable, None)
    return table


@functools.cache
def windows():
    """kernel32, typed for the calls here, and the Toolhelp process entry."""
    from ctypes import wintypes

    class Entry(ctypes.Structure):
        _fields_ = [
            ("size", wintypes.DWORD),
            ("usage", wintypes.DWORD),
            ("pid", wintypes.DWORD),
            ("heap", ctypes.c_size_t),
            ("module", wintypes.DWORD),
            ("threads", wintypes.DWORD),
            ("parent", wintypes.DWORD),
            ("priority", ctypes.c_long),
            ("flags", wintypes.DWORD),
            ("name", ctypes.c_wchar * 260),
        ]

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    handle, dword, pointer = wintypes.HANDLE, wintypes.DWORD, ctypes.c_void_p
    signatures = {
        "CreateToolhelp32Snapshot": (handle, [dword, dword]),
        "Process32FirstW": (wintypes.BOOL, [handle, ctypes.POINTER(Entry)]),
        "Process32NextW": (wintypes.BOOL, [handle, ctypes.POINTER(Entry)]),
        "OpenProcess": (handle, [dword, wintypes.BOOL, dword]),
        "GetExitCodeProcess": (wintypes.BOOL, [handle, ctypes.POINTER(dword)]),
        "QueryFullProcessImageNameW": (
            wintypes.BOOL, [handle, dword, wintypes.LPWSTR, ctypes.POINTER(dword)]
        ),
        "GetProcessTimes": (wintypes.BOOL, [handle, pointer, pointer, pointer, pointer]),
        "TerminateProcess": (wintypes.BOOL, [handle, wintypes.UINT]),
        "CloseHandle": (wintypes.BOOL, [handle]),
    }
    for function, (result, arguments) in signatures.items():
        getattr(kernel32, function).restype = result
        getattr(kernel32, function).argtypes = arguments
    return kernel32, Entry


def windows_processes():
    kernel32, Entry = windows()
    snapshot = kernel32.CreateToolhelp32Snapshot(0x2, 0)  # TH32CS_SNAPPROCESS
    if snapshot is None or snapshot == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    table = {}
    entry = Entry(size=ctypes.sizeof(Entry))
    try:
        listed = kernel32.Process32FirstW(snapshot, ctypes.byref(entry))
        while listed:
            details = windows_details(entry.pid)
            if details is not None:
                table[entry.pid] = Process(entry.parent, *details)
            listed = kernel32.Process32NextW(snapshot, ctypes.byref(entry))
    finally:
        kernel32.CloseHandle(snapshot)
    return table


def windows_details(pid):
    """`(executable, created)`, both None for a process this one cannot open, or None for
    one that has exited."""
    from ctypes import wintypes

    kernel32, _ = windows()
    handle = kernel32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
    if not handle:
        return None, None
    try:
        code = wintypes.DWORD()
        if kernel32.GetExitCodeProcess(handle, ctypes.byref(code)):
            if code.value != WINDOWS_STILL_ACTIVE:
                return None
        path = ctypes.create_unicode_buffer(32768)
        size = wintypes.DWORD(len(path))
        executable = None
        if kernel32.QueryFullProcessImageNameW(handle, 0, path, ctypes.byref(size)):
            executable = path.value
        times = [wintypes.FILETIME() for _ in range(4)]
        created = None
        if kernel32.GetProcessTimes(handle, *map(ctypes.byref, times)):
            created = (times[0].dwHighDateTime << 32) | times[0].dwLowDateTime
        return executable, created
    finally:
        kernel32.CloseHandle(handle)


def kill(pid):
    if os.name != "nt":
        with suppress(ProcessLookupError, PermissionError):
            os.kill(pid, signal.SIGKILL)
        return
    kernel32, _ = windows()
    handle = kernel32.OpenProcess(0x1, False, pid)  # PROCESS_TERMINATE
    if handle:
        kernel32.TerminateProcess(handle, 1)
        kernel32.CloseHandle(handle)


def reap():
    """Collects killed children, adopted orphans among them, so none lingers as a zombie."""
    if os.name == "nt":
        return
    with suppress(ChildProcessError):
        while os.waitpid(-1, os.WNOHANG)[0]:
            pass


def measure(root, cargo, timeout_factor, output, sources):
    runs = suite(cargo)
    for _, command in runs:
        subprocess.run(command + ["--no-run"], check=True)
    timeouts = {}
    if sources:
        for stage, command in earlier(cargo, [root / source for source in sources]):
            seconds = timed(command)
            timeouts[stage] = max(MINIMUM_TIMEOUT, timeout_factor * seconds)
            print(f"Stage `{stage}` took {seconds:.1f}s unloaded.", file=sys.stderr)
        seconds = sum(timed(command) for _, command in runs)
        timeouts[SUITE] = max(MINIMUM_TIMEOUT, timeout_factor * seconds)
        print(f"The suite took {seconds:.1f}s unloaded.", file=sys.stderr)
    output.write_text(json.dumps(timeouts))


def timed(command):
    started = time.monotonic()
    code = contain(command, None)
    if code:
        raise subprocess.CalledProcessError(code, command)
    return time.monotonic() - started


def main(arguments):
    adopt_orphans()
    if not arguments:
        return run()
    root, cargo, timeout_factor, output, *sources = arguments
    measure(Path(root), cargo, float(timeout_factor), Path(output), sources)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
