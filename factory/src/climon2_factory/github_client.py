"""The GitHub port: candidate pushes, the Windows CI gate and delivery PRs, via git and gh."""

import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

from climon2_factory.ports import CiResult, TestReport
from climon2_factory.reports import (
    MAX_REPORT_BYTES,
    build_failed_report,
    collected_tests,
    failed_report,
    parse_report,
)

LABEL = "github"
WINDOWS_WORKFLOW = ".github/workflows/factory-windows.yml"
CANDIDATE_PREFIX = "factory/"
_IDENTITY = {
    "GIT_AUTHOR_NAME": "climon2 factory",
    "GIT_AUTHOR_EMAIL": "factory@climon2.invalid",
    "GIT_COMMITTER_NAME": "climon2 factory",
    "GIT_COMMITTER_EMAIL": "factory@climon2.invalid",
}


class GitHubError(RuntimeError):
    pass


class GitHubClient:
    def __init__(self, repo: str, poll_s: float = 30, ci_timeout_s: float = 3600) -> None:
        self._repo = repo
        self._poll_s = poll_s
        self._ci_timeout_s = ci_timeout_s

    def push_candidate(self, workspace: Path, branch: str) -> str:
        """Commit the workspace as it stands on top of HEAD and force-push it to `branch`.

        A temporary index is used, so the user's HEAD, index and working tree are untouched.
        Re-pushing the same branch (a resumed run) replaces the earlier candidate."""
        if not branch.startswith(CANDIDATE_PREFIX) or branch == CANDIDATE_PREFIX:
            raise ValueError(f"candidates are pushed only to {CANDIDATE_PREFIX}* branches")
        with tempfile.TemporaryDirectory() as tmp:
            env = {**os.environ, **_IDENTITY, "GIT_INDEX_FILE": str(Path(tmp) / "index")}
            _git(workspace, env, "read-tree", "HEAD")
            _git(workspace, env, "add", "-A", ".")
            tree = _git(workspace, env, "write-tree")
            message = f"factory candidate for {branch}"
            commit = _git(workspace, env, "commit-tree", tree, "-p", "HEAD", "-m", message)
        _git(workspace, None, "push", "-q", "--force", "origin", f"{commit}:refs/heads/{branch}")
        return commit

    def windows_ci(self, commit: str) -> CiResult:
        run = self._completed_run(commit)
        with tempfile.TemporaryDirectory() as tmp:
            download = ["run", "download", str(run["id"]), "--repo", self._repo, "--dir", tmp]
            self._gh(*download, check=False)
            report = _windows_report(Path(tmp), str(run["conclusion"]))
        return CiResult(str(run["head_sha"]), report)

    def changed_files(self, workspace: Path, commit: str) -> tuple[str, ...]:
        _git(workspace, None, "fetch", "-q", "origin", "main")
        diff = _git(workspace, None, "diff", "--name-only", f"origin/main...{commit}")
        return tuple(diff.splitlines())

    def open_pr(self, branch: str, title: str, body: str) -> int:
        url = self._gh(
            "pr",
            "create",
            "--repo",
            self._repo,
            "--base",
            "main",
            "--head",
            branch,
            "--title",
            title,
            "--body",
            body,
        )
        return int(url.rstrip("/").rsplit("/", 1)[1])

    def find_pr(self, branch: str) -> int | None:
        found = json.loads(
            self._gh(
                "pr",
                "list",
                "--repo",
                self._repo,
                "--head",
                branch,
                "--state",
                "all",
                "--json",
                "number",
            )
        )
        return int(found[0]["number"]) if found else None

    def pr_state(self, number: int) -> str:
        view = self._gh("pr", "view", str(number), "--repo", self._repo, "--json", "state")
        return str(json.loads(view)["state"]).lower()

    def _completed_run(self, commit: str) -> dict[str, object]:
        """The Windows workflow run for `commit`, once it has finished."""
        deadline = time.monotonic() + self._ci_timeout_s
        while True:
            found = json.loads(
                self._gh("api", f"repos/{self._repo}/actions/runs?head_sha={commit}&event=push")
            )
            runs = [r for r in found["workflow_runs"] if r["path"] == WINDOWS_WORKFLOW]
            latest = max(runs, key=lambda r: r["id"], default=None)
            if latest is not None and latest["status"] == "completed":
                return latest
            if time.monotonic() > deadline:
                raise GitHubError(f"Windows CI for {commit} did not finish in time")
            time.sleep(self._poll_s)

    def _gh(self, *args: str, check: bool = True) -> str:
        result = subprocess.run(["gh", *args], capture_output=True, text=True, check=False)
        if check and result.returncode != 0:
            raise GitHubError(f"gh {args[0]} {args[1]} failed: {result.stderr.strip()}")
        return result.stdout.strip()


def _windows_report(artifacts: Path, conclusion: str) -> TestReport:
    collected_dir = artifacts / "collected"
    results = artifacts / "results"
    if not (collected_dir / "exit-code.txt").is_file():
        return failed_report((), f"Windows CI produced no results ({conclusion})", LABEL)
    collect_exit = _int(collected_dir / "exit-code.txt")
    collect_output = _text(collected_dir / "collected.txt")
    if collect_exit not in (0, 5):
        reason = f"collection failed (exit {collect_exit}):\n{collect_output}"
        return failed_report((), reason, LABEL, fallback="<collect>")
    expected = collected_tests(collect_output) if collect_exit == 0 else ()
    build_exit = _int(results / "build-exit-code.txt")
    if build_exit is None:
        return failed_report(expected, f"Windows CI produced no build result ({conclusion})", LABEL)
    if build_exit != 0:
        return build_failed_report(expected, _text(results / "build.log"), LABEL)
    test_exit = _int(results / "exit-code.txt")
    if test_exit is None:
        return failed_report(expected, f"Windows CI produced no test results ({conclusion})", LABEL)
    report = results / "report.xml"
    xml = report.read_bytes()[: MAX_REPORT_BYTES + 1] if report.is_file() else b""
    return parse_report(xml, expected, test_exit, LABEL)


def _text(path: Path) -> str:
    return path.read_text(errors="replace")[-20_000:] if path.is_file() else ""


def _int(path: Path) -> int | None:
    try:
        return int(path.read_text().strip())
    except (OSError, ValueError):
        return None


def _git(workspace: Path, env: dict[str, str] | None, *args: str) -> str:
    result = subprocess.run(
        ["git", *args], cwd=workspace, env=env, capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise GitHubError(f"git {args[0]} failed: {result.stderr.strip()}")
    return result.stdout.strip()
