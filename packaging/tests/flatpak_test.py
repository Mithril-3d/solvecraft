"""Fast failure-path checks for the desktop Flatpak process probe."""
import importlib.util
import json
import struct
import zlib
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("flatpak_smoke", ROOT / "packaging/linux/flatpak-smoke.py")
SMOKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE)


def healthy_replies():
    return [
        {"id": 1, "ok": True, "result": {"renderer": "gpu", "viewport": [250, 126, 1000, 700]}},
        {"id": 2, "ok": True, "result": {"feature": 1, "bodies": ["Body1"]}},
        {"id": 3, "ok": True, "result": {
            "timeline": [{"error": None}],
            "bodies": [{"volume_mm3": 20 * 30 * 10, "bbox": {"min": [0, 0, 0], "max": [30, 20, 10]}}],
        }},
    ]


def output(replies):
    return "\n".join(json.dumps(reply) for reply in replies)


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(data, zlib.crc32(kind)))


def rgba_png(width, height, rows):
    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


class DesktopProbe(unittest.TestCase):
    def test_real_presented_frame_is_required_not_just_responsive_control(self):
        black = rgba_png(2, 2, b"\x00" + bytes([0, 0, 0, 255]) * 2 + b"\x00" + bytes([0, 0, 0, 255]) * 2)
        with self.assertRaisesRegex(ValueError, "blank presented frame"):
            SMOKE.frame_statistics(black)
        # Independently encoded two rows: red/green, blue/white.
        colour = rgba_png(2, 2, b"\x00" + bytes([255, 0, 0, 255, 0, 255, 0, 255])
                          + b"\x00" + bytes([0, 0, 255, 255, 255, 255, 255, 255]))
        self.assertEqual(SMOKE.frame_statistics(colour), {"width": 2, "height": 2, "nonblack": 4, "pixels": 4})
        # Same pixels encoded with PNG Sub filtering (not a second decoder).
        sub = rgba_png(2, 2, b"\x01" + bytes([255, 0, 0, 255, 1, 255, 0, 0])
                       + b"\x01" + bytes([0, 0, 255, 255, 255, 255, 0, 0]))
        self.assertEqual(SMOKE.frame_statistics(sub), SMOKE.frame_statistics(colour))

    def test_frame_byte_dimensions_and_decompression_are_bounded(self):
        with self.assertRaisesRegex(ValueError, "dimensions"):
            SMOKE.frame_statistics(rgba_png(SMOKE.MAX_PIXELS + 1, 1, b""))
        with self.assertRaisesRegex(ValueError, "decompressed size"):
            SMOKE.frame_statistics(rgba_png(1, 1, b"\x00" * 10000))
        truncated = rgba_png(1, 1, b"\x00\xff\x00\x00\xff")[:-1]
        with self.assertRaises(ValueError):
            SMOKE.frame_statistics(truncated)

    def test_ui_thread_and_independent_box_measurement_are_required(self):
        replies = healthy_replies()
        self.assertEqual(SMOKE.validate_replies(output(replies)), "gpu")
        replies[2]["result"]["bodies"][0]["volume_mm3"] = 5900
        with self.assertRaises(ValueError):
            SMOKE.validate_replies(output(replies))

    def test_cli_version_or_partial_control_output_is_not_desktop_success(self):
        with self.assertRaises(ValueError):
            SMOKE.validate_replies("solvecraft-cli 0.3.0\n")
        with self.assertRaises(ValueError):
            SMOKE.validate_replies(output(healthy_replies()[:1]))

    def test_startup_without_viewport_or_failed_feature_is_not_success(self):
        for failure in ["viewport", "feature", "control", "bounds"]:
            with self.subTest(failure=failure):
                replies = healthy_replies()
                if failure == "viewport":
                    replies[0]["result"]["viewport"][2] = 0
                elif failure == "feature":
                    replies[2]["result"]["timeline"][0]["error"] = "operation failed"
                elif failure == "control":
                    replies[1] = {"id": 2, "ok": False, "error": "startup failure"}
                else:
                    replies[2]["result"]["bodies"][0]["bbox"]["max"][0] = 29
                with self.assertRaises(ValueError):
                    SMOKE.validate_replies(output(replies))


if __name__ == "__main__":
    unittest.main()
