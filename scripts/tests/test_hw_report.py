# SPDX-License-Identifier: GPL-3.0-only
"""scripts/hw_report.py: the verdict a real machine's `hw save` report gets."""
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import hw_report  # noqa: E402

SAMPLE = os.path.join(HERE, "fixtures", "hw_report_x17_sample.txt")


def rows_of(text):
    return {r["id"]: r for r in hw_report.verdict(hw_report.parse(text))}


class HwReport(unittest.TestCase):
    def setUp(self):
        with open(SAMPLE) as f:
            self.text = f.read()

    def test_fields_and_quoted_values(self):
        lines = hw_report.parse(self.text)
        drive = next(l for l in lines if l["tag"] == "sexdrive.nvme.drive")
        self.assertEqual(drive["fields"]["model"], "Samsung SSD 980 PRO 1TB")
        self.assertEqual(drive["fields"]["vendor"], "144d")

    def test_a_good_x17_report(self):
        r = rows_of(self.text)
        for cid in ("nvme", "nvme_format", "nvme_irq", "iommu", "display", "gpu", "wifi", "sound", "usb", "touchpad", "acpi", "crash"):
            self.assertEqual(r[cid]["state"], "ok", cid)
        # A census warning (a device without a driver) is reported, not hidden.
        self.assertEqual(r["census"]["state"], "bad")
        self.assertEqual(r["windows"]["state"], "unknown")

    def test_bad_lines_win_and_say_why(self):
        r = rows_of(self.text + "[sexdrive.nvme.namespace.refused] reason=sector_size lba_bytes=4096\n[sexdrive.nvme.irq.silent] polls=64\n")
        self.assertEqual(r["nvme_format"]["state"], "bad")
        self.assertIn("4 KiB", r["nvme_format"]["meaning"])
        self.assertEqual(r["nvme_irq"]["state"], "bad")

    def test_field_conditions(self):
        r = rows_of("[sexdrive.nvme.irq] mode=polling\n")
        self.assertEqual(r["nvme_irq"]["state"], "unknown")
        r = rows_of("[sexacpi.ready] aml=0 reason=no_tables\n")
        self.assertEqual(r["acpi"]["state"], "bad")

    def test_compare_names_fixed_and_broken(self):
        old = hw_report.verdict(hw_report.parse("[wifi.failed] step=alive\n[bell.hda.ready] vendor=1\n"))
        new = hw_report.verdict(hw_report.parse("[wifi.ready] mac=x\n[bell.hda.failed] reason=no_codec\n"))
        kinds = {q: k for k, q, _, _ in hw_report.compare(old, new)}
        self.assertEqual(kinds["Wi-Fi up"], "fixed")
        self.assertEqual(kinds["Sound (HDA codec)"], "broken")

    def test_devices_and_markdown(self):
        lines = hw_report.parse(self.text)
        devs = hw_report.devices(lines)
        self.assertEqual(devs["00:02.0"]["id"], "8086:9a60")
        md = hw_report.markdown("x17r1", hw_report.verdict(lines), devs)
        self.assertIn("| The SSD answered | ok |", md)
        self.assertIn("8086:43a4", "".join(r["evidence"][0] for r in hw_report.verdict(lines) if r["id"] == "census"))

    def test_edid_becomes_a_fuzz_seed(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(hw_report.seeds(hw_report.parse(self.text), d), 1)
            seed = os.listdir(os.path.join(d, "drv_display_intel"))[0]
            with open(os.path.join(d, "drv_display_intel", seed), "rb") as f:
                self.assertEqual(f.read()[:4], b"\x00\xff\xff\xff")


if __name__ == "__main__":
    unittest.main()
