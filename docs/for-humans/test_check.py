"""Tests for check.py against small trees built in a temporary directory."""

import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check  # noqa: E402


def readme(audience, rows, extra=""):
    table = "\n".join(f"| [{name}]({name}) | x |" for name in rows)
    return f"# T\n\n{extra}\n\n**Audience:** {audience}.\n\n| Doc | Purpose |\n|---|---|\n{table}\n"


class TreeCheck(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.here = self.root / "docs" / "for-humans"
        self.saved = check.HERE, check.ROOT, check.ROOT_README
        check.HERE, check.ROOT, check.ROOT_README = self.here, self.root, self.root / "README.md"
        self.write("README.md", "[docs](docs/for-humans/README.md)\n")
        self.write("docs/for-humans/README.md", readme("Users and Contributors", ["a.md", "s/README.md"]))
        self.write("docs/for-humans/a.md", "# A\n")
        self.write("docs/for-humans/s/README.md", readme("Contributors", ["p/README.md", "q/README.md", "top.md"]))
        self.write("docs/for-humans/s/top.md", "# Top\n")
        self.write("docs/for-humans/s/p/README.md", readme("Contributors", ["x.md"]))
        self.write("docs/for-humans/s/p/x.md", "# X\n")
        self.write("docs/for-humans/s/q/README.md", readme("Contributors", ["y.md"]))
        self.write("docs/for-humans/s/q/y.md", "# Y\n")

    def tearDown(self):
        check.HERE, check.ROOT, check.ROOT_README = self.saved
        self.tmp.cleanup()

    def write(self, rel, text):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(textwrap.dedent(text))

    def append(self, rel, text):
        path = self.root / rel
        path.write_text(path.read_text() + text)

    def errors(self):
        return check.check()

    def test_well_formed_tree_passes(self):
        self.assertEqual(self.errors(), [])

    def test_orphan_fails(self):
        self.write("docs/for-humans/s/README.md", readme("Contributors", ["q/README.md", "top.md"]))
        errors = self.errors()
        self.assertIn("docs/for-humans/s/p/x.md: orphan, not reached from /README.md", errors)

    def test_folder_without_readme_fails(self):
        self.write("docs/for-humans/s/r/z.md", "# Z\n")
        self.assertTrue(any("has no README.md" in e for e in self.errors()))

    def test_table_missing_a_file_fails(self):
        self.write("docs/for-humans/s/p/extra.c", "int x;\n")
        self.assertTrue(any("table lacks extra.c" in e for e in self.errors()))

    def test_audience_its_parent_lacks_fails(self):
        self.write("docs/for-humans/s/p/README.md", readme("Users", ["x.md"]))
        self.assertTrue(any("not in parent" in e for e in self.errors()))

    def test_links_straight_up_and_down_pass(self):
        self.append("docs/for-humans/s/p/x.md", "[up](../top.md) [root](../../a.md)\n")
        self.append("docs/for-humans/s/top.md", "[down](p/x.md)\n")
        self.assertEqual(self.errors(), [])

    def test_sideways_link_to_a_cousin_fails(self):
        self.append("docs/for-humans/s/p/x.md", "[cousin](../q/y.md)\n")
        self.assertTrue(any("breaks the family rule" in e for e in self.errors()))

    def test_sideways_link_to_a_cousin_readme_passes(self):
        self.append("docs/for-humans/s/p/x.md", "[branch](../q/README.md)\n")
        self.assertEqual(self.errors(), [])

    def test_builds_on_allows_one_way_and_replies(self):
        self.write("docs/for-humans/s/p/README.md", readme("Contributors", ["x.md"], "**Builds on:** [q](../q/README.md)"))
        self.append("docs/for-humans/s/p/x.md", "[relays](../q/y.md)\n")
        self.append("docs/for-humans/s/q/y.md", "[reply](../p/x.md)\n")
        self.assertEqual(self.errors(), [])

    def test_unanswered_reverse_link_fails(self):
        self.write("docs/for-humans/s/p/README.md", readme("Contributors", ["x.md"], "**Builds on:** [q](../q/README.md)"))
        self.append("docs/for-humans/s/q/y.md", "[back](../p/x.md)\n")
        self.assertTrue(any("breaks the family rule" in e for e in self.errors()))

    def test_mutual_builds_on_fails(self):
        self.write("docs/for-humans/s/p/README.md", readme("Contributors", ["x.md"], "**Builds on:** [q](../q/README.md)"))
        self.write("docs/for-humans/s/q/README.md", readme("Contributors", ["y.md"], "**Builds on:** [p](../p/README.md)"))
        self.assertTrue(any("which builds on it" in e for e in self.errors()))

    def test_links_between_subjects_pass(self):
        self.write("docs/for-humans/README.md", readme("Users and Contributors", ["a.md", "s/README.md", "t/README.md"]))
        self.write("docs/for-humans/t/README.md", readme("Contributors", ["w/README.md"]))
        self.write("docs/for-humans/t/w/README.md", readme("Contributors", ["v.md"]))
        self.write("docs/for-humans/t/w/v.md", "[cite](../../s/p/x.md)\n")
        self.assertEqual(self.errors(), [])

    def test_reference_outside_the_folder_it_serves_fails(self):
        self.write("docs/for-humans/s/p/README.md", readme("Contributors", ["x.md"], "**Reference:** [terms](../q/y.md)"))
        self.assertTrue(any("outside the folder it serves" in e for e in self.errors()))

    def test_missing_reference_fails(self):
        self.write("docs/for-humans/s/p/README.md", readme("Contributors", ["x.md"], "**Reference:** [terms](TERMS.md)"))
        self.assertTrue(any("does not exist" in e for e in self.errors()))


if __name__ == "__main__":
    unittest.main()
