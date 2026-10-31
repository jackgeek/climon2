import contextlib
import io
import json
import unittest
from unittest.mock import patch

from capacity import analyse, main, planned_rows, wait_for_marker


def history(indices):
    return "\n".join(f"CAP-52-{index:05}" for index in indices)


class CapacityTests(unittest.TestCase):
    def test_budget_is_rejected_instead_of_clamping_or_stressing_the_host(self):
        for limit in [20000, 19977]:
            with self.subTest(limit=limit), self.assertRaisesRegex(ValueError, "budget"):
                planned_rows(limit, 6, 20000)
        self.assertEqual(planned_rows(19975, 6, 20000) + 3, 20000)

    def test_retained_native_tail_is_contiguous_and_bounded(self):
        result = analyse(history(range(17, 278)), 278, 256, 6)
        self.assertEqual(result["first_retained"], 17)
        self.assertEqual(result["last_retained"], 277)
        self.assertEqual(result["retained_rows"], 261)

    def test_no_eviction_is_not_a_passing_capacity_result(self):
        with self.assertRaisesRegex(RuntimeError, "oldest"):
            analyse(history(range(278)), 278, 256, 6)

    def test_a_missing_middle_row_is_not_successful_oldest_first_eviction(self):
        indices = list(range(17, 278))
        indices.remove(101)
        with self.assertRaisesRegex(RuntimeError, "contiguous"):
            analyse(history(indices), 278, 256, 6)

    def test_an_overly_small_tail_does_not_pass_as_a_finite_row_cap(self):
        with self.assertRaisesRegex(RuntimeError, "configured"):
            analyse(history(range(273, 278)), 278, 256, 6)

    def test_driver_observes_fresh_markers_and_exports_only_a_bounded_tail(self):
        payloads = []

        def read(window, tty, expression):
            if expression == "name of current settings of testTab":
                return "OWNED"
            if expression == "history of testTab":
                lines = b"".join(payloads).decode().split("\r\n")
                return "\n".join(lines[-262:])
            return "24,6"

        argv = [
            "capacity.py", "--permission-confirmed", "--window-id", "258",
            "--tty", "/dev/ttys008", "--profile", "OWNED", "--limit", "256",
        ]
        output = io.StringIO()
        with patch("sys.argv", argv), patch("capacity.script", side_effect=read), \
                patch("capacity.write_tty", side_effect=lambda window, tty, data: payloads.append(data)), \
                contextlib.redirect_stdout(output):
            main()
        result = json.loads(output.getvalue())
        self.assertTrue(result["fresh_prior_content_evicted"])
        self.assertTrue(result["fresh_end_checkpoint"])
        self.assertEqual(result["emitted_newline_operations"], 281)
        self.assertEqual(result["result"]["first_retained"], 18)
        self.assertEqual(result["result"]["retained_rows"], 260)
        self.assertNotIn(b"\x1b", b"".join(payloads))

    def test_checkpoint_wait_ignores_old_content_and_fails_on_deadline(self):
        with patch("capacity.script", side_effect=["OLD", "OLD\nFRESH"]), \
                patch("capacity.time.sleep"), patch("capacity.time.monotonic", return_value=0):
            self.assertEqual(wait_for_marker(258, "/dev/ttys008", "FRESH"), "OLD\nFRESH")
        with patch("capacity.script", return_value="OLD"), \
                patch("capacity.time.monotonic", side_effect=[0, 6]):
            with self.assertRaisesRegex(RuntimeError, "checkpoint"):
                wait_for_marker(258, "/dev/ttys008", "FRESH")


if __name__ == "__main__":
    unittest.main()
