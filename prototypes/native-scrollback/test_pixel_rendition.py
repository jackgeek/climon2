import unittest
import json
from pathlib import Path
import subprocess
from unittest.mock import patch

from pixel_rendition import owned_window, pixel_box, sample_cell, qualify, capture
from test_rendition import layout


class PixelRenditionTests(unittest.TestCase):
    def test_capture_missing_screenshot_refuses_and_deletes_private_directory(self):
        title = "synthetic CLIMON2-52-NATIVE-TEST"
        metadata = {"id": 258, "owner": "Terminal", "title": title, "permitted": True}
        image_paths = []
        def native_command(args, **kwargs):
            if args[0] == "screencapture":
                image_paths.append(Path(args[-1]))
                self.assertEqual(image_paths[-1].parent.stat().st_mode & 0o777, 0o700)
                return subprocess.CompletedProcess(args, 0)
            return subprocess.CompletedProcess(args, 0, stdout=json.dumps(metadata))
        with patch("pixel_rendition.subprocess.run", side_effect=native_command):
            with self.assertRaisesRegex(RuntimeError, "screenshot unavailable"):
                capture(title, {}, 24, {})
        self.assertEqual(len(image_paths), 1)
        self.assertFalse(image_paths[0].parent.exists())

    def test_capture_refuses_missing_ambiguous_wrong_owner_or_permission(self):
        title = "synthetic CLIMON2-52-NATIVE-TEST"
        candidate = {"id": 258, "owner": "Terminal", "title": title}
        self.assertEqual(owned_window([candidate], title, True), candidate)
        for candidates, permitted in [
            ([], True), ([candidate, candidate], True),
            ([dict(candidate, owner="Other")], True),
            ([dict(candidate, title="unrelated")], True),
            ([candidate], False),
        ]:
            with self.subTest(candidates=candidates, permitted=permitted):
                with self.assertRaises(RuntimeError):
                    owned_window(candidates, title, permitted)

    def test_native_points_map_at_measured_scale_and_reject_clipping(self):
        window = [60, 40, 200, 120]
        viewport = [68, 61, 168, 75]
        self.assertEqual(
            pixel_box([68, 61, 14, 15], viewport, window, [400, 240]),
            [16, 42, 28, 30],
        )
        self.assertEqual(
            pixel_box([68, 61, 7, 15], viewport, window, [200, 120]),
            [8, 21, 7, 15],
        )
        for box, image in [
            ([68, 60, 7, 15], [400, 240]),
            ([230, 61, 7, 15], [400, 240]),
            ([68, 61, 7, 0], [400, 240]),
            ([68, 61, 7, 15], [400, 120]),
            ([68, 61, 7, 15], [0, 0]),
            ([68, 61, float("nan"), 15], [400, 240]),
        ]:
            with self.subTest(box=box, image=image), self.assertRaises(RuntimeError):
                pixel_box(box, viewport, window, image)

    def test_ink_and_relative_blue_reject_blank_wrong_or_ambiguous_colour(self):
        white = (255, 255, 255)
        black = (0, 0, 0)
        blue = (0, 0, 180)
        def image(ink):
            pixels = [white] * 100
            for index in [22, 23, 24, 32, 33, 34, 42, 43, 44, 52, 53, 54]:
                pixels[index] = ink
            return bytes(channel for pixel in pixels for channel in pixel)
        self.assertEqual(sample_cell(image(white), [10, 10], [0, 0, 10, 10], white, black, "blank")["ink_pixels"], 0)
        self.assertEqual(sample_cell(image(black), [10, 10], [0, 0, 10, 10], white, black, "text")["ink_pixels"], 12)
        self.assertEqual(sample_cell(image(blue), [10, 10], [0, 0, 10, 10], white, black, "blue")["dominant_rgb"], [0, 0, 180])
        for ink, expected in [
            (white, "text"), (black, "blue"), ((180, 0, 0), "blue"),
            ((100, 90, 115), "blue"), (blue, "text"), (black, "blank"),
        ]:
            with self.subTest(ink=ink, expected=expected), self.assertRaises(RuntimeError):
                sample_cell(image(ink), [10, 10], [0, 0, 10, 10], white, black, expected)

    def test_unavailable_truncated_and_low_contrast_images_never_pass(self):
        for pixels in [b"", bytes(299), bytes([240] * 300)]:
            with self.subTest(length=len(pixels)), self.assertRaises(RuntimeError):
                sample_cell(pixels, [10, 10], [0, 0, 10, 10], [255] * 3, [0] * 3, "text")

    def test_default_text_accepts_antialiasing_on_independent_colour_segment(self):
        pixels = bytes([255] * 120 + [120] * 60 + [255] * 120)
        result = sample_cell(pixels, [10, 10], [0, 0, 10, 10], [255] * 3, [0] * 3, "text")
        self.assertEqual(result["colour_match_fraction"], 1)
        tinted = bytes([255] * 120 + [120, 120, 155] * 20 + [255] * 120)
        with self.assertRaisesRegex(RuntimeError, "disagrees"):
            sample_cell(tinted, [10, 10], [0, 0, 10, 10], [255] * 3, [0] * 3, "text")

    def test_full_oracle_requires_wide_ink_in_both_cells_and_blank_neighbour(self):
        observation = dict(layout(24), viewport_rect=[0, 0, 168, 90], selection_cleared=True)
        palette = {"text": [0, 0, 0], "background": [255, 255, 255]}
        pixels = bytearray([255] * (200 * 90 * 3))
        def paint(x, y, color):
            for row in range(y, y + 8):
                offset = (row * 200 + x) * 3
                pixels[offset:offset + 3] = bytes(color)
        for x in [2, 9, 16, 23, 30, 37]:
            paint(x, 3, [0, 0, 0])
            paint(x, 63, [0, 0, 0])
        for x in range(2, 168, 7):
            paint(x, 18, [0, 0, 0])
        for x in [2, 9, 16, 23]:
            paint(x, 33, [0, 0, 0])
        paint(2, 48, [0, 0, 180])
        paint(9, 48, [0, 0, 180])
        paint(16, 48, [0, 0, 0])
        result = qualify(pixels, [200, 90], [0, 0, 200, 90], observation, 24, palette)
        self.assertTrue(result["wide_ink_in_both_cells"])
        self.assertEqual(len(result["ascii"]), 40)
        with self.assertRaisesRegex(RuntimeError, "highlighting"):
            qualify(pixels, [200, 90], [0, 0, 200, 90], dict(observation, selection_cleared=False), 24, palette)
        paint(9, 48, [255, 255, 255])
        with self.assertRaisesRegex(RuntimeError, "ink missing"):
            qualify(pixels, [200, 90], [0, 0, 200, 90], observation, 24, palette)
        paint(9, 48, [0, 0, 180])
        paint(23, 48, [0, 0, 0])
        with self.assertRaisesRegex(RuntimeError, "not blank"):
            qualify(pixels, [200, 90], [0, 0, 200, 90], observation, 24, palette)


if __name__ == "__main__":
    unittest.main()
