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


if __name__ == '__main__':
    unittest.main()
