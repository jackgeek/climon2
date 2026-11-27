"""Find a passed own-hardware gate run on the same tree as a push to main (#442).

A squash-merge of an up-to-date pull request gives main the tree its branch's
head already had, and `quality.yml` gated that head on its push. This script
looks for such a run: a successful `quality.yml` push run on a branch other
than main whose head commit has the given tree, and in which every gate job
passed. On a match it writes `run=<the run's URL>` to `$GITHUB_OUTPUT`, and the
gate jobs on main skip. Anything else (no match, a match whose gates did not
all pass, or an API error) writes no output, so the gates run.

Each outcome is printed and added to `$GITHUB_STEP_SUMMARY`.
Run with `python3 tools/quality-prior-run.py --repo OWNER/NAME --tree TREE`.
"""

import argparse
import json
import os
import subprocess
import sys

# The gate jobs' names in quality.yml; each must have passed in the run relied on.
GATES = (
    "Linux CRAP and mutation / own hardware",
    "Windows-only CRAP and mutation / own hardware",
    "macOS-only CRAP and mutation / own hardware",
)
RUNS = "repos/{repo}/actions/workflows/quality.yml/runs?event=push&status=success&per_page=100"
JOBS = "repos/{repo}/actions/runs/{run}/jobs?filter=latest&per_page=100"


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


def find(tree, repo, api):
    """(the run relied on or None, why) for a push to main whose tree is `tree`."""
    try:
        runs = api(RUNS.format(repo=repo))["workflow_runs"]
        candidates = [
            run
            for run in runs
            if run.get("head_branch") != "main"
            and run.get("event") == "push"
            and run.get("status") == "completed"
            and run.get("conclusion") == "success"
            and (run.get("head_commit") or {}).get("tree_id") == tree
        ]
        if not candidates:
            return None, f"no passed run on another branch has tree {tree}"
        partial = []
        for run in candidates:
            jobs = api(JOBS.format(repo=repo, run=run["id"]))["jobs"]
            conclusions = {job.get("name"): job.get("conclusion") for job in jobs}
            if all(conclusions.get(gate) == "success" for gate in GATES):
                return run, (
                    f"tree {tree} passed every gate in {run['html_url']}"
                    f" ({run['head_branch']} at {run['head_sha']})"
                )
            partial.append(run["html_url"])
    except (ApiError, KeyError, TypeError, AttributeError) as error:
        return None, f"could not look up passed runs: {error}"
    return None, f"tree {tree} did not pass every gate in {', '.join(partial)}"


def append(variable, text):
    path = os.environ.get(variable)
    if path:
        with open(path, "a", encoding="utf-8") as handle:
            handle.write(text)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", required=True)
    parser.add_argument("--tree", required=True)
    parser.add_argument("--fixtures", help="answer API calls from this JSON file (tests)")
    args = parser.parse_args()
    api = fixture_api(args.fixtures) if args.fixtures else gh_api
    run, why = find(args.tree, args.repo, api)
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
