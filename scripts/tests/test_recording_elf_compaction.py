import importlib.util,struct,unittest
from pathlib import Path
path=Path(__file__).resolve().parents[1]/'strip_recording_boot.py'
spec=importlib.util.spec_from_file_location('record_strip',path);module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
class ELFCompaction(unittest.TestCase):
    def fixture(self):
        # Synthetic header/load/stack fixture, not runnable guest code.
        b=bytearray(240);b[:7]=b'\x7fELF\x02\x01\x01'
        struct.pack_into('<HHIQQQ',b,16,3,62,1,176,64,240)
        struct.pack_into('<HHHHHH',b,52,64,56,2,64,1,0)
        struct.pack_into('<IIQQQQQQ',b,64,1,5,0,0,0,240,4096,4096)
        struct.pack_into('<IIQQQQQQ',b,120,0x6474e551,6,0,0,0,0,1048576,16)
        b[176:]=bytes(range(64));return bytes(b)
    def test_only_section_metadata_may_change(self):
        before=self.fixture();after=bytearray(before)
        after[40:48]=b'\0'*8;after[58:64]=b'\0'*6
        self.assertEqual(len(module.validate(before,bytes(after))),1)
    def test_runtime_headers_stack_and_loaded_payload_cannot_change(self):
        before=self.fixture()
        for offset in (24,32,54,64,120,160,180,239):
            after=bytearray(before);after[offset]^=1
            with self.subTest(offset=offset),self.assertRaises(AssertionError):module.validate(before,bytes(after))
    def test_truncation_and_wrong_architecture_reject(self):
        before=self.fixture()
        with self.assertRaises(AssertionError):module.validate(before,before[:-1])
        changed=bytearray(before);changed[18]=3
        with self.assertRaises(AssertionError):module.validate(before,bytes(changed))
if __name__=='__main__':unittest.main()
