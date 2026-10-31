"""THROWAWAY #43: bounded hand-selected mutants of probe/src/restore.rs.

mutator/crapper/dryer are not installed here. Each mutant is compiled and
tested with one Cargo job and one test thread; the source is always restored.
"""

import subprocess
import sys
from pathlib import Path

SOURCE = Path("probe/src/restore.rs")
MUTANTS = [
    ("alternate grid not reinstated", "*term.grid_mut() = alternate;", "let _ = alternate;"),
    ("no swap: active grid serialised as normal",
     "        term.swap_alt();\n        writer.history_and_screen(term.grid());\n        writer.cursor(&term.grid().cursor);\n        term.swap_alt();",
     "        writer.history_and_screen(term.grid());\n        writer.cursor(&term.grid().cursor);"),
    ("normal cursor not placed before 1049h", "        writer.cursor(&term.grid().cursor);\n        term.swap_alt();", "        term.swap_alt();"),
    ("saved cursor not stored", 'self.out.extend(b"\\x1b7");', ""),
    ("soft wrap ignored", "continued = self.row(grid, line);", "self.row(grid, line);\n            continued = false;"),
    ("trailing blanks not trimmed", "true => grid.columns(),", "_ if true => grid.columns(),"),
    ("modes not restored", "    writer.modes(*term.mode());\n", ""),
    ("links never emitted", "if link != self.link {", "if false && link != self.link {"),
    ("link id omitted", '"\\x1b]8;id={};{}\\x1b\\\\", link.id(), link.uri()', '"\\x1b]8;;{}\\x1b\\\\", link.uri()'),
    ("SGR flags inverted", "if style.flags.contains(flag) {", "if !style.flags.contains(flag) {"),
    ("bright colours shifted", "base + 52 + named as u8", "base + 60 + named as u8"),
    ("indexed colour dropped", 'Color::Indexed(index) => write!(self.out, ";{};5;{index}", base + 8),', "Color::Indexed(_) => Ok(()),"),
    ("rgb colour dropped", 'Color::Spec(rgb) => write!(self.out, ";{};2;{};{};{}", base + 8, rgb.r, rgb.g, rgb.b),', "Color::Spec(_) => Ok(()),"),
    ("cursor row off by one", "point.line.0 + 1,", "point.line.0,"),
    ("wide spacers emitted", ".intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)", ".intersects(Flags::empty())"),
    ("zero-width marks dropped", "for c in cell.zerowidth().unwrap_or_default() {", "for c in cell.zerowidth().unwrap_or_default().iter().take(0) {"),
    ("pen not reset before line break", "                self.pen(Style::default(), None);\n                self.out.extend(b\"\\r\\n\");", "                self.out.extend(b\"\\r\\n\");"),
    ("alternate rows not erased", '"\\x1b[{}H\\x1b[2K"', '"\\x1b[{}H"'),
]


def run():
    command = ["cargo", "test", "-j1", "--quiet", "--offline", "--locked",
               "--manifest-path", "probe/Cargo.toml", "--test", "restore", "--test", "osc8",
               "--", "--test-threads=1"]
    try:
        return subprocess.run(command, capture_output=True, timeout=600).returncode
    except subprocess.TimeoutExpired:
        return "timeout"


def main():
    original = SOURCE.read_text()
    survivors = 0
    try:
        assert run() == 0, "baseline must pass"
        for name, old, new in MUTANTS:
            assert original.count(old) == 1, f"mutant site not unique: {name}"
            SOURCE.write_text(original.replace(old, new))
            result = run()
            verdict = "KILLED" if result not in (0, "timeout") else ("TIMEOUT" if result == "timeout" else "SURVIVED")
            survivors += verdict == "SURVIVED"
            print(f"{verdict:9} {name} (exit {result})", flush=True)
    finally:
        SOURCE.write_text(original)
    print(f"{len(MUTANTS)} mutants, {survivors} survivors")
    return 1 if survivors else 0


if __name__ == "__main__":
    sys.exit(main())
