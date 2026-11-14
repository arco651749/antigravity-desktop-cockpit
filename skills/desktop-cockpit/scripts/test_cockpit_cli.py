import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.stdout.reconfigure(encoding="utf-8")

import cmd_input
import cockpit_cli
import ws_client


class TestWsClientHelpers(unittest.TestCase):
    def test_parse_hwnd(self):
        self.assertIsNone(ws_client.parse_hwnd(None))
        self.assertEqual(ws_client.parse_hwnd(1234), 1234)
        self.assertEqual(ws_client.parse_hwnd("0x30790"), 0x30790)
        self.assertEqual(ws_client.parse_hwnd("  0X704C4  "), 0x704C4)
        self.assertEqual(ws_client.parse_hwnd("42"), 42)

    def test_normalize_center_to_uv_virtual_and_clamping(self):
        fake_mons = [
            {"primary": True, "x": 0, "y": 0, "width": 1920, "height": 1080},
            {"primary": False, "x": 3200, "y": -1000, "width": 1920, "height": 1080},
        ]
        with patch.object(ws_client, "list_system_monitors", return_value=fake_mons):
            u, v = ws_client.normalize_center_to_uv([4160, -460])
            self.assertAlmostEqual(u, 0.5, places=3)
            self.assertAlmostEqual(v, 0.5, places=3)

            # Out-of-bounds should clamp to [0.0, 1.0]
            u2, v2 = ws_client.normalize_center_to_uv([9999, 9999])
            self.assertEqual((u2, v2), (1.0, 1.0))

    def test_normalize_center_to_uv_root_rect_and_primary(self):
        fake_mons = [
            {"primary": True, "x": 0, "y": 0, "width": 1920, "height": 1080},
            {"primary": False, "x": 3200, "y": -1000, "width": 1920, "height": 1080},
        ]
        with patch.object(ws_client, "list_system_monitors", return_value=fake_mons):
            # Inside root_rect on primary monitor -> relative to root_rect
            u, v = ws_client.normalize_center_to_uv([500, 300], root_rect=[100, 100, 800, 400])
            self.assertAlmostEqual(u, 0.5, places=3)
            self.assertAlmostEqual(v, 0.5, places=3)

            # Fallback to primary monitor when outside root_rect
            u2, v2 = ws_client.normalize_center_to_uv([960, 540])
            self.assertAlmostEqual(u2, 0.5, places=3)
            self.assertAlmostEqual(v2, 0.5, places=3)


class TestCockpitCliParser(unittest.TestCase):
    def test_subcommand_parsing(self):
        parser = cockpit_cli.build_parser()

        args_type = parser.parse_args(["type", "hello 世界", "--hwnd", "0x30790"])
        self.assertEqual(args_type.command, "type")
        self.assertEqual(args_type.text, "hello 世界")
        self.assertEqual(args_type.hwnd, "0x30790")

        args_drag = parser.parse_args(["drag", "0.1", "0.2", "0.8", "0.9", "-b", "middle", "--steps", "10"])
        self.assertEqual(args_drag.command, "drag")
        self.assertEqual(args_drag.button, "middle")
        self.assertEqual(args_drag.steps, 10)

        args_fill = parser.parse_args(["fill", "@3", "test", "--hwnd", "0x10"])
        self.assertEqual(args_fill.command, "fill")
        self.assertEqual(args_fill.target, "@3")
        self.assertEqual(args_fill.text, "test")

        args_disp = parser.parse_args(["display", "status"])
        self.assertEqual(args_disp.command, "display")
        self.assertEqual(args_disp.action, "status")


if __name__ == "__main__":
    unittest.main()
