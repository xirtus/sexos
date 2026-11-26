"""Tests that make_wifi_firmware_pack.py bundles both iwlwifi-* and ibt-* blobs."""
import importlib.util
import struct
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "make_wifi_firmware_pack",
    Path(__file__).resolve().parents[1] / "make_wifi_firmware_pack.py",
)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)


def parse_entries(blob: bytes):
    assert blob[:8] == b"SEXWIFW2"
    count, _ = struct.unpack_from("<II", blob, 8)
    out = {}
    for i in range(count):
        e = blob[16 + i * 88 : 16 + (i + 1) * 88]
        name = e[:44].split(b"\0", 1)[0].decode()
        inflated, off, length = struct.unpack_from("<III", e, 44)
        out[name] = (inflated, blob[off : off + length])
    return out


class MakeWifiFirmwarePackTests(unittest.TestCase):
    def test_packs_wifi_compressed_and_bt_stored(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            wifi = root / "wifi"
            bt = root / "bt"
            wifi.mkdir()
            bt.mkdir()
            (wifi / "iwlwifi-ty-a0-gf-a0-89.ucode").write_bytes(b"W" * (80 * 1024))
            (wifi / "iwlwifi-ty-a0-gf-a0.pnvm").write_bytes(b"pnvm-data")
            (wifi / "iwlwifi-incomplete.part").write_bytes(b"skip")
            (bt / "ibt-0041-0041.sfi").write_bytes(b"B" * (80 * 1024))
            (bt / "ibt-0041-0041.ddc").write_bytes(b"ddc-data")
            (bt / "ibt-incomplete.part").write_bytes(b"skip")

            files = mod.collect_files(wifi, bt)
            self.assertEqual(
                [f.name for f in files],
                [
                    "ibt-0041-0041.ddc",
                    "ibt-0041-0041.sfi",
                    "iwlwifi-ty-a0-gf-a0-89.ucode",
                    "iwlwifi-ty-a0-gf-a0.pnvm",
                ],
            )
            blob = mod.build(files)
            entries = parse_entries(blob)
            # Large Wi-Fi ucode is DEFLATE-compressed (inflated > 0).
            self.assertEqual(entries["iwlwifi-ty-a0-gf-a0-89.ucode"][0], 80 * 1024)
            # Large BT .sfi MUST remain stored uncompressed (inflated == 0) for sex_bt::pack::find.
            self.assertEqual(entries["ibt-0041-0041.sfi"][0], 0)
            self.assertEqual(entries["ibt-0041-0041.sfi"][1], b"B" * (80 * 1024))
            self.assertEqual(entries["ibt-0041-0041.ddc"], (0, b"ddc-data"))


if __name__ == "__main__":
    unittest.main()
