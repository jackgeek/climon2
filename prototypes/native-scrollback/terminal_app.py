"""Permissioned #52 observer: one explicitly identified Terminal.app test tab."""

import argparse
import json
from pathlib import Path
import subprocess
import time


STAGES = {
    "prime": "01-prime.ansi",
    "normal": "02-normal.ansi",
    "regain": "03-regain-full-resend.ansi",
    "alternate": "04-alternate.ansi",
    "return": "05-return-normal.ansi",
    "application-output": "06-application-output.ansi",
}


def script(window, tty, expression):
    # Only numeric window IDs and a validated /dev/ttysNNN path enter this script.
    source = f"""
tell application "Terminal"
    set testTab to tab 1 of window id {window}
    if tty of testTab is not "{tty}" then error "Test tab identity changed"
    return {expression}
end tell
"""
    return subprocess.run(
        ["osascript", "-"], input=source, text=True, capture_output=True,
        check=True, timeout=10,
    ).stdout.rstrip("\n")


def facts(contents, history):
    """Summarize synthetic sentinels only; never publish surrounding shell text."""
    return {
        "visible_normal": [f"NORMAL-{n}" in contents for n in range(6)],
        "visible_alternate": [f"ALT-{n}" in contents for n in range(6)],
        "saved_sentinel": "SAVED-SENTINEL" in history,
        "saved_sentinel_copies": history.count("SAVED-SENTINEL"),
        "prior_visible": [f"VISIBLE-{n}" in history for n in range(6)],
        "hard_a_copies": history.count("HARD-A"),
        "hard_z_copies": history.count("HARD-Z"),
        "soft_text_present": "SOFT01234567890123456789tail" in history.replace("\n", "").replace("\r", ""),
        "wide_combining_present": "界e\u0301" in history,
        "alternate_in_history": any(f"ALT-{n}" in history for n in range(6)),
        "application_output_present": "APPLICATION-ECHO" in history,
        "application_output_copies": history.count("APPLICATION-ECHO"),
        "soft_text_unbroken_copies": history.count("SOFT01234567890123456789tail"),
        "synthetic_history_fragments": [
            line for line in history.splitlines()
            if line in {"SOFT01234567890123456789tail", "HARD-A", "HARD-Z", "界e\u0301"}
        ],
    }


def ready(stage, observation, before=None):
    if before is not None:
        counters = {
            "prime": "saved_sentinel_copies",
            "normal": "hard_a_copies",
            "regain": "hard_a_copies",
            "alternate": "hard_a_copies",
            "application-output": "application_output_copies",
        }
        counter = counters.get(stage)
        if counter and observation[counter] <= before[counter]:
            return False
    if stage == "prime":
        return observation["saved_sentinel"] and all(observation["prior_visible"])
    if stage == "alternate":
        return all(observation["visible_alternate"])
    if stage == "application-output":
        return observation["application_output_present"]
    return all(observation["visible_normal"]) and observation["hard_a_copies"] > 0


def observe(window, tty):
    # On an AppleScript variable, "contents" dereferences the variable itself.
    contents = script(window, tty, f"contents of tab 1 of window id {window}")
    history = script(window, tty, "history of testTab")
    return facts(contents, history)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--permission-confirmed", action="store_true", required=True)
    parser.add_argument("--window-id", type=int, required=True)
    parser.add_argument("--tty", required=True)
    parser.add_argument("--stage", choices=[*STAGES, "resize"], required=True)
    parser.add_argument("--columns", type=int, choices=[12, 24])
    args = parser.parse_args()
    suffix = args.tty.removeprefix("/dev/ttys")
    if (
        args.window_id <= 0 or not args.tty.startswith("/dev/ttys")
        or not suffix.isascii() or not suffix.isdecimal() or len(suffix) != 3
    ):
        parser.error("require a positive dedicated window ID and /dev/ttysNNN")
    geometry = script(
        args.window_id, args.tty,
        '(number of columns of testTab as text) & "," & (number of rows of testTab as text)',
    )
    if args.stage == "resize":
        if args.columns is None or geometry not in {"24,6", "12,6", "20,6"}:
            parser.error("resize requires --columns 12 or 24 and a 6-row dedicated tab")
    elif geometry != "24,6":
        raise RuntimeError(f"fixture requires 24x6; dedicated tab is {geometry}")
    metadata = script(
        args.window_id, args.tty,
        'version & "|" & name of current settings of testTab & "|" & '
        'font name of current settings of testTab & "|" & '
        '(font size of current settings of testTab as text)',
    )
    before = observe(args.window_id, args.tty)
    if args.stage == "resize":
        source = f"""
tell application "Terminal"
    set testTab to tab 1 of window id {args.window_id}
    if tty of testTab is not "{args.tty}" then error "Test tab identity changed"
    set number of columns of testTab to {args.columns}
    return (number of columns of testTab as text) & "," & (number of rows of testTab as text)
end tell
"""
        geometry = subprocess.run(
            ["osascript", "-"], input=source, text=True, capture_output=True,
            check=True, timeout=10,
        ).stdout.strip()
        if geometry != f"{args.columns},6":
            raise RuntimeError(
                f"native resize did not reach requested size: {geometry}; "
                "the tab remains at that actual size; use --stage resize --columns 24 "
                "only with permission to restore it"
            )
        after = observe(args.window_id, args.tty)
        for field in ["hard_a_copies", "hard_z_copies", "saved_sentinel_copies"]:
            if after[field] != before[field]:
                raise RuntimeError(f"history sentinel changed during resize: {field}")
        print(json.dumps({
            "stage": f"resize-{args.columns}", "metadata": metadata,
            "geometry": geometry, "observation": after,
            "boundary": "native history text only; styles/copy/physical-wrap provenance remain unqualified",
        }, ensure_ascii=True, indent=2))
        return
    payload = (Path(__file__).parent / "artifacts" / STAGES[args.stage]).read_bytes()
    if len(payload) > 65536:
        raise RuntimeError("native fixture file exceeds its per-stage budget")
    # Recheck identity immediately before writing; no front-window targeting.
    script(args.window_id, args.tty, "tty of testTab")
    with open(args.tty, "wb", buffering=0) as terminal:
        remaining = memoryview(payload)
        while remaining:
            written = terminal.write(remaining)
            if written is None or written <= 0:
                raise OSError("dedicated TTY write made no progress")
            remaining = remaining[written:]
    deadline = time.monotonic() + 5
    while True:
        observation = observe(args.window_id, args.tty)
        if ready(args.stage, observation, before):
            break
        if time.monotonic() >= deadline:
            raise RuntimeError(f"stage {args.stage} did not reach its checkpoint: {observation}")
        time.sleep(0.1)
    print(json.dumps({
        "stage": args.stage, "metadata": metadata, "geometry": geometry,
        "observation": observation,
        "boundary": "text/history oracle only; not native copy, styles or input qualification",
    }, ensure_ascii=True, indent=2))


if __name__ == "__main__":
    main()
