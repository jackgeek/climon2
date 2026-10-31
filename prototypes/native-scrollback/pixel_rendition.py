"""Throwaway #52 window-only raster oracle; no text/geometry fallback."""

import math
import statistics
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import time

from rendition import BLOCK, entry, final_state, load_fixture, resize, verify_layout, visible_observation
from terminal_app import script
from terminal_gui import TITLE_MARKER


def owned_window(candidates, title, permitted):
    if not permitted:
        raise RuntimeError("Screen Recording permission unavailable")
    if TITLE_MARKER not in title:
        raise RuntimeError("owned marker missing")
    matches = [
        item for item in candidates
        if item["owner"] == "Terminal" and TITLE_MARKER in item["title"]
    ]
    if len(matches) != 1 or matches[0]["title"] != title or matches[0]["id"] <= 0:
        raise RuntimeError("missing, wrong or ambiguous owned CGWindow")
    return matches[0]


def contained(box, outer):
    x, y, width, height = box
    ox, oy, ow, oh = outer
    return x >= ox and y >= oy and x + width <= ox + ow and y + height <= oy + oh


def pixel_box(box, viewport, window, image):
    if any(
        len(rect) != 4 or not all(math.isfinite(n) for n in rect)
        or rect[2] <= 0 or rect[3] <= 0
        for rect in [box, viewport, window]
    ):
        raise RuntimeError("invalid point rectangle")
    if len(image) != 2 or any(not isinstance(n, int) or n <= 0 for n in image):
        raise RuntimeError("screenshot unavailable or invalid dimensions")
    if not contained(box, viewport) or not contained(box, window):
        raise RuntimeError("offscreen or partially clipped native box")
    sx, sy = image[0] / window[2], image[1] / window[3]
    if not math.isclose(sx, sy, rel_tol=0.001) or not 0.5 <= sx <= 4:
        raise RuntimeError("nonuniform or implausible captured backing scale")
    edges = [
        (box[0] - window[0]) * sx, (box[1] - window[1]) * sy,
        (box[0] + box[2] - window[0]) * sx,
        (box[1] + box[3] - window[1]) * sy,
    ]
    left, top = math.ceil(edges[0] - 1e-6), math.ceil(edges[1] - 1e-6)
    right, bottom = math.floor(edges[2] + 1e-6), math.floor(edges[3] + 1e-6)
    if right <= left or bottom <= top:
        raise RuntimeError("native box has no wholly contained pixels")
    return [left, top, right - left, bottom - top]


def sample_cell(pixels, dimensions, box, background, text, expected):
    width, height = dimensions
    if width <= 0 or height <= 0 or len(pixels) != width * height * 3:
        raise RuntimeError("screenshot unavailable or malformed RGB data")
    if not contained(box, [0, 0, width, height]) or min(box[2:]) <= 0:
        raise RuntimeError("invalid pixel sample box")
    x, y, w, h = box
    ink = []
    colors = []
    for row in range(y, y + h):
        for column in range(x, x + w):
            offset = (row * width + column) * 3
            color = tuple(pixels[offset:offset + 3])
            colors.append(color)
            if max(abs(c - b) for c, b in zip(color, background)) > 24:
                ink.append((column - x, color))
    if expected == "blank":
        if ink:
            raise RuntimeError("background control is not blank")
        return {
            "ink_pixels": 0, "pixel_box": box,
            "dominant_rgb": [statistics.median(color[i] for color in colors) for i in range(3)],
            "max_background_distance": max(max(abs(c - b) for c, b in zip(color, background)) for color in colors),
        }
    if expected not in {"blue", "text"}:
        raise RuntimeError("unknown expected colour")
    strong = [
        (column, color) for column, color in ink
        if max(abs(c - b) for c, b in zip(color, background)) >= 96
    ]
    minimum = max(3, math.ceil(w * h * 0.01))
    if len(strong) < minimum:
        raise RuntimeError("glyph ink missing or antialiasing ambiguous")
    def matches(color):
        if expected == "blue":
            return color[2] - max(color[0], color[1]) >= 40
        direction = [t - b for t, b in zip(text, background)]
        magnitude = sum(d * d for d in direction)
        if magnitude == 0:
            raise RuntimeError("foreground/background controls have no contrast")
        coverage = sum((c - b) * d for c, b, d in zip(color, background, direction)) / magnitude
        mixture = [b + coverage * d for b, d in zip(background, direction)]
        return 0 <= coverage <= 1 and max(abs(c - m) for c, m in zip(color, mixture)) <= 16
    matched = sum(matches(color) for _, color in strong)
    if matched / len(strong) < 0.9:
        dominant = [statistics.median(color[i] for _, color in strong) for i in range(3)]
        raise RuntimeError(
            f"dominant ink colour disagrees with independent expectation: expected={expected}, "
            f"box={box}, strong={len(strong)}, matched={matched}, median_sRGB={dominant}"
        )
    return {
        "pixel_box": box, "ink_pixels": len(ink), "strong_ink_pixels": len(strong),
        "colour_match_fraction": matched / len(strong),
        "dominant_rgb": [statistics.median(color[i] for _, color in strong) for i in range(3)],
        "ink_columns": sorted({column for column, _ in strong}),
        "expected": expected,
    }


DIRECTORY = Path(__file__).parent


def bridge(*args):
    result = subprocess.run(
        [str(DIRECTORY / "artifacts" / "capture-pixels"), *args],
        text=True, capture_output=True, timeout=15,
    )
    if result.returncode:
        raise RuntimeError(result.stderr.strip())
    return json.loads(result.stdout)


def capture(title, observation, columns, palette):
    metadata = bridge("--metadata", title)
    owned_window([metadata], title, metadata["permitted"])
    with tempfile.TemporaryDirectory(prefix="climon2-52-pixels-") as directory:
        image = Path(directory) / "owned.png"
        raw = Path(directory) / "owned.rgb"
        subprocess.run(
            ["screencapture", "-x", "-o", "-l", str(metadata["id"]), str(image)],
            capture_output=True, check=True, timeout=15,
        )
        if not image.is_file() or image.stat().st_size == 0:
            raise RuntimeError("owned screenshot unavailable")
        decoded = bridge("--decode", str(image), str(raw))
        if bridge("--metadata", title) != metadata:
            raise RuntimeError("owned window moved or changed during capture")
        pixels = raw.read_bytes()
        dimensions = decoded["dimensions"]
        result = qualify(pixels, dimensions, metadata["bounds"], observation, columns, palette)
    if image.exists() or raw.exists() or Path(directory).exists():
        raise RuntimeError("transient image cleanup incomplete")
    result["transient_images_deleted"] = True
    result["capture"] = {
        "window_bounds_points": metadata["bounds"], **decoded,
        "backing_scale": dimensions[0] / metadata["bounds"][2],
        "window_only": True, "shadow_excluded": True,
    }
    return result


def qualify(pixels, dimensions, window, observation, columns, palette):
    layout = verify_layout(observation, columns)
    if observation.get("selection_cleared") is not True:
        raise RuntimeError("selection highlighting not cleared")
    viewport = observation["viewport_rect"]
    def points(index):
        return observation["unit_bounds"][index]["bounds"]["rect"]
    def sample(box, expected):
        return sample_cell(
            pixels, dimensions, pixel_box(box, viewport, window, dimensions),
            palette["background"], palette["text"], expected,
        )
    wide = points(36)
    cell = layout["ascii_cell_points"]
    combining = points(37)
    blanks = [
        [wide[0] + 3 * cell, wide[1], cell, wide[3]],
        [wide[0] + 4 * cell, wide[1], cell, wide[3]],
    ]
    controls = [sample(box, "blank") for box in blanks]
    ascii_samples = [
        {"utf16_index": i, **sample(points(i), "text")}
        for i in [*range(6), *range(7, 35), *range(40, 46)]
    ]
    blue = sample(wide, "blue")
    halves = [
        sample([wide[0] + n * cell, wide[1], cell, wide[3]], "blue")
        for n in [0, 1]
    ]
    combined = sample(combining, "text")
    return {
        "columns": columns, "layout": layout, "native_observation": observation,
        "blank_controls": controls,
        "ascii": ascii_samples, "wide": blue, "wide_halves": halves,
        "combining": combined, "wide_ink_in_both_cells": True,
        "combining_ink_within_shared_box_and_adjacent_cell_blank": True,
        "relative_blue_and_glyph_presence_verified": True,
        "exact_ansi_palette_fidelity_qualified": False,
        "glyph_outline_or_combining_accent_shape_qualified": False,
    }


def native_checkpoint(window, tty, columns, palette):
    visible_observation(window, tty)
    title = script(window, tty, f"name of window id {window}")
    result = subprocess.run(
        [str(DIRECTORY / "artifacts" / "select-range"), "--observe-pixels", title, "block", BLOCK],
        text=True, capture_output=True, timeout=10,
    )
    if result.returncode:
        raise RuntimeError(f"native pixel range probe failed: {result.stderr.strip()}")
    observation = json.loads(result.stdout)
    verify_layout(observation, columns)
    time.sleep(0.2)
    return capture(title, observation, columns, palette)


def native_palette(window, tty):
    result = {
        "source": "live Terminal Basic AppleScript normal text/background colours, 16-bit RGB",
        "blue_expectation": "relative: strong blue channel exceeds red and green by >=40 in sRGB8; not exact ANSI palette",
        "thresholds": {
            "background_distance": 24, "strong_ink_distance": 96,
            "minimum_strong_fraction": 0.01, "minimum_strong_pixels": 3,
            "text_antialias_colour_segment_residual": 16, "blue_channel_margin": 40, "required_match_fraction": 0.9,
        },
    }
    for key, property_name in [("text", "normal text color"), ("background", "background color")]:
        components = script(window, tty, f"{property_name} of current settings of testTab").split(", ")
        if len(components) != 3:
            raise RuntimeError("live palette unavailable")
        values = [int(n) for n in components]
        if any(not 0 <= n <= 65535 for n in values):
            raise RuntimeError("invalid live palette")
        result[key] = [n * 255 / 65535 for n in values]
    if result["text"] != [0, 0, 0] or result["background"] != [255, 255, 255]:
        raise RuntimeError("relative observer only qualified for black-on-white Basic controls")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--permission-confirmed", action="store_true", required=True)
    parser.add_argument("--window-id", type=int, required=True)
    parser.add_argument("--tty", required=True)
    parser.add_argument("--report-file", type=Path, required=True)
    args = parser.parse_args()
    if args.report_file.exists():
        parser.error("refusing to overwrite report")
    entry(args.window_id, args.tty)
    palette = native_palette(args.window_id, args.tty)
    title = script(args.window_id, args.tty, f"name of window id {args.window_id}")
    metadata = bridge("--metadata", title)
    owned_window([metadata], title, metadata["permitted"])
    result = {"palette": palette, "checkpoints": []}
    try:
        load_fixture(args.window_id, args.tty)
        for columns in [24, 32, 24]:
            resize(args.window_id, args.tty, columns)
            result["checkpoints"].append(native_checkpoint(args.window_id, args.tty, columns, palette))
    except (RuntimeError, subprocess.SubprocessError, OSError) as error:
        result["status"] = "BLOCKED"
        result["blocker"] = str(error)
    else:
        result["status"] = "PASS_RELATIVE_COLOUR_AND_GLYPH_PRESENCE"
    finally:
        resize(args.window_id, args.tty, 24)
        result["cleanup"] = final_state(args.window_id, args.tty)
    result["host"] = script(
        args.window_id, args.tty,
        'version & "|" & name of current settings of testTab & "|" & font name of current settings of testTab & "|" & (font size of current settings of testTab as text)',
    )
    with args.report_file.open("x") as output:
        json.dump(result, output, indent=2, ensure_ascii=True)
        output.write("\n")
    print(json.dumps({"status": result["status"], "blocker": result.get("blocker"), "cleanup": result["cleanup"]}))
    if result["status"] == "BLOCKED":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
