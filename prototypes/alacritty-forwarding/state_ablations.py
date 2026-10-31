#!/usr/bin/env python3
"""THROWAWAY #43: bounded, exact source ablations; always restore own worktree."""
import os
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parent
source = root / "probe/src/restore.rs"
original = source.read_text()
env = dict(os.environ, CARGO_TARGET_DIR=str(root / ".cache/target"))
binary = root / ".cache/target/debug/examples/derive_state"
manifest = str(root / "probe/Cargo.toml")
build = ["cargo", "build", "-j1", "--quiet", "--locked", "--manifest-path", manifest,
         "--example", "derive_state"]
rust = ["cargo", "test", "-j1", "--quiet", "--locked", "--manifest-path", manifest,
        "--test", "osc8", "--", "--test-threads=1"]
ablations = [
    ("raw URI", "if ('\\u{7f}'..='\\u{9f}').contains(&c)", "if false", "encoding"),
    ("raw id", "self.token_part(link.id());", "self.out.extend(link.id().as_bytes());", "encoding"),
    ("merged ids", "self.token_part(link.id());", 'self.token_part("all");', "encoding"),
    ("URI alias merges", "self.token_part(link.uri());", 'self.token_part("all");', "encoding"),
    ("lost pending wrap", "if cursor.input_needs_wrap {", "if false {", "state"),
    ("lost designation", "StandardCharset::SpecialCharacterAndLineDrawing => b'0'",
     "StandardCharset::SpecialCharacterAndLineDrawing => b'B'", "state"),
    ("lost underline colour", "underline: cell.underline_color()", "underline: None", "state"),
    ("lost cursor style", "writer.cursor_style(term.cursor_style());", "", "state"),
    ("lost selected charset", "let charset = probe_charset(term);",
     "let charset = CharsetIndex::G0;", "blockers"),
    ("lost scroll region", "let region = probe_region(term);",
     "let region = (0, term.screen_lines());", "blockers"),
    ("lost tabs", "writer.tabs(&tabs);", "", "state"),
    ("tab cell emitted as control", "if cell.c == '\\t'", "if false", "state"),
    ("lost origin", "writer.cursors(term.grid(), origin, region, charset);",
     "writer.cursors(term.grid(), false, region, charset);", "blockers"),
    ("lost normal saved pen", "writer.cursor(&term.grid().cursor, term.grid(), false, region, charset);",
     "writer.cursor(&term.grid().cursor, term.grid(), false, region, charset);\n"
     "        writer.pen(Style::default(), None);", "state"),
]

results = []
try:
    for name, old, new, oracle in ablations:
        count = original.count(old)
        assert count in (1, 2, 3), (name, "unexpected source match count", count)
        mutated = original.replace(old, new)
        assert mutated != original
        # This file is inside this session's isolated worktree, not shared code.
        source.write_text(mutated)
        built = subprocess.run(build, env=env, capture_output=True, text=True, timeout=120)
        if built.returncode:
            raise RuntimeError(f"{name}: invalid mutant, not a kill\n{built.stderr}")
        command = rust if name == "URI alias merges" else [
            "node", str(root / f"host-probe/{oracle}.mjs"), str(binary)]
        result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=30)
        if result.returncode == 0:
            raise RuntimeError(f"SURVIVED: {name}")
        text = result.stdout + result.stderr
        signal = next((line for line in text.splitlines()
                       if "AssertionError" in line or line.startswith("FAIL ") or "FAILED" in line), "")
        if not signal:
            raise RuntimeError(f"{name}: failure was not an assertion\n{text}")
        results.append(f"KILLED {name}: {signal}")
finally:
    source.write_text(original)
    # The persisted binary must match the restored source, even on a failure.
    subprocess.run(build, env=env, check=True, timeout=120)
    (root / "state-ablation-results.txt").write_text(
        "One Cargo job, one test thread. Exact source patterns; no upstream edits.\n"
        + "\n".join(results) + "\n")
print("\n".join(results))
print(f"PASS: {len(results)}/{len(ablations)} targeted ablations killed")
