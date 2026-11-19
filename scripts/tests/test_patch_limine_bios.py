# SPDX-License-Identifier: GPL-3.0-only
"""scripts/patch_limine_bios.py: the one-byte Limine countdown fix."""
import gzip
import hashlib
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import patch_limine_bios as plb  # noqa: E402

# cmp dword [0xf212], edx ; je +0x2e ; push ecx ; push edx ; mov ah, 1
LOOP = bytes.fromhex("66391612f2742e66516652b401")


class PatchLimineBios(unittest.TestCase):
    def setUp(self):
        self.saved = (plb.STAGE2, plb.STAGE2_FIXED)
        self.stage2 = b"\x90" * 600 + LOOP + bytes(range(256)) * 40
        fixed = plb.fix_loop(self.stage2)
        plb.STAGE2 = hashlib.sha256(self.stage2).hexdigest()
        plb.STAGE2_FIXED = hashlib.sha256(fixed).hexdigest()

    def tearDown(self):
        plb.STAGE2, plb.STAGE2_FIXED = self.saved

    def test_loop_exit_becomes_unsigned_at_least(self):
        fixed = plb.fix_loop(self.stage2)
        self.assertEqual(fixed[600:613], bytes.fromhex("66391612f2732e66516652b401"))
        self.assertEqual(len(fixed), len(self.stage2))
        with self.assertRaises(ValueError):
            plb.fix_loop(self.stage2 + LOOP)

    def test_stream_patch_keeps_size_and_is_idempotent(self):
        image = b"BOOT" * 100 + gzip.compress(self.stage2, 1, mtime=0) + b"AFTER"
        out = plb.fix_stream(image)
        self.assertEqual(len(out), len(image))
        self.assertTrue(out.endswith(b"AFTER"))
        self.assertEqual(plb.fix_stream(out), out)

    def test_unknown_stage2_is_refused(self):
        with self.assertRaises(ValueError):
            plb.fix_stream(gzip.compress(b"something else", mtime=0))

    def test_c_array_round_trip(self):
        text = "const uint8_t x[] = {\n    0x01, 0xff,\n};\n"
        data = plb.c_array(text)
        self.assertEqual(data, b"\x01\xff")
        self.assertEqual(plb.c_array(plb.to_c_array(text, data)), data)

    def test_unknown_files_are_refused(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "limine-bios.sys"
            p.write_bytes(b"\x00" * 64 + LOOP)
            with self.assertRaises(ValueError):
                plb.patch_file(p)
            q = Path(d) / "other.bin"
            q.write_bytes(b"")
            with self.assertRaises(ValueError):
                plb.patch_file(q)


if __name__ == "__main__":
    unittest.main()
