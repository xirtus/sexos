"""The install image (loaded into RAM on every boot entry) never carries the 38 MB NVIDIA
GSP pack, and installs get no NVIDIA entry, whose firmware lives only on the ISO."""
import importlib.util
import unittest
from pathlib import Path

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('make_boot_image',ROOT/'scripts'/'make_boot_image.py')
mod=importlib.util.module_from_spec(spec);spec.loader.exec_module(mod)


class InstalledConfig(unittest.TestCase):
    def setUp(self):
        self.iso=(ROOT/'limine.cfg').read_text()
        self.cfg=mod.installed_cfg(self.iso,True)

    def test_no_nvidia_entry_or_pack(self):
        self.assertIn(mod.NVIDIA_ENTRY+'\n',self.iso)
        self.assertNotIn('NVIDIA',self.cfg)
        self.assertNotIn('nvidia-gsp.pack',self.cfg)
        self.assertNotIn('sexgpu-nv',self.cfg)

    def test_other_entries_kept_whole(self):
        iso_entries=[l for l in self.iso.splitlines() if l.startswith(':') and l!=mod.NVIDIA_ENTRY]
        self.assertEqual(iso_entries,[l for l in self.cfg.splitlines() if l.startswith(':')])
        self.assertNotIn('boot:///boot.img',self.cfg)
        self.assertIn('boot:///firmware/wifi.pack',self.cfg)
        self.assertNotIn('boot:///firmware/wifi.pack',mod.installed_cfg(self.iso,False))

    def test_image_never_copies_the_pack(self):
        src=(ROOT/'scripts'/'make_boot_image.py').read_text()
        self.assertNotIn("'nvidia-gsp.pack'",src)


if __name__=='__main__':unittest.main()
