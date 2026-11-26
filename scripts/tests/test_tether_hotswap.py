# SPDX-License-Identifier: GPL-3.0-only
"""Host unit tests for scripts/tether_host.py live server hot-swap framing."""
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import tether_host


class TetherHotswapTests(unittest.TestCase):
    def test_fnv64_matches_standard_vector(self):
        self.assertEqual(tether_host.fnv64(b""), 0xcbf29ce484222325)
        self.assertEqual(tether_host.fnv64(b"foobar"), 0x85944171f73967e8)

    def test_build_hotswap_frame_roundtrip(self):
        elf = b"\x7fELF" + b"\x00" * 60
        frame = tether_host.build_hotswap_frame("sexbell", elf)
        header, payload = frame.split(b"\n", 1)
        parts = header.decode("ascii").split()
        self.assertEqual(parts[0], "SEXOS-HOTSWAP")
        self.assertEqual(parts[1], "sexbell")
        self.assertEqual(int(parts[2]), len(elf))
        self.assertEqual(int(parts[3], 16), tether_host.fnv64(elf))
        self.assertEqual(payload, elf)

    def test_build_hotswap_frame_rejects_bad_inputs(self):
        with self.assertRaises(ValueError):
            tether_host.build_hotswap_frame("sexbell", b"NOT_ELF")
        with self.assertRaises(ValueError):
            tether_host.build_hotswap_frame("sexbell", b"")
        with self.assertRaises(ValueError):
            tether_host.build_hotswap_frame("sexbell", b"\x7fELF" + b"\x00" * tether_host.HOTSWAP_MAX_BYTES)
        with self.assertRaises(ValueError):
            tether_host.build_hotswap_frame("bad name", b"\x7fELF" + b"\x00" * 60)


if __name__ == "__main__":
    unittest.main()
