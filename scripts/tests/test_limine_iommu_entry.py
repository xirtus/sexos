# SPDX-License-Identifier: GPL-3.0-only
"""The "SexOS without DMA protection" boot entry must boot the same system as
the main entry, only with the IOMMU off (kernel/src/iommu.rs reads iommu=off)."""
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def entries(text):
    out, name = {}, None
    for line in text.splitlines():
        if line.startswith(':'):
            name = line[1:].strip(); out[name] = []
        elif name and line.strip() and not line.lstrip().startswith('#'):
            out[name].append(line.strip())
    return out


class IommuOffEntry(unittest.TestCase):
    def test_entry_matches_main_entry_with_iommu_off(self):
        e = entries((ROOT / 'limine.cfg').read_text())
        off = e['SexOS without DMA protection']
        self.assertIn('CMDLINE=iommu=off', off)
        self.assertEqual([l for l in off if l != 'CMDLINE=iommu=off'], e['SexOS'])
        self.assertEqual(list(e)[0], 'SexOS', 'the protected entry stays the default')

    def test_safe_start_matches_main_entry_with_safe(self):
        # kernel/src/lib.rs safe_start() reads the word `safe`.
        e = entries((ROOT / 'limine.cfg').read_text())
        safe = e['SexOS safe start']
        self.assertIn('CMDLINE=safe', safe)
        self.assertEqual([l for l in safe if l != 'CMDLINE=safe'], e['SexOS'])

    def test_report_entries_match_main_entry_with_autorun(self):
        # apps/spindle reads `autorun` from the kernel's [iommu.boot_options] line.
        e = entries((ROOT / 'limine.cfg').read_text())
        for name, cmd in (('SexOS report to the Mac (no typing)', 'CMDLINE=autorun'),
                          ('SexOS safe start, report to the Mac', 'CMDLINE=safe autorun')):
            self.assertIn(cmd, e[name])
            self.assertEqual([l for l in e[name] if l != cmd], e['SexOS'])


if __name__ == '__main__':
    unittest.main()
