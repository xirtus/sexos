# SPDX-License-Identifier: GPL-3.0-only
"""The "SexOS with NVIDIA GPU" entry boots the main system plus sexgpu-nv and its
firmware pack (stage N4b); no other entry starts it, so they keep the dGPU off."""
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EXTRA = ['MODULE_PATH=boot:///servers/sexgpu-nv', 'MODULE_PATH=boot:///firmware/nvidia-gsp.pack']


def entries(text):
    out, name = {}, None
    for line in text.splitlines():
        if line.startswith(':'):
            name = line[1:].strip(); out[name] = []
        elif name and line.strip() and not line.lstrip().startswith('#'):
            out[name].append(line.strip())
    return out


class NvidiaEntry(unittest.TestCase):
    def setUp(self):
        self.e = entries((ROOT / 'limine.cfg').read_text())

    def test_entry_is_the_main_entry_plus_the_gpu_service(self):
        nv = self.e['SexOS with NVIDIA GPU']
        for line in EXTRA:
            self.assertIn(line, nv)
        self.assertEqual([l for l in nv if l not in EXTRA], self.e['SexOS'])

    def test_only_that_entry_starts_the_gpu_service(self):
        for name, lines in self.e.items():
            if name != 'SexOS with NVIDIA GPU':
                self.assertFalse(any('sexgpu-nv' in l or 'nvidia-gsp.pack' in l for l in lines), name)
        self.assertEqual(list(self.e)[0], 'SexOS')

    def test_kernel_and_pack_names_agree(self):
        pdx = (ROOT / 'crates/sex-pdx/src/nvidia_gpu.rs').read_text()
        self.assertIn('SERVICE_MODULE: &str = "/sexgpu-nv"', pdx)
        self.assertIn('FIRMWARE_MODULE: &str = "nvidia-gsp.pack"', pdx)
        spec = (ROOT / 'sexos_build_spec.toml').read_text()
        self.assertIn('dest_artifact = "iso_root/servers/sexgpu-nv"', spec)
        self.assertIn('artifacts = ["iso_root/firmware/nvidia-gsp.pack"]', spec)


if __name__ == '__main__':
    unittest.main()
