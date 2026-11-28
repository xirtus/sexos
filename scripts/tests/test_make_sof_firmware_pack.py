"""make_sof_firmware_pack.py: the Tiger Lake SOF images and Intel's licence as one
stored SEXWIFW2 pack (sex_wifi::pack), named as sex_pdx::audio_dsp expects, and wired
into the image build like the Wi-Fi and NVIDIA packs."""
import hashlib
import importlib.util
import re
import struct
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("make_sof_firmware_pack", ROOT / "scripts" / "make_sof_firmware_pack.py")
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
        out[name] = (inflated, off, blob[off : off + length], e[56:88])
    return out


class MakeSofFirmwarePack(unittest.TestCase):
    def test_images_and_licence_stored_aligned_and_hashed(self):
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            (d / "sof-tgl-h.ri").write_bytes(b"H" * 5000)
            (d / "sof-tgl.ri").write_bytes(b"L" * 70000)
            (d / "LICENCE.Intel").write_bytes(b"BSD 3 Clause")
            (d / "sof-tgl.ri.part").write_bytes(b"skip")
            files = mod.collect_files(d)
            entries = parse_entries(mod.build(files))
            self.assertEqual(sorted(entries), ["LICENCE.Intel", "sof-tgl-h.ri", "sof-tgl.ri"])
            for name, (inflated, off, data, sha) in entries.items():
                self.assertEqual(inflated, 0, name)  # never compressed
                self.assertEqual(off % 4096, 0, name)
                self.assertEqual(data, (d / name).read_bytes())
                self.assertEqual(sha, hashlib.sha256(data).digest())

    def test_no_images_gives_an_empty_pack(self):
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            (d / "LICENCE.Intel").write_bytes(b"licence only")
            self.assertEqual(mod.collect_files(d), [])
            self.assertEqual(mod.collect_files(d / "missing"), [])
            blob = mod.build([])
            self.assertEqual(parse_entries(blob), {})
            self.assertEqual(len(blob), 4096)

    def test_names_agree_with_the_kernel_and_fetch(self):
        pdx = (ROOT / "crates/sex-pdx/src/audio_dsp.rs").read_text()
        self.assertIn('FIRMWARE_MODULE: &str = "sof.pack"', pdx)
        for image in mod.IMAGES:
            self.assertIn(f'"{image}"', pdx)
        fetch = (ROOT / "scripts/fetch_sof_firmware.sh").read_text()
        for name in mod.IMAGES + (mod.LICENCE,):
            self.assertRegex(fetch, rf"(\n|LIST=')[0-9a-f]{{64}} {re.escape(name)} ")
        self.assertIn("sof/intel-signed/sof-tgl-h.ri", fetch)

    def test_every_entry_with_wifi_pack_carries_sof_pack(self):
        cfg = (ROOT / "limine.cfg").read_text()
        self.assertEqual(cfg.count("boot:///firmware/wifi.pack"), cfg.count("boot:///firmware/sof.pack"))
        build = (ROOT / "scripts/sexos_build_trace.sh").read_text()
        self.assertIn("make_sof_firmware_pack.py --firmware target/sof-firmware --out iso_root/firmware/sof.pack", build)
        self.assertIn('artifacts = ["iso_root/firmware/sof.pack"]', (ROOT / "sexos_build_spec.toml").read_text())

    def test_install_image_keeps_the_line_only_with_the_pack(self):
        spec = importlib.util.spec_from_file_location("make_boot_image", ROOT / "scripts" / "make_boot_image.py")
        boot = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(boot)
        cfg = (ROOT / "limine.cfg").read_text()
        self.assertIn("boot:///firmware/sof.pack", boot.installed_cfg(cfg, True, True))
        self.assertNotIn("boot:///firmware/sof.pack", boot.installed_cfg(cfg, True, False))
        self.assertIn("'::/firmware/sof.pack'", (ROOT / "scripts" / "make_boot_image.py").read_text())


if __name__ == "__main__":
    unittest.main()
