"""Choose and report the rolling full mutation pass on main (#452).

Each pull request squash-merged to main mutates one tenth of the shipped files,
whole, on Linux, Windows and macOS (`quality-full.yml`). A file's slice comes
from a hash of its path (`cargo xtask mutate --mutate-slice`), so ten merges in
a row mutate every file once. This script holds the workflow's two decisions:

- `slice`: which slice a push to main mutates. A squash-merge is a
  first-parent commit of main whose subject ends with the pull request's
  number, such as "Fix the frame (#432) (#438)". The merge that takes main to
  n squash-merges mutates slice n mod 10, so the rotation is read from git
  history alone and needs no stored counter. A push that adds no squash-merge
  mutates nothing. A manual run mutates the slice it names, or all of them.
  Writes `slice=<0-9 or all>`, or an empty `slice=` for none, to
  `$GITHUB_OUTPUT`.
- `report`: reads each platform's log and exit status and puts its CRAP,
  SURVIVED and UNCOVERED findings, and any failure or timeout without
  findings, into the slice's section of one open tracking issue, which it
  opens or updates.

Each outcome is printed and added to `$GITHUB_STEP_SUMMARY`.
"""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

SLICES = 10
LABELS = [str(index) for index in range(SLICES)] + ["all"]
SQUASH = re.compile(r"\(#\d+\)$")
ZERO = "0" * 40
# The tracking issue: the one open issue the workflow's token opened with this title.
TITLE = "Full mutation pass findings"
BOT = "github-actions[bot]"
ISSUES = "repos/{repo}/issues?state=open&creator=github-actions%5Bbot%5D&per_page=100"
PLATFORMS = (("linux", "Linux"), ("windows", "Windows"), ("macos", "macOS"))
CRAP_THRESHOLD = 5.0
MUTANT = re.compile(r"^(SURVIVED|UNCOVERED) +\S+:\d+ ")
SECTION = re.compile(r"<!-- slice (\w+) -->\n.*?<!-- end slice \1 -->\n", re.S)
HEADER = (
    "Each squash-merge to main mutates one slice of the shipped files, so ten in a row "
    "mutate every file (docs/for-humans/quality-runners.md). Each slice's section below "
    "holds its latest pass's findings and is replaced by its next pass.\n\n"
)
# GitHub refuses an issue body over 65,536 characters; each of the 11 sections gets a share.
SECTION_LIMIT = 5500


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


def choose(event, requested, before, after):
    """(the slice to mutate or "" for none, why) for a push or manual run at `after`."""
    if event == "workflow_dispatch":
        if requested not in LABELS:
            return "", f"a manual run names slice 0 to 9 or all, not {requested!r}"
        return requested, f"a manual run asked for slice {requested}"
    now = squash_merges(after)
    if now is None:
        return "", f"{after} is not a commit here"
    then = squash_merges(before)
    if then is None:
        # A first push, or a force push whose old head is gone: count this push's own commit.
        parent = squash_merges(f"{after}^")
        then = parent if parent is not None else 0
    if now <= then:
        return "", f"the push adds no squash-merge; main still has {now}"
    chosen = str(now % SLICES)
    why = f"main has {now} squash-merges, so slice {chosen} of {SLICES}"
    if now - then > 1:
        why += (
            f"; the push adds {now - then}, and the slices of the others wait"
            " for their next turn"
        )
    return chosen, why


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
    """(exit status or None, findings) from a platform's log and status files."""
    log = folder / "gate.log"
    lines = (
        log.read_text(encoding="utf-8", errors="replace").splitlines() if log.is_file() else []
    )
    findings = crap_findings(lines) + [line.rstrip() for line in lines if MUTANT.match(line)]
    status_file = folder / "status"
    status = status_file.read_text(encoding="utf-8").strip() if status_file.is_file() else None
    return status, findings


def listed(findings, run_url, room):
    """The findings in a text block of at most about `room` characters."""
    shown = []
    used = 0
    for finding in findings:
        if used + len(finding) + 1 > room:
            break
        shown.append(finding)
        used += len(finding) + 1
    text = "```text\n" + "\n".join(shown) + "\n```\n"
    if len(shown) < len(findings):
        text += f"\nAnd {len(findings) - len(shown)} more in {run_url}.\n"
    return text


def section(logs, label, run_url, sha):
    """(the slice's section of the issue body, whether it needs the owner's attention)."""
    title = "All slices" if label == "all" else f"Slice {label} of {SLICES}"
    parts = [f"## {title}, at {sha}\n\n{run_url}\n"]
    attention = False
    room = SECTION_LIMIT // len(PLATFORMS) - 200
    for key, name in PLATFORMS:
        folder = logs / f"full-run-{key}"
        if not folder.is_dir():
            parts.append(f"### {name}\n\nDid not run.\n")
            continue
        status, findings = platform_result(folder)
        if status is None:
            outcome = "no exit status: the slice timed out or was cancelled"
        else:
            outcome = f"exit {status}"
        if findings:
            parts.append(
                f"### {name}: {len(findings)} findings, {outcome}\n\n"
                + listed(findings, run_url, room)
            )
            attention = True
        elif status != "0":
            parts.append(f"### {name}: no findings, {outcome}\n\nSee the run's log.\n")
            attention = True
        else:
            parts.append(f"### {name}: no findings\n")
    text = "\n".join(parts)
    return f"<!-- slice {label} -->\n{text}<!-- end slice {label} -->\n", attention


def merged(body, label, new):
    """The issue body with slice `label`'s section replaced by `new`, slices in order."""
    sections = {match.group(1): match.group(0) for match in SECTION.finditer(body or "")}
    sections[label] = new
    return HEADER + "\n".join(sections[key] for key in LABELS if key in sections)


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


def report(repo, api, logs, label, run_url, sha):
    """What the report did with the tracking issue."""
    new, attention = section(logs, label, run_url, sha)
    name = "all slices" if label == "all" else f"slice {label}"
    try:
        issue = tracking_issue(repo, api)
        if issue:
            # A clean pass clears the slice's old findings, so the issue keeps only open ones.
            body = merged(issue.get("body"), label, new)
            api("PATCH", f"repos/{repo}/issues/{issue['number']}", {"body": body})
            if attention:
                api(
                    "POST",
                    f"repos/{repo}/issues/{issue['number']}/comments",
                    {"body": f"New findings in {name}: {run_url}"},
                )
                return f"Findings in {name} updated in {issue['html_url']}"
            return f"No findings in {name}; its section in {issue['html_url']} is cleared"
        if attention:
            body = merged("", label, new)
            created = api("POST", f"repos/{repo}/issues", {"title": TITLE, "body": body})
            return f"Findings in {name} opened in {created['html_url']}"
        return f"No findings in {name}"
    except (ApiError, KeyError, TypeError, AttributeError) as error:
        return f"Could not update the tracking issue for {name}: {error}"


def append(variable, text):
    path = os.environ.get(variable)
    if path:
        with open(path, "a", encoding="utf-8") as handle:
            handle.write(text)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    choose_command = commands.add_parser("slice")
    choose_command.add_argument("--event", required=True)
    choose_command.add_argument("--requested", default="")
    choose_command.add_argument("--before", default="")
    choose_command.add_argument("--after", required=True)
    report_command = commands.add_parser("report")
    report_command.add_argument("--repo", required=True)
    report_command.add_argument("--slice", required=True, choices=LABELS)
    report_command.add_argument("--logs", required=True, type=Path)
    report_command.add_argument("--run-url", required=True)
    report_command.add_argument("--sha", required=True)
    report_command.add_argument("--fixtures", help="answer API calls from this JSON file (tests)")
    report_command.add_argument("--record", help="append API writes to this file (tests)")
    args = parser.parse_args()

    if args.command == "slice":
        chosen, why = choose(args.event, args.requested, args.before, args.after)
        append("GITHUB_OUTPUT", f"slice={chosen}\n")
        if chosen:
            target = "all slices" if chosen == "all" else f"slice {chosen}"
            message = f"Full mutation pass mutates {target}: {why}."
        else:
            message = f"Full mutation pass skipped: {why}."
    else:
        api = fixture_api(args.fixtures, args.record) if args.fixtures else gh_api
        message = report(args.repo, api, args.logs, args.slice, args.run_url, args.sha) + "."
    print(message)
    append("GITHUB_STEP_SUMMARY", f"- {message}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
