"""Check specification declarations, never production runtime conformance."""

import os
import pathlib
import re
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[3]


def write_examples(directory):
    """The layout check, the examples, and each C snippet of the pages as a
    function body of its own, in `directory`."""
    sources = [HERE / "layout.c", HERE / "examples.c"]
    for page in sorted(HERE.glob("*.md")):
        for index, snippet in enumerate(
            re.findall(r"```c\n(.*?)```", page.read_text(encoding="utf-8"), re.S)
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
    return sources


def msvc_environment():
    """The environment of the newest Visual C++ x64 toolset, from vcvars64."""
    vswhere = (
        pathlib.Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)"))
        / "Microsoft Visual Studio" / "Installer" / "vswhere.exe"
    )
    installation = subprocess.run(
        [
            str(vswhere), "-latest", "-products", "*", "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property", "installationPath",
        ],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    vcvars = pathlib.Path(installation) / "VC" / "Auxiliary" / "Build" / "vcvars64.bat"
    listing = subprocess.run(
        f'"{vcvars}" >nul && set', shell=True,
        capture_output=True, text=True, check=True,
    ).stdout
    return dict(line.split("=", 1) for line in listing.splitlines() if "=" in line)


class NativeDeclarations(unittest.TestCase):
    @unittest.skipUnless(sys.platform == "win32", "MSVC is checked on Windows")
    def test_msvc_declarations_and_examples(self):
        """Native revision 1 under the Microsoft x64 convention (#327): the
        layout check, the examples and the snippets compile with MSVC."""
        environment = msvc_environment()
        # C4210 calls a block-scope function declaration, which is standard
        # C11 and which request-create.md's snippet uses, an extension.
        with tempfile.TemporaryDirectory(prefix="climon2-native-docs-") as directory:
            for source in write_examples(directory):
                result = subprocess.run(
                    [
                        "cl", "/nologo", "/std:c11", "/W4", "/WX", "/wd4210", "/Zs",
                        "/I", str(HERE), str(source),
                    ],
                    capture_output=True, text=True, check=False,
                    env=environment, shell=True,
                )
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_c_declarations_and_examples(self):
        with tempfile.TemporaryDirectory(prefix="climon2-native-docs-") as directory:
            sources = write_examples(directory)
            checks = [(None, sources)]
            for target, system in (
                ("arm64-apple-macos11", "darwin"),
                ("x86_64-apple-macos11", "darwin"),
                ("x86_64-linux-gnu", "linux"),
            ):
                # Layout uses only compiler headers; examples need the target's libc headers.
                checks.append((target, sources if sys.platform == system else sources[:1]))
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
        index = (HERE / "README.md").read_text()
        for page in HERE.glob("*.md"):
            if page.name == "README.md":
                continue
            self.assertIn(f"]({page.name})", index, page.name)
            text = page.read_text()
            self.assertIn("README.md", text, page.name)
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
