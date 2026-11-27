"""Decide, start and report the full mutation pass on main (#452).

Every 10th pull request squash-merged to main starts a full pass: the
own-hardware gates of `quality-full.yml` mutate every shipped file, whole, on
Linux, Windows and macOS. This script holds its three decisions:

- `due`: whether a push to main brings it to a multiple of 10 squash-merges.
  A squash-merge is a first-parent commit of main whose subject ends with the
  pull request's number, such as "Fix the frame (#432) (#438)". The count is
  taken from git history alone, so no stored counter is needed: the last full
  pass was at the latest multiple of 10. A manual dispatch is always due.
  Writes `due=true` or `due=false` to `$GITHUB_OUTPUT`.
- `wait`: waits until no `quality.yml` run is queued or in progress, so a pass
  starts behind the gates already waiting for the machines, and stops waiting
  after `--limit` seconds or an API error.
- `report`: reads each platform's gate log and exit status and puts its CRAP,
  SURVIVED and UNCOVERED findings, and any failure without findings, into one
  open tracking issue, which it opens or updates.

Each outcome is printed and added to `$GITHUB_STEP_SUMMARY`.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

# A full pass starts on every PERIOD-th squash-merge.
PERIOD = 10
SQUASH = re.compile(r"\(#\d+\)$")
ZERO = "0" * 40
# The statuses of a quality.yml run that has not finished.
UNFINISHED = ("queued", "in_progress", "pending", "waiting", "requested")
RUNS = "repos/{repo}/actions/workflows/quality.yml/runs?status={status}&per_page=1"
# The tracking issue: the one open issue the workflow's token opened with this title.
TITLE = "Full mutation pass findings"
BOT = "github-actions[bot]"
ISSUES = "repos/{repo}/issues?state=open&creator=github-actions%5Bbot%5D&per_page=100"
PLATFORMS = (("linux", "Linux"), ("windows", "Windows"), ("macos", "macOS"))
CRAP_THRESHOLD = 5.0
MUTANT = re.compile(r"^(SURVIVED|UNCOVERED) +\S+:\d+ ")
# GitHub refuses an issue body over 65,536 characters.
BODY_LIMIT = 60000


class ApiError(Exception):
    pass


def git(*args):
    result = subprocess.run(["git", *args], capture_output=True, text=True, check=False)
    return result.stdout if result.returncode == 0 else None


def squash_merges(commit):
    """Squash-merges on the first-parent history up to `commit`, or None if it is unknown."""
    if not commit or commit == ZERO or git("cat-file", "-e", f"{commit}^{{commit}}") is None:
        return None
    subjects = git("log", "--first-parent", "--format=%s", commit)
    if subjects is None:
        return None
    return sum(1 for subject in subjects.splitlines() if SQUASH.search(subject.strip()))


def due(event, before, after):
    """(whether a full pass is due, why) for a push from `before` to `after`."""
    if event == "workflow_dispatch":
        return True, "a manual run is always a full pass"
    now = squash_merges(after)
    if now is None:
        return False, f"{after} is not a commit here"
    then = squash_merges(before)
    if then is None:
        # A first push, or a force push whose old head is gone: count this push's own commit.
        parent = squash_merges(f"{after}^")
        then = parent if parent is not None else 0
    last = now // PERIOD * PERIOD
    if now // PERIOD > then // PERIOD:
        return True, f"main has {now} squash-merges, and the full pass at {last} is due"
    return False, (
        f"main has {now} squash-merges; the last full pass was due at {last}"
        f" and the next is at {last + PERIOD}"
    )


def gh_api(method, path, body=None):
    """The JSON GitHub returns for `method path`, through `gh api`."""
    command = ["gh", "api", "--method", method, path]
    if body is not None:
        command += ["--input", "-"]
    try:
        result = subprocess.run(
            command,
            input=None if body is None else json.dumps(body),
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise ApiError(f"gh api {path}: {error}") from error
    if result.returncode != 0:
        raise ApiError(f"gh api {path}: exit {result.returncode}: {result.stderr.strip()}")
    try:
        return json.loads(result.stdout or "{}")
    except ValueError as error:
        raise ApiError(f"gh api {path}: {error}") from error


def fixture_api(file, record):
    """For tests: answers GETs from a JSON file mapping each path to its response, and
    appends every other call to `record` as one JSON line."""
    with open(file, encoding="utf-8") as handle:
        responses = json.load(handle)

    def api(method, path, body=None):
        if method != "GET":
            with open(record, "a", encoding="utf-8") as handle:
                handle.write(json.dumps({"method": method, "path": path, "body": body}) + "\n")
            return {"html_url": f"https://github.com/{path}"}
        if path not in responses:
            raise ApiError(f"no response for {path}")
        return responses[path]

    return api


def busy(repo, api):
    """The number of quality.yml runs that have not finished."""
    return sum(
        api("GET", RUNS.format(repo=repo, status=status))["total_count"] for status in UNFINISHED
    )


def wait(repo, api, interval, limit, sleep=time.sleep):
    """Why the full pass may start: no gate run waits, the wait reached its limit, or the
    API failed. The pass always starts in the end, since a skipped one is never made up."""
    waited = 0
    while True:
        try:
            runs = busy(repo, api)
        except (ApiError, KeyError, TypeError) as error:
            return f"could not look up quality.yml runs, so it starts now: {error}"
        if runs == 0:
            return f"no quality.yml run was queued or in progress after {waited // 60} min"
        if waited >= limit:
            return f"{runs} quality.yml runs still unfinished after {waited // 60} min; it starts anyway"
        sleep(interval)
        waited += interval


def crap_findings(lines):
    """The CRAP report's rows over the threshold."""
    findings = []
    in_table = False
    for line in lines:
        if line.startswith("Function ") and line.rstrip().endswith("CRAP"):
            in_table = True
        elif in_table and not line.strip():
            in_table = False
        elif in_table and not line.startswith("---"):
            try:
                score = float(line.split()[-1])
            except (ValueError, IndexError):
                continue
            if score > CRAP_THRESHOLD:
                findings.append("CRAP " + " ".join(line.split()))
    return findings


def platform_result(folder):
    """(exit status or None, findings) from a platform's gate log and status files."""
    log = folder / "gate.log"
    lines = (
        log.read_text(encoding="utf-8", errors="replace").splitlines() if log.is_file() else []
    )
    findings = crap_findings(lines) + [line.rstrip() for line in lines if MUTANT.match(line)]
    status_file = folder / "status"
    status = status_file.read_text(encoding="utf-8").strip() if status_file.is_file() else None
    return status, findings


def report_body(logs, run_url, sha):
    """(the issue body, whether anything needs the owner's attention)."""
    sections = []
    attention = False
    for key, name in PLATFORMS:
        folder = logs / f"full-run-{key}"
        if not folder.is_dir():
            sections.append(f"### {name}\n\nDid not run.\n")
            continue
        status, findings = platform_result(folder)
        if status is None:
            outcome = "no exit status: the gate timed out or was cancelled"
        else:
            outcome = f"exit {status}"
        if findings:
            listed = "\n".join(findings)
            sections.append(
                f"### {name}: {len(findings)} findings, {outcome}\n\n```text\n{listed}\n```\n"
            )
            attention = True
        elif status != "0":
            sections.append(f"### {name}: no findings, {outcome}\n\nSee the run's log.\n")
            attention = True
        else:
            sections.append(f"### {name}: no findings\n")
    body = (
        f"The latest full mutation pass mutated every shipped file at {sha}: {run_url}\n\n"
        "This issue lists its findings and is updated by each full pass while it is open "
        "(docs/for-humans/quality-runners.md).\n\n" + "\n".join(sections)
    )
    if len(body) > BODY_LIMIT:
        body = body[:BODY_LIMIT] + f"\n```\n\nCut short; the full logs are in {run_url}.\n"
    return body, attention


def tracking_issue(repo, api):
    issues = api("GET", ISSUES.format(repo=repo))
    for issue in issues:
        if (
            issue.get("title") == TITLE
            and (issue.get("user") or {}).get("login") == BOT
            and "pull_request" not in issue
        ):
            return issue
    return None


def report(repo, api, logs, run_url, sha):
    """What the report did with the tracking issue."""
    body, attention = report_body(logs, run_url, sha)
    try:
        issue = tracking_issue(repo, api)
        if attention and issue:
            api("PATCH", f"repos/{repo}/issues/{issue['number']}", {"body": body})
            api(
                "POST",
                f"repos/{repo}/issues/{issue['number']}/comments",
                {"body": f"Updated with the findings of {run_url}."},
            )
            return f"Full pass findings updated in {issue['html_url']}"
        if attention:
            created = api("POST", f"repos/{repo}/issues", {"title": TITLE, "body": body})
            return f"Full pass findings opened in {created['html_url']}"
        if issue:
            api(
                "POST",
                f"repos/{repo}/issues/{issue['number']}/comments",
                {"body": f"{run_url} found nothing on any platform that ran."},
            )
            return f"Full pass found nothing; said so in {issue['html_url']}"
        return "Full pass found nothing"
    except (ApiError, KeyError, TypeError, AttributeError) as error:
        return f"Full pass could not update its tracking issue: {error}"


def append(variable, text):
    path = os.environ.get(variable)
    if path:
        with open(path, "a", encoding="utf-8") as handle:
            handle.write(text)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    due_command = commands.add_parser("due")
    due_command.add_argument("--event", required=True)
    due_command.add_argument("--before", default="")
    due_command.add_argument("--after", required=True)
    for name in ("wait", "report"):
        command = commands.add_parser(name)
        command.add_argument("--repo", required=True)
        command.add_argument("--fixtures", help="answer API calls from this JSON file (tests)")
        command.add_argument("--record", help="append API writes to this file (tests)")
    commands.choices["wait"].add_argument("--interval", type=int, default=300)
    commands.choices["wait"].add_argument("--limit", type=int, default=5 * 60 * 60)
    commands.choices["report"].add_argument("--logs", required=True, type=Path)
    commands.choices["report"].add_argument("--run-url", required=True)
    commands.choices["report"].add_argument("--sha", required=True)
    args = parser.parse_args()

    if args.command == "due":
        is_due, why = due(args.event, args.before, args.after)
        append("GITHUB_OUTPUT", f"due={'true' if is_due else 'false'}\n")
        message = f"Full mutation pass {'runs' if is_due else 'skipped'}: {why}."
    else:
        api = fixture_api(args.fixtures, args.record) if args.fixtures else gh_api
        if args.command == "wait":
            message = "Full mutation pass starts: " + wait(
                args.repo, api, args.interval, args.limit
            ) + "."
        else:
            message = report(args.repo, api, args.logs, args.run_url, args.sha) + "."
    print(message)
    append("GITHUB_STEP_SUMMARY", f"- {message}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
