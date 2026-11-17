"""The install boot image grows with what it holds instead of failing at 32 MiB."""
import importlib.util
import unittest
from pathlib import Path

spec=importlib.util.spec_from_file_location('make_boot_image',Path(__file__).resolve().parents[1]/'make_boot_image.py')
mod=importlib.util.module_from_spec(spec);spec.loader.exec_module(mod)
MIB=1<<20


class BootImageSize(unittest.TestCase):
    def test_small_system_keeps_12_mib(self):
        self.assertEqual(mod.sectors_for([3*MIB,4*MIB],0),mod.SECTORS)

    def test_firmware_pack_takes_at_least_32_mib(self):
        self.assertEqual(mod.sectors_for([3*MIB,14*MIB],14*MIB),mod.SECTORS_WITH_FIRMWARE)

    def test_outgrowing_32_mib_rounds_up_to_whole_mib(self):
        # 2026-11-17: kernel, modules and a 13.8 MB Wi-Fi pack passed 32 MiB and mcopy failed.
        sizes=[3*MIB]+[300*1024]*80+[13795328]
        sectors=mod.sectors_for(sizes,13795328)
        self.assertGreater(sectors,mod.SECTORS_WITH_FIRMWARE)
        self.assertEqual(sectors%2048,0)
        self.assertGreaterEqual(sectors*512,sum(sizes)+2*MIB)

    def test_more_than_the_system_slot_is_refused(self):
        with self.assertRaises(SystemExit):mod.sectors_for([300*MIB],0)


if __name__=='__main__':unittest.main()
