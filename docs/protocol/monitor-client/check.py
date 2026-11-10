"""Check specification declarations, never production runtime conformance."""

import pathlib
import re
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]


class NativeDeclarations(unittest.TestCase):
    def test_c_declarations_and_examples(self):
        sources = [HERE / "layout.c", HERE / "examples.c"]
        with tempfile.TemporaryDirectory(prefix="climon2-native-docs-") as directory:
            for page in sorted(HERE.glob("*.md")):
                for index, snippet in enumerate(
                    re.findall(r"```c\n(.*?)```", page.read_text(), re.S)
                ):
                    source = pathlib.Path(directory) / f"{page.stem}-{index}.c"
                    source.write_text(
                        '#include "client-v1.h"\n#include <string.h>\n'
                        "cm1_call example(const cm1_interface *api,\n"
                        "cm1_communication *communication, uint64_t generation,\n"
                        "uint64_t period, uint64_t restoration_id,\n"
                        "const cm1_record *record) {\n"
                        "(void)api; (void)communication; (void)generation;\n"
                        "(void)period; (void)restoration_id; (void)record;\n"
                        + snippet
                        + "\n}\n"
                    )
                    sources.append(source)
            checks = [(None, sources)]
            for target in ("arm64-apple-macos11", "x86_64-apple-macos11"):
                # Layout uses only compiler headers; examples need the target's libc SDK.
                checks.append((target, sources if sys.platform == "darwin" else sources[:1]))
            for target, target_sources in checks:
                options = ["-target", target, "-ffreestanding"] if target else []
                for source in target_sources:
                    result = subprocess.run(
                        [
                            "clang", *options, "-std=c11",
                            "-Wall", "-Wextra", "-Werror", "-fsyntax-only",
                            "-I", str(HERE), str(source),
                        ],
                        capture_output=True, text=True, check=False,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)

    def test_reference_reachability_and_examples(self):
        overview = (HERE.parent / "README.md").read_text()
        for page in HERE.glob("*.md"):
            if page.name == "core-v1.md":
                continue
            self.assertTrue(f"monitor-client/{page.name}" in overview, page.name)
            text = page.read_text()
            self.assertIn("core-v1.md", text, page.name)
            if "extension" not in page.name:
                self.assertIn("```c\n", text, page.name)
            self.assertNotRegex(
                text, r"(?i)(declarations|schemas?) (pending|unresolved)"
            )

    def test_relative_links(self):
        pages = list((ROOT / "docs").rglob("*.md")) + [ROOT / "README.md"]
        for page in pages:
            text = page.read_text()
            for target in re.findall(r"\[[^\]]*\]\(([^)\s]+)\)", text):
                if re.match(r"[a-z][a-z0-9+.-]*:", target):
                    continue
                path, _, anchor = target.partition("#")
                destination = (page.parent / path).resolve() if path else page
                self.assertTrue(destination.exists(), f"{page}: {target}")
                if anchor and destination.suffix == ".md":
                    headings = re.findall(
                        r"^#{1,6}\s+(.+)$", destination.read_text(), re.M
                    )
                    anchors = [
                        re.sub(
                            r"[^\w -]", "",
                            re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", heading)
                            .lower().replace("`", ""),
                        ).replace(" ", "-")
                        for heading in headings
                    ]
                    self.assertIn(anchor, anchors, f"{page}: {target}")


if __name__ == "__main__":
    unittest.main()
