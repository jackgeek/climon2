"""Choose, start and report the rolling full mutation pass on main (#452).

Each pull request squash-merged to main mutates one tenth of the shipped files,
whole, on Linux, Windows and macOS (`quality-full.yml`). A file's slice comes
from a hash of its path (`cargo xtask mutate --mutate-slice`), so ten merges in
a row mutate every file once. This script holds the workflow's decisions:

- `slice`: which slices a push to main mutates. A squash-merge is a
  first-parent commit of main whose subject ends with the pull request's
  number, such as "Fix the frame (#432) (#438)". Only the subject counts, so a
  direct push whose subject ends that way counts too (#457). The merge that
  takes main to n squash-merges mutates slice n mod 10, so the rotation is read
  from git history alone and needs no stored counter. A push that adds no
  squash-merge mutates nothing; after a force push, which may lower the count,
  only the commits the push brought count as added. A manual run mutates the
  slice it names, or all ten.
  Writes the slices as a JSON list, such as `slices=["3"]`, or an empty
  `slices=` for none, to `$GITHUB_OUTPUT`, and `gate=` the pushed commit when
  its push runs quality.yml, whose own run each slice then waits for.
- `wait`: waits, on a hosted runner, until no `quality.yml` run and no older
  `quality-full.yml` run is queued or in progress, and main's own `quality.yml`
  run for `--gate` has appeared, so a slice never holds a machine that a gate
  or an earlier slice is waiting for. It makes at most two API calls a check. Writes `idle=true`
  when they are idle, or `idle=false` when they were still busy, or GitHub's
  API still failed, after `--limit` seconds.
- `start`: from the wait jobs' results, whether the gates went idle (`idle`),
  stayed busy (`busy`), or a wait job failed (`failed`). Writes `state=`.
- `report`: reads each platform's log and status and puts its CRAP, SURVIVED
  and UNCOVERED findings, and any failure, timeout or missing log, into each
  slice's section of one open tracking issue, which it opens or updates. A
  slice or platform that did not run keeps its earlier result, with a dated
  note saying why it did not run this time. Report jobs share one concurrency
  group in `quality-slice.yml`, so two never write the issue at once.

Each outcome is printed and added to `$GITHUB_STEP_SUMMARY`.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time
from collections import namedtuple
from pathlib import Path

from quality_paths import gated

SLICES = 10
LABELS = [str(index) for index in range(SLICES)]
SQUASH = re.compile(r"\(#\d+\)$")
ZERO = "0" * 40
# The statuses of a run that has not finished.
UNFINISHED = ("queued", "in_progress", "pending", "waiting", "requested")
# The latest runs of each workflow, newest first, whatever their status.
GATE_RUNS = "repos/{repo}/actions/workflows/quality.yml/runs?per_page=100"
FULL_RUNS = "repos/{repo}/actions/workflows/quality-full.yml/runs?per_page=100"
# The tracking issue: the one open issue the workflow's token opened with this title.
TITLE = "Full mutation pass findings"
BOT = "github-actions[bot]"
ISSUES = "repos/{repo}/issues?state=open&creator=github-actions%5Bbot%5D&per_page=100&page={page}"
ISSUES_PER_PAGE = 100
# Enough pages for 10,000 of the bot's open issues; the search stops at the first short page.
ISSUE_PAGES = 100
PLATFORMS = (("linux", "Linux"), ("windows", "Windows"), ("macos", "macOS"))
CRAP_THRESHOLD = 5.0
MUTANT = re.compile(r"^(SURVIVED|UNCOVERED) +\S+:\d+ ")
# A section may lack its final newline once the body has been edited in the browser.
SECTION = re.compile(r"<!-- slice (\w+) -->\n(.*?)<!-- end slice \1 -->(?:\n|\Z)", re.S)
PLATFORM = re.compile(r"<!-- platform (\w+) -->\n(.*?)<!-- end platform \1 -->(?:\n|\Z)", re.S)
# The dated note of why a slice or platform did not run; the next note replaces it.
NOTE = re.compile(r"\A> (Skipped|Did not run) on .*\n\n?")
HEADER = (
    "Each squash-merge to main mutates one slice of the shipped files, so ten in a row "
    "mutate every file (docs/for-humans/quality-runners.md). Each slice's section below "
    "holds its latest pass's findings and is replaced by its next pass.\n\n"
)
# GitHub refuses an issue body over 65,536 characters; each of the 10 sections gets a share.
SECTION_LIMIT = 6000
# What a platform's status file says before its pass records an exit status.
SETUP_FAILED = "setup-failed"
STARTED = "started"


# What a report knows of its run: the platforms' logs, the run's link, CLIMON2_OWN_RUNNERS,
# today's date and the commit.
Run = namedtuple("Run", "logs url own date sha")


class ApiError(Exception):
    pass


def git(*args):
    result = subprocess.run(["git", *args], capture_output=True, text=True, check=False)
    return result.stdout if result.returncode == 0 else None


def squash_merges(commit, since=None):
    """Squash-merges on the first-parent history up to `commit`, and not in `since`'s
    history, or None if it is unknown. Only the subject counts: see the module's doc."""
    if not commit or commit == ZERO or git("cat-file", "-e", f"{commit}^{{commit}}") is None:
        return None
    excluded = [f"^{since}"] if since else []
    subjects = git("log", "--first-parent", "--format=%s", commit, *excluded)
    if subjects is None:
        return None
    return sum(1 for subject in subjects.splitlines() if SQUASH.search(subject.strip()))


def choose(event, requested, before, after):
    """(the slices to mutate, none for nothing, and why) for a push or manual run."""
    if event == "workflow_dispatch":
        if requested == "all":
            return LABELS, "a manual run asked for all of them"
        if requested not in LABELS:
            return [], f"a manual run names slice 0 to 9 or all, not {requested!r}"
        return [requested], f"a manual run asked for slice {requested}"
    now = squash_merges(after)
    if now is None:
        return [], f"{after} is not a commit here"
    then = squash_merges(before)
    if then is None:
        # A first push, or a force push whose old head is gone: count this push's own commit.
        parent = squash_merges(f"{after}^")
        then = parent if parent is not None else 0
    elif git("merge-base", "--is-ancestor", before, after) is None:
        # A force push that replaced the old head: count only the commits it brought.
        then = now - (squash_merges(after, since=before) or 0)
    if now <= then:
        return [], f"the push adds no squash-merge; main still has {now}"
    chosen = str(now % SLICES)
    why = f"main has {now} squash-merges, so slice {chosen} of {SLICES}"
    if now - then > 1:
        why += (
            f"; the push adds {now - then}, and the slices of the others wait"
            " for their next turn"
        )
    return [chosen], why


def named(slices):
    """`slices` in words, such as "slice 3" or "slices 0 to 9"."""
    if len(slices) == 1:
        return f"slice {slices[0]}"
    if slices == LABELS:
        return f"slices 0 to {SLICES - 1}"
    return "slices " + ", ".join(slices)


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


def gate_commit(event, before, after):
    """The commit whose own quality.yml run a slice waits for, or "" when the push runs
    none: a manual run, or a push that changes only paths quality.yml ignores."""
    if event == "workflow_dispatch":
        return ""
    base = before if squash_merges(before) is not None else f"{after}^"
    changed = git("diff", "--name-only", base, after)
    if changed is None:
        return after
    return after if any(gated(path) for path in changed.split()) else ""


def busy(repo, api, run_id, gate):
    """(why the gates are busy, or "" when they are idle, other unfinished full runs).

    One call lists the full runs: an older one that has not finished holds the machines,
    and only older ones count, so two never wait on each other. Only then, when none does,
    a second lists the quality.yml runs: one that has not finished holds them, as does the
    absence of `gate`'s own run, which may not have been queued yet."""
    runs = api("GET", FULL_RUNS.format(repo=repo))["workflow_runs"]
    others = [run for run in runs if run["status"] in UNFINISHED and int(run["id"]) != run_id]
    older = sum(1 for run in others if int(run["id"]) < run_id)
    if older:
        return f"{older} older quality-full.yml runs unfinished", len(others)
    runs = api("GET", GATE_RUNS.format(repo=repo))["workflow_runs"]
    gates = sum(1 for run in runs if run["status"] in UNFINISHED)
    if gates:
        return f"{gates} quality.yml runs unfinished", len(others)
    if gate and not any(run.get("head_sha") == gate for run in runs):
        return f"main's own quality.yml run for {gate} has not appeared yet", len(others)
    return "", len(others)


def wait(repo, api, run_id, gate, interval, limit, sleep=time.sleep, clock=time.monotonic):
    """(whether the gates and older full runs went idle within `limit` seconds, why). An
    API error counts as busy, so a slice never starts while they may be waiting. Each
    other full run that is waiting too stretches the interval, to spare the API."""
    started = clock()
    while True:
        try:
            why, others = busy(repo, api, run_id, gate)
        except (ApiError, KeyError, TypeError, ValueError) as error:
            why, others = f"could not look up unfinished runs: {error}", 0
        waited = int(clock() - started)
        if not why:
            return True, f"no quality.yml or older full run was unfinished after {waited // 60} min"
        pause = interval * (1 + others)
        if waited >= limit:
            return False, f"{why} after {waited // 60} min, checking every {pause // 60} min"
        sleep(pause)


def start(needs):
    """(idle, busy or failed, why) from the wait jobs' `needs` context."""
    waits = [needs.get(f"wait{index}") or {} for index in range(1, 5)]
    ran = [job for job in waits if job.get("result") != "skipped"]
    if any(
        job.get("result") == "success" and (job.get("outputs") or {}).get("idle") == "true"
        for job in ran
    ):
        return "idle", "the gates went idle"
    if ran and all(
        job.get("result") == "success" and (job.get("outputs") or {}).get("idle") == "false"
        for job in ran
    ):
        return "busy", f"the gates stayed busy through {len(ran)} waits"
    results = ", ".join(f"wait{index + 1} {job.get('result')}" for index, job in enumerate(waits))
    return "failed", f"a wait job did not finish: {results}"


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
    """(status, findings) from a platform's log and status files. The status is an exit
    status, SETUP_FAILED, or STARTED when the pass never recorded its end."""
    log = folder / "gate.log"
    lines = (
        log.read_text(encoding="utf-8", errors="replace").splitlines() if log.is_file() else []
    )
    findings = crap_findings(lines) + [line.rstrip() for line in lines if MUTANT.match(line)]
    status_file = folder / "status"
    status = status_file.read_text(encoding="utf-8").strip() if status_file.is_file() else ""
    return status or STARTED, findings


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


def blocks(pattern, text):
    """Each block `pattern` finds in `text`, by its key, with its content."""
    return {match.group(1): match.group(2) for match in pattern.finditer(text or "")}


def kept(old, note, fallback):
    """`note` first, then an earlier block's content without its earlier note, or
    `fallback` when there is none. The note always comes first, so the next replaces it."""
    return f"{note}\n\n{NOTE.sub('', old) if old is not None else fallback}"


def platform_block(key, text):
    return f"<!-- platform {key} -->\n{text}<!-- end platform {key} -->\n"


def enabled(key, run):
    """Whether CLIMON2_OWN_RUNNERS lists platform `key`, as the workflow's contains() does."""
    return key in run.own.lower()


def platform_text(key, name, label, run):
    """(a platform's new content or None to keep its old one with a note, the note,
    whether it needs the owner's attention)."""
    folder = run.logs / f"full-run-{key}-{label}"
    if not folder.is_dir():
        if not enabled(key, run):
            return None, f"> Did not run on {run.date}: it is not in CLIMON2_OWN_RUNNERS.", False
        return None, f"> Did not run on {run.date}: its job left no log ({run.url}).", True
    status, findings = platform_result(folder)
    if status == SETUP_FAILED:
        outcome = "its set-up failed"
    elif status == STARTED:
        outcome = "no exit status: the slice timed out or was cancelled"
    else:
        outcome = f"exit {status}"
    room = SECTION_LIMIT // len(PLATFORMS) - 200
    if findings:
        head = f"### {name}: {len(findings)} findings, {outcome}\n\n"
        return head + listed(findings, run.url, room), None, True
    if status != "0":
        return f"### {name}: no findings, {outcome}\n\nSee the run's log.\n", None, True
    return f"### {name}: no findings\n", None, False


def section(old, label, gates, run):
    """(slice `label`'s new section content, whether it needs the owner's attention).
    `old` is its earlier content, or None."""
    if gates != "idle":
        why = "gates were busy" if gates == "busy" else "a job that waits for the gates failed"
        note = f"> Skipped on {run.date}: {why} ({run.url})."
        return kept(old, note, f"## Slice {label} of {SLICES}\n"), gates == "failed"
    parts = [f"## Slice {label} of {SLICES}, at {run.sha}\n\n{run.url}\n"]
    earlier = blocks(PLATFORM, old)
    attention = False
    for key, name in PLATFORMS:
        text, note, needs = platform_text(key, name, label, run)
        attention = attention or needs
        if text is None:
            text = kept(earlier.get(key), note, f"### {name}\n")
        parts.append(platform_block(key, text))
    return "\n".join(parts), attention


def merged(body, new):
    """The issue body with each slice's content in `new` replacing its section, slices in
    order. A body edited in the browser comes back with CRLF line ends, which would hide
    its sections from the pattern and drop them."""
    sections = blocks(SECTION, (body or "").replace("\r\n", "\n"))
    sections.update(new)
    return HEADER + "\n".join(
        f"<!-- slice {key} -->\n{sections[key]}<!-- end slice {key} -->\n"
        for key in LABELS
        if key in sections
    )


def tracking_issue(repo, api):
    """The open tracking issue, from every page of the bot's open issues, or None."""
    for page in range(1, ISSUE_PAGES + 1):
        issues = api("GET", ISSUES.format(repo=repo, page=page))
        for issue in issues:
            if (
                issue.get("title") == TITLE
                and (issue.get("user") or {}).get("login") == BOT
                and "pull_request" not in issue
            ):
                return issue
        if len(issues) < ISSUES_PER_PAGE:
            return None
    return None


def report(repo, api, slices, gates, run):
    """What the report did with the tracking issue."""
    try:
        issue = tracking_issue(repo, api)
        failure = None
    except (ApiError, KeyError, TypeError, AttributeError) as error:
        issue = None
        failure = error
    earlier = blocks(SECTION, ((issue or {}).get("body") or "").replace("\r\n", "\n"))
    new = {}
    noted = []
    for label in slices:
        new[label], attention = section(earlier.get(label), label, gates, run)
        if attention:
            noted.append(label)
    name = named(slices)
    if gates == "busy":
        lead = f"{name} skipped: gates were busy"
    elif gates == "failed":
        lead = f"{name} skipped: a job that waits for the gates failed"
    elif not any(enabled(key, run) for key, _ in PLATFORMS):
        lead = f"no pass of {name} ran: CLIMON2_OWN_RUNNERS lists no machine"
    elif noted:
        lead = f"findings in {named(noted)}"
    else:
        lead = f"no findings in {name}"
    try:
        if failure:
            raise ApiError(failure)
        if issue:
            # A clean pass clears the slice's old findings, so the issue keeps only open ones.
            body = merged(issue.get("body"), new)
            api("PATCH", f"repos/{repo}/issues/{issue['number']}", {"body": body})
            if noted:
                api(
                    "POST",
                    f"repos/{repo}/issues/{issue['number']}/comments",
                    {"body": f"{lead[0].upper()}{lead[1:]}: {run.url}"},
                )
            return f"{lead}; {issue['html_url']} is updated"
        if noted:
            body = merged("", new)
            created = api("POST", f"repos/{repo}/issues", {"title": TITLE, "body": body})
            return f"{lead}; {created['html_url']} is opened"
        return lead
    except (ApiError, KeyError, TypeError, AttributeError) as error:
        return f"{lead}, but could not update the tracking issue: {error}"


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
    start_command = commands.add_parser("start")
    start_command.add_argument("--needs", required=True, type=json.loads)
    for name in ("wait", "report"):
        command = commands.add_parser(name)
        command.add_argument("--repo", required=True)
        command.add_argument("--fixtures", help="answer API calls from this JSON file (tests)")
        command.add_argument("--record", help="append API writes to this file (tests)")
    wait_command = commands.choices["wait"]
    wait_command.add_argument("--run-id", required=True, type=int)
    wait_command.add_argument("--gate", default="", help="wait for this commit's quality.yml run")
    wait_command.add_argument("--interval", type=int, default=300)
    wait_command.add_argument("--limit", type=int, default=5 * 60 * 60)
    report_command = commands.choices["report"]
    report_command.add_argument("--slices", required=True, type=json.loads)
    report_command.add_argument("--gates", required=True, choices=("idle", "busy", "failed"))
    report_command.add_argument("--own-runners", default="")
    report_command.add_argument("--date", required=True)
    report_command.add_argument("--logs", required=True, type=Path)
    report_command.add_argument("--run-url", required=True)
    report_command.add_argument("--sha", required=True)
    args = parser.parse_args()

    if args.command == "slice":
        chosen, why = choose(args.event, args.requested, args.before, args.after)
        gate = gate_commit(args.event, args.before, args.after) if chosen else ""
        append("GITHUB_OUTPUT", f"slices={json.dumps(chosen) if chosen else ''}\ngate={gate}\n")
        if chosen:
            message = f"Full mutation pass mutates {named(chosen)}: {why}."
        else:
            message = f"Full mutation pass skipped: {why}."
    elif args.command == "start":
        state, why = start(args.needs)
        append("GITHUB_OUTPUT", f"state={state}\n")
        message = f"Gates {state}: {why}."
    else:
        api = fixture_api(args.fixtures, args.record) if args.fixtures else gh_api
        if args.command == "wait":
            idle, why = wait(args.repo, api, args.run_id, args.gate, args.interval, args.limit)
            append("GITHUB_OUTPUT", f"idle={'true' if idle else 'false'}\n")
            message = f"{'Gates idle' if idle else 'Gates still busy'}: {why}."
        else:
            slices = [label for label in args.slices if label in LABELS]
            if not slices:
                parser.error(f"--slices names no slice: {args.slices!r}")
            run = Run(args.logs, args.run_url, args.own_runners, args.date, args.sha)
            message = report(args.repo, api, slices, args.gates, run)
            message = message[0].upper() + message[1:] + "."
    print(message)
    append("GITHUB_STEP_SUMMARY", f"- {message}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
