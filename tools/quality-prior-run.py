"""Find a passed own-hardware gate run of the same code as a push to main (#442, #447).

A squash-merge of an up-to-date pull request gives main the tree its branch's
head already had, and `quality.yml` gated that head on its push. This script
looks for such a run: a successful `quality.yml` push run on a branch other
than main whose head commit has the given tree, and in which every gate job
that `CLIMON2_OWN_RUNNERS` lists passed. A platform it does not list runs no
gate on the branch or on main, so it never stops a match.

A branch whose last push changed only paths `quality.yml` ignores has no run of
the merged tree. So when no run has the tree, the script asks GitHub which pull
request the push merged, takes the latest passed run on that pull request's
branch, and relies on it when its commit differs from the pushed one only in
paths `quality.yml` ignores. It fetches that commit, alone, to compare them.

On a match it writes `run=<the run's URL>` to `$GITHUB_OUTPUT`, and the gate
jobs on main skip. Anything else (no match, a match whose listed gates did not
all pass, or an API or git error) writes no output, so the gates run.

Each outcome is printed and added to `$GITHUB_STEP_SUMMARY`.
Run with `python3 tools/quality-prior-run.py --repo OWNER/NAME --tree TREE
--commit SHA --own-runners "$CLIMON2_OWN_RUNNERS"`, in a clone that has SHA.
"""

import argparse
import json
import os
import subprocess
import sys

from quality_paths import gated

# Each platform's key in CLIMON2_OWN_RUNNERS and its gate job's name in quality.yml.
GATES = (
    ("linux", "Linux CRAP and mutation / own hardware"),
    ("windows", "Windows-only CRAP and mutation / own hardware"),
    ("macos", "macOS-only CRAP and mutation / own hardware"),
)
RUNS = "repos/{repo}/actions/workflows/quality.yml/runs?event=push&status=success&per_page=100"
JOBS = "repos/{repo}/actions/runs/{run}/jobs?filter=latest&per_page=100"
PULLS = "repos/{repo}/commits/{commit}/pulls"


class ApiError(Exception):
    pass


def gh_api(path):
    """The JSON GitHub returns for `path`, through `gh api`."""
    try:
        result = subprocess.run(
            ["gh", "api", path], capture_output=True, text=True, timeout=60, check=False
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise ApiError(f"gh api {path}: {error}") from error
    if result.returncode != 0:
        raise ApiError(f"gh api {path}: exit {result.returncode}: {result.stderr.strip()}")
    try:
        return json.loads(result.stdout)
    except ValueError as error:
        raise ApiError(f"gh api {path}: {error}") from error


def fixture_api(file):
    """Answers from a JSON file mapping each API path to its response, for tests."""
    with open(file, encoding="utf-8") as handle:
        responses = json.load(handle)

    def api(path):
        if path not in responses:
            raise ApiError(f"no response for {path}")
        return responses[path]

    return api


def git(*args):
    """git's output, or None when it fails."""
    try:
        result = subprocess.run(
            ["git", *args], capture_output=True, text=True, timeout=120, check=False
        )
    except (OSError, subprocess.SubprocessError):
        return None
    return result.stdout if result.returncode == 0 else None


def has_commit(sha):
    """Whether the clone has commit `sha`, fetching it alone when it does not."""
    if git("cat-file", "-e", f"{sha}^{{commit}}") is not None:
        return True
    return git("fetch", "--quiet", "--no-tags", "--depth=1", "origin", sha) is not None


def ignored_difference(base, commit):
    """The paths in which `commit` differs from `base` when quality.yml ignores every one
    of them, or None when it gates one, or git cannot compare them."""
    if not (has_commit(base) and has_commit(commit)):
        return None
    changed = git("diff", "--name-only", "-z", base, commit)
    if changed is None:
        return None
    paths = [path for path in changed.split("\0") if path]
    return None if any(gated(path) for path in paths) else paths


def required_gates(own_runners):
    """The gate jobs that must have passed: those of the platforms CLIMON2_OWN_RUNNERS
    lists, matched as quality.yml's contains() matches them."""
    listed = own_runners.lower()
    return [name for key, name in GATES if key in listed]


def merged_branch(repo, commit, api):
    """The branch of the pull request whose merge made `commit`, or None."""
    for pull in api(PULLS.format(repo=repo, commit=commit)):
        if pull.get("merge_commit_sha") == commit:
            return (pull.get("head") or {}).get("ref")
    return None


def described(run):
    return f"{run['html_url']} ({run['head_branch']} at {run['head_sha']})"


def find(tree, commit, repo, api, required):
    """(the run relied on or None, why) for a push to main of `commit`, whose tree is `tree`."""

    def passed(run):
        jobs = api(JOBS.format(repo=repo, run=run["id"]))["jobs"]
        conclusions = {job.get("name"): job.get("conclusion") for job in jobs}
        return all(conclusions.get(gate) == "success" for gate in required)

    try:
        runs = [
            run
            for run in api(RUNS.format(repo=repo))["workflow_runs"]
            if run.get("head_branch") != "main"
            and run.get("event") == "push"
            and run.get("status") == "completed"
            and run.get("conclusion") == "success"
        ]
        partial = []
        for run in runs:
            if (run.get("head_commit") or {}).get("tree_id") != tree:
                continue
            if passed(run):
                return run, f"tree {tree} passed every listed gate in {described(run)}"
            partial.append(run["html_url"])
        branch = merged_branch(repo, commit, api)
        latest = next((run for run in runs if run.get("head_branch") == branch), None)
        if latest is not None and (latest.get("head_commit") or {}).get("tree_id") != tree:
            paths = ignored_difference(latest["head_sha"], commit)
            if paths is not None and passed(latest):
                return latest, (
                    f"tree {tree} differs only in paths quality.yml ignores"
                    f" ({', '.join(paths)}) from the one that passed every listed gate"
                    f" in {described(latest)}"
                )
    except (ApiError, KeyError, TypeError, AttributeError) as error:
        return None, f"could not look up passed runs: {error}"
    if partial:
        return None, f"tree {tree} did not pass every listed gate in {', '.join(partial)}"
    return None, (
        f"no passed run on another branch has tree {tree},"
        " or one that differs from it only in paths quality.yml ignores"
    )


def append(variable, text):
    path = os.environ.get(variable)
    if path:
        with open(path, "a", encoding="utf-8") as handle:
            handle.write(text)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", required=True)
    parser.add_argument("--tree", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--own-runners", default="")
    parser.add_argument("--fixtures", help="answer API calls from this JSON file (tests)")
    args = parser.parse_args()
    api = fixture_api(args.fixtures) if args.fixtures else gh_api
    run, why = find(args.tree, args.commit, args.repo, api, required_gates(args.own_runners))
    if run:
        message = f"Own-hardware gates skipped: {why}."
        append("GITHUB_OUTPUT", f"run={run['html_url']}\n")
    else:
        message = f"Own-hardware gates run: {why}."
    print(message)
    append("GITHUB_STEP_SUMMARY", f"- {message}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
