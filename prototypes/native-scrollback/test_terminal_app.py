import unittest

from terminal_app import facts, ready


class ObserverTests(unittest.TestCase):
    def test_facts_distinguish_visible_normal_from_retained_history(self):
        contents = "\n".join(f"NORMAL-{n}" for n in range(6))
        history = "SAVED-SENTINEL\nHARD-A\nHARD-A\n界e\u0301\n" + contents
        result = facts(contents, history)
        self.assertTrue(ready("normal", result))
        self.assertEqual(result["hard_a_copies"], 2)
        self.assertTrue(result["wide_combining_present"])
        self.assertFalse(result["alternate_in_history"])
        self.assertFalse(ready("alternate", result))

    def test_prime_requires_every_previous_visible_sentinel(self):
        result = facts("VISIBLE-5", "SAVED-SENTINEL\nVISIBLE-5")
        self.assertFalse(ready("prime", result))

    def test_shell_text_is_not_in_the_result(self):
        result = facts("unrelated shell prompt", "unrelated shell history")
        self.assertNotIn("unrelated", str(result))

    def test_regain_does_not_pass_on_an_old_screen_without_history_delivery(self):
        contents = "\n".join(f"NORMAL-{n}" for n in range(6))
        result = facts(contents, contents)
        self.assertFalse(ready("regain", result))

    def test_repeat_stage_waits_for_a_new_delivered_copy(self):
        contents = "\n".join(f"NORMAL-{n}" for n in range(6))
        before = facts(contents, "HARD-A\n" + contents)
        self.assertFalse(ready("regain", before, before))
        after = facts(contents, "HARD-A\nHARD-A\n" + contents)
        self.assertTrue(ready("regain", after, before))

    def test_numeric_shell_text_is_not_exported_as_fixture_fragments(self):
        result = facts("", "123456\n890123456789\n")
        self.assertEqual(result["synthetic_history_fragments"], [])


if __name__ == "__main__":
    unittest.main()
