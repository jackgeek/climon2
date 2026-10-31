import unittest
from unittest.mock import patch

from rendition import validate_entry, summarize, verify_layout, run_native, load_fixture


def layout(columns):
    cell, height = 7, 15
    units = []
    for index in range(46):
        if index < 7:
            column, row = index, 0
        elif index < 36:
            column, row = (index - 7) % columns, 1 + (index - 7) // columns
        elif index < 40:
            column, row = [0, 2, 2, 3][index - 36], 1 + (28 + columns - 1) // columns
        else:
            column, row = index - 40, 2 + (28 + columns - 1) // columns
        units.append({
            "utf16_index": index,
            "bounds": {"status": "observed", "rect": [
                column * cell, row * height, 14 if index == 36 else cell, height,
            ]},
        })
    return {"range_visible": True, "unit_bounds": units}


class RenditionTests(unittest.TestCase):
    def test_live_derived_title_is_not_the_owned_identity(self):
        validate_entry("shell - CLIMON2-52-NATIVE-TEST", "1", "false", "24,6", "Basic")
        for values in [
            ("other", "1", "false", "24,6", "Basic"),
            ("CLIMON2-52-NATIVE-TEST", "2", "false", "24,6", "Basic"),
            ("CLIMON2-52-NATIVE-TEST", "1", "true", "24,6", "Basic"),
        ]:
            with self.subTest(values=values), self.assertRaises(RuntimeError):
                validate_entry(*values)

    def test_unavailable_geometry_and_style_are_not_qualification(self):
        observation = {
            "target": "wide", "range_verified": True,
            "bounds": {"status": "unsupported", "ax_error": -25205},
            "style": {"status": "unsupported", "ax_error": -25205},
        }
        result = summarize(observation, "wide", "20,6", 20)
        self.assertTrue(result["logical_range_verified"])
        self.assertFalse(result["physical_geometry_qualified"])
        self.assertFalse(result["rendition_qualified"])
        self.assertEqual(result["bounds"], observation["bounds"])

    def test_wrong_range_target_size_and_api_errors_fail_explicitly(self):
        valid = {
            "target": "wide", "range_verified": True,
            "bounds": {"status": "unsupported"},
            "style": {"status": "unsupported"},
        }
        for changed in [
            dict(valid, target="hard"),
            dict(valid, range_verified=False),
            dict(valid, bounds={"status": "error", "ax_error": -25202}),
        ]:
            with self.subTest(changed=changed), self.assertRaises(RuntimeError):
                summarize(changed, "wide", "24,6", 24)
        with self.assertRaisesRegex(RuntimeError, "actual"):
            summarize(valid, "wide", "20,6", 24)

    def test_api_rectangles_and_style_attributes_do_not_prove_glyph_or_color_fidelity(self):
        result = summarize({
            "target": "wide", "range_verified": True,
            "bounds": {"status": "observed", "rect": [0, 0, 24, 12]},
            "style": {"status": "observed", "attribute_names": []},
        }, "wide", "24,6", 24)
        self.assertFalse(result["physical_geometry_qualified"])
        self.assertFalse(result["rendition_qualified"])

    def test_layout_uses_native_boxes_to_verify_wraps_and_wide_combining_cells(self):
        for columns in [20, 24, 32]:
            result = verify_layout(layout(columns), columns)
            self.assertEqual(result["ascii_cell_points"], 7)
            self.assertEqual(result["soft_physical_rows"], 1 if columns == 32 else 2)
            self.assertTrue(result["combining_box_shared"])

    def test_wrong_wide_combining_or_soft_geometry_and_offscreen_ranges_fail(self):
        for index in [36, 38, 31]:
            observation = layout(24)
            observation["unit_bounds"][index]["bounds"]["rect"][2 if index == 36 else 1] += 7
            with self.subTest(index=index), self.assertRaises(RuntimeError):
                verify_layout(observation, 24)
        with self.assertRaises(RuntimeError):
            verify_layout(dict(layout(24), range_visible=False), 24)

    def test_fixture_readiness_moves_a_previously_scrolled_viewport_to_bottom(self):
        state = {"bottom": False, "copies": 2}

        def native_gui(_window, _tty, action):
            if "set value of scroll bar 1 of targetArea to 1.0" in action:
                state["bottom"] = True

        def native_observe(_window, _tty):
            return {"hard_a_copies": state["copies"], "visible_normal": [state["bottom"]] * 6}

        def native_write(*_args):
            state["copies"] += 1

        with patch("rendition.gui", side_effect=native_gui), \
             patch("rendition.observe", side_effect=native_observe), \
             patch("rendition.write_tty", side_effect=native_write), \
             patch("rendition.time.monotonic", side_effect=[0, 10]):
            load_fixture(258, "/dev/ttys008")

    def test_resize_sequence_checks_every_size_and_restores_on_failure(self):
        with patch("rendition.entry"), patch("rendition.load_fixture"), \
             patch("rendition.checkpoint", return_value={}) as checkpoint, \
             patch("rendition.resize") as resize, \
             patch("rendition.final_state", return_value={"geometry": "24,6", "profile": "Basic", "busy": "false"}):
            result = run_native(258, "/dev/ttys008")
            self.assertEqual([call.args[2] for call in resize.call_args_list], [20, 32, 24])
            self.assertEqual([call.args[2] for call in checkpoint.call_args_list], [24, 20, 32, 24])
            self.assertEqual(len(result["checkpoints"]), 4)
            resize.reset_mock()
            checkpoint.side_effect = [{}, RuntimeError("actual 21,6")]
            with self.assertRaisesRegex(RuntimeError, "actual 21,6"):
                run_native(258, "/dev/ttys008")
            self.assertEqual([call.args[2] for call in resize.call_args_list], [20, 24])
            resize.reset_mock()
            checkpoint.side_effect = [{}, {}, {}, RuntimeError("final geometry mismatch")]
            with self.assertRaisesRegex(RuntimeError, "final geometry mismatch"):
                run_native(258, "/dev/ttys008")
            self.assertEqual([call.args[2] for call in resize.call_args_list], [20, 32, 24])


if __name__ == "__main__":
    unittest.main()
