"""Throwaway #52 bounded native row-capacity oracle."""

import argparse
import json
import re
import time
import uuid

from terminal_app import script, validate_target, write_tty


def planned_rows(limit, screen_rows, budget):
    if limit < 0 or screen_rows < 2 or not 1 <= budget <= 20000:
        raise ValueError("invalid native row limit, screen geometry or output budget")
    emitted = limit + screen_rows + 16
    if emitted + 3 > budget:
        raise ValueError("configured limit does not fit the consented output budget")
    return emitted


def capacity_indices(text):
    return [
        int(line.removeprefix("CAP-52-")) for line in text.splitlines()
        if re.fullmatch(r"CAP-52-[0-9]{5}", line)
    ]


def analyse(history, emitted, limit, screen_rows):
    indices = capacity_indices(history)
    if not indices or 0 in indices:
        raise RuntimeError("oldest synthetic row was not demonstrably evicted")
    if indices != list(range(indices[0], emitted)):
        raise RuntimeError("retained native rows are not a contiguous newest tail")
    if not max(1, limit - 2) <= len(indices) <= limit + screen_rows:
        raise RuntimeError("retained native tail is inconsistent with the configured row cap")
    return {
        "oldest_evicted": True,
        "first_retained": indices[0],
        "last_retained": indices[-1],
        "retained_rows": len(indices),
        "configured_limit": limit,
        "screen_rows": screen_rows,
        "boundary": "native history API includes viewport; final checkpoint and newline use two rows",
    }


def wait_for_marker(window, tty, marker):
    deadline = time.monotonic() + 5
    while True:
        history = script(window, tty, "history of testTab")
        if marker in history:
            return history
        if time.monotonic() >= deadline:
            raise RuntimeError("fresh capacity checkpoint was not observed")
        time.sleep(0.1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--permission-confirmed", action="store_true", required=True)
    parser.add_argument("--window-id", type=int, required=True)
    parser.add_argument("--tty", required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--limit", type=int, required=True)
    parser.add_argument("--output-budget", type=int, default=20000)
    args = parser.parse_args()
    validate_target(args.window_id, args.tty)
    emitted = planned_rows(args.limit, 6, args.output_budget)
    geometry = script(
        args.window_id, args.tty,
        '(number of columns of testTab as text) & "," & (number of rows of testTab as text)',
    )
    if geometry != "24,6":
        raise RuntimeError("capacity fixture requires the authorized 24x6 tab")

    def check_profile():
        if script(args.window_id, args.tty, "name of current settings of testTab") != args.profile:
            raise RuntimeError("dedicated tab does not use the verified temporary profile")

    check_profile()
    attempt = uuid.uuid4().hex[:16]
    prior_marker = f"PRE-52-{attempt}"
    end_marker = f"END-52-{attempt}"
    before = script(args.window_id, args.tty, "history of testTab").count(prior_marker)
    if before:
        raise RuntimeError("attempt marker already exists; refusing ambiguous replay")
    write_tty(args.window_id, args.tty, f"\r\n{prior_marker}\r\n".encode())
    wait_for_marker(args.window_id, args.tty, prior_marker)
    for start in range(0, emitted, 512):
        check_profile()
        payload = "".join(
            f"CAP-52-{index:05}\r\n" for index in range(start, min(start + 512, emitted))
        ).encode()
        write_tty(args.window_id, args.tty, payload)
    check_profile()
    write_tty(args.window_id, args.tty, f"{end_marker}\r\n".encode())
    history = wait_for_marker(args.window_id, args.tty, end_marker)
    result = analyse(history, emitted, args.limit, 6)
    if prior_marker in history:
        raise RuntimeError("fresh pre-append sentinel survived beyond the configured row cap")
    print(json.dumps({
        "profile": args.profile, "geometry": geometry, "emitted_capacity_rows": emitted,
        "emitted_newline_operations": emitted + 3, "output_budget": args.output_budget,
        "fresh_prior_content_evicted": True, "fresh_end_checkpoint": True, "result": result,
        "emitted_controls": "CR/LF only; no clear, reset, query or OSC",
    }, indent=2))


if __name__ == "__main__":
    main()
