import unittest

from terminal_gui import selection_range


class SelectionTests(unittest.TestCase):
    def test_native_range_uses_utf16_after_surrogate_and_combining_text(self):
        text = "😀e\u0301\nSOFT01234567890123456789tail"
        self.assertEqual(selection_range(text, "SOFT01234567890123456789tail"), (5, 28))

    def test_missing_target_does_not_select_other_terminal_content(self):
        with self.assertRaisesRegex(RuntimeError, "known restored text is absent"):
            selection_range("other terminal content", "HARD-A\n")


if __name__ == "__main__":
    unittest.main()
