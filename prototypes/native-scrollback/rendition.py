"""Throwaway #52 synthetic-range AX observations; never a visual passing oracle."""

import argparse
import json
import math
from pathlib import Path
import subprocess
import time

from terminal_app import observe, ready, script, validate_target, write_tty
from terminal_gui import COPY_TARGETS, TITLE_MARKER, gui


BLOCK = "HARD-A\n" + COPY_TARGETS["soft"] + "\n" + COPY_TARGETS["wide"] + "\nHARD-Z"
COUNTERS = ["hard_a_copies", "hard_z_copies", "saved_sentinel_copies"]


def validate_entry(title, tabs, busy, geometry, profile):
    if TITLE_MARKER not in title or tabs != "1" or busy != "false":
        raise RuntimeError("owned marker/single-tab/idle entry state mismatch")
    if geometry != "24,6" or profile != "Basic":
        raise RuntimeError("owned entry requires Basic at actual 24x6")


def summarize(observation, target, actual, requested):
    if actual != f"{requested},6":
        raise RuntimeError(f"requested {requested}x6; actual {actual}")
    if observation["target"] != target or observation["range_verified"] is not True:
        raise RuntimeError("native synthetic target/range mismatch")
    for field in ["bounds", "style"]:
        if observation[field]["status"] not in {"observed", "unsupported"}:
            raise RuntimeError(f"native {field} observation failed: {observation[field]}")
    return {
        **observation,
        "actual_geometry": actual,
        "logical_range_verified": True,
        "physical_geometry_qualified": False,
        "rendition_qualified": False,
        "boundary": "AX range data is not independently qualified glyph/cell or color fidelity",
    }


def verify_layout(observation, columns):
    if observation["range_visible"] is not True:
        raise RuntimeError("known synthetic range is outside the native visible range")
    units = observation["unit_bounds"]
    if len(units) != 46 or [unit["utf16_index"] for unit in units] != list(range(46)):
        raise RuntimeError("wrong native UTF-16 geometry range")
    boxes = {}
    for index in [*range(6), *range(7, 35), 36, 37, 38, *range(40, 46)]:
        bounds = units[index]["bounds"]
        if bounds["status"] != "observed":
            raise RuntimeError("native unit bounds unavailable")
        box = bounds["rect"]
        if len(box) != 4 or not all(isinstance(n, (int, float)) and math.isfinite(n) for n in box):
            raise RuntimeError("invalid native unit rectangle")
        boxes[index] = box
    left, top, cell, height = boxes[0]
    if cell <= 0 or height <= 0:
        raise RuntimeError("nonpositive native ASCII cell geometry")
    soft_rows = (28 + columns - 1) // columns
    expected = {i: [left + i * cell, top, cell, height] for i in range(6)}
    expected.update({
        7 + i: [left + (i % columns) * cell, top + (1 + i // columns) * height, cell, height]
        for i in range(28)
    })
    expected.update({
        40 + i: [left + i * cell, top + (2 + soft_rows) * height, cell, height]
        for i in range(6)
    })
    wide_top = top + (1 + soft_rows) * height
    expected.update({
        36: [left, wide_top, 2 * cell, height],
        37: [left + 2 * cell, wide_top, cell, height],
        38: [left + 2 * cell, wide_top, cell, height],
    })
    for index, box in expected.items():
        if boxes[index] != box:
            raise RuntimeError(f"native fixture geometry mismatch at synthetic UTF-16 index {index}: {boxes[index]} != {box}")
    return {
        "accessibility_layout_verified": True,
        "ascii_cell_points": cell, "row_points": height,
        "soft_physical_rows": soft_rows,
        "hard_lines_distinct": True, "wide_cell_ratio": 2,
        "combining_box_shared": True,
        "boundary": "visible native AX boxes, not a raster/glyph-appearance oracle",
    }


def geometry(window, tty):
    return script(window, tty, '(number of columns of testTab as text) & "," & (number of rows of testTab as text)')


def entry(window, tty):
    validate_target(window, tty)
    validate_entry(
        script(window, tty, f"name of window id {window}"),
        script(window, tty, f"count of tabs of window id {window}"),
        script(window, tty, "busy of testTab"),
        geometry(window, tty), script(window, tty, "name of current settings of testTab"),
    )
    gui(window, tty, 'return value of scroll bar 1 of targetArea')


def load_fixture(window, tty):
    before = observe(window, tty)
    gui(window, tty, 'set value of scroll bar 1 of targetArea to 1.0')
    write_tty(window, tty, (Path(__file__).parent / "artifacts" / "02-normal.ansi").read_bytes())
    deadline = time.monotonic() + 5
    while not ready("normal", observe(window, tty), before):
        if time.monotonic() >= deadline:
            raise RuntimeError("fresh normal fixture delivery timed out")
        time.sleep(0.1)


def resize(window, tty, columns):
    if columns not in {20, 24, 32}:
        raise ValueError("resize outside the authorized widths")
    gui(window, tty, f'''
tell application "Terminal"
    set testTab to tab 1 of window id {window}
    if tty of testTab is not "{tty}" then error "Test tab identity changed"
    set number of columns of testTab to {columns}
end tell
''')
    actual = geometry(window, tty)
    if actual != f"{columns},6":
        raise RuntimeError(f"requested {columns}x6; actual {actual}")


def inspect(window, tty):
    gui(window, tty, 'return value of scroll bar 1 of targetArea')
    title = script(window, tty, f"name of window id {window}")
    helper = Path(__file__).parent / "artifacts" / "select-range"
    result = subprocess.run(
        [str(helper), "--observe", title, "block", BLOCK],
        text=True, capture_output=True, timeout=10,
    )
    if result.returncode:
        raise RuntimeError(f"native range probe failed: {result.stderr.strip()}")
    return json.loads(result.stdout)


def visible_observation(window, tty):
    low, high = 0.0, 1.0
    for attempt in range(20):
        position = 1.0 if attempt == 0 else (low + high) / 2
        gui(window, tty, f'set value of scroll bar 1 of targetArea to {position}')
        result = inspect(window, tty)
        if result["bounds"]["status"] == "unsupported":
            return result
        if result["bounds"]["status"] != "observed":
            raise RuntimeError(f"native range bounds failed: {result['bounds']}")
        _, y, _, height = result["bounds"]["rect"]
        _, viewport_y, _, viewport_height = result["viewport_rect"]
        if y >= viewport_y and y + height <= viewport_y + viewport_height:
            if result["range_visible"] is not True:
                raise RuntimeError("AX bounds and visible range disagree")
            return result
        if y < viewport_y:
            high = position
        else:
            low = position
    raise RuntimeError("bounded native scrolling did not expose the known fixture range")


def checkpoint(window, tty, columns):
    actual = geometry(window, tty)
    if actual != f"{columns},6":
        raise RuntimeError(f"requested {columns}x6; actual {actual}")
    result = summarize(visible_observation(window, tty), "block", geometry(window, tty), columns)
    if result["bounds"]["status"] == "observed":
        result["layout"] = verify_layout(result, columns)
    else:
        result["layout"] = {"status": "unsupported"}
    facts = observe(window, tty)
    result["retained_synthetic_counters"] = {key: facts[key] for key in COUNTERS}
    return result


def final_state(window, tty):
    gui(window, tty, 'set value of scroll bar 1 of targetArea to 1.0')
    state = {
        "geometry": geometry(window, tty),
        "profile": script(window, tty, "name of current settings of testTab"),
        "busy": script(window, tty, "busy of testTab"),
    }
    if state != {"geometry": "24,6", "profile": "Basic", "busy": "false"}:
        raise RuntimeError(f"native cleanup incomplete; actual final state: {state}")
    return state


def run_native(window, tty):
    entry(window, tty)
    checkpoints = []
    restored = False
    try:
        load_fixture(window, tty)
        checkpoints.append(checkpoint(window, tty, 24))
        for columns in [20, 32, 24]:
            resize(window, tty, columns)
            restored = columns == 24
            checkpoints.append(checkpoint(window, tty, columns))
    finally:
        if not restored:
            resize(window, tty, 24)
        cleanup = final_state(window, tty)
    counters = [point.get("retained_synthetic_counters") for point in checkpoints]
    if any(value != counters[0] for value in counters):
        raise RuntimeError("retained synthetic history changed across native resizing")
    return {"checkpoints": checkpoints, "cleanup": cleanup}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--permission-confirmed", action="store_true", required=True)
    parser.add_argument("--window-id", type=int, required=True)
    parser.add_argument("--tty", required=True)
    parser.add_argument("--report-file", type=Path, required=True)
    args = parser.parse_args()
    if args.report_file.exists():
        parser.error("refusing to overwrite an existing report")
    result = run_native(args.window_id, args.tty)
    result["host"] = script(
        args.window_id, args.tty,
        'version & "|" & name of current settings of testTab & "|" & '
        'font name of current settings of testTab & "|" & (font size of current settings of testTab as text)',
    )
    report = json.dumps(result, ensure_ascii=True, indent=2) + "\n"
    with args.report_file.open("x") as output:
        output.write(report)
    print(report, end="")


if __name__ == "__main__":
    main()
