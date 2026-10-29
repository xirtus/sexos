import pathlib,struct,sys,unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from make_document_window_seed import seed
from check_document_disk import fnv,recover

def fixture():
    d=bytearray(2048*512);slot=5;entry=2046*512+16+slot*32
    d[entry]=1;struct.pack_into('<I',d,entry+24,65536);struct.pack_into('<H',d,entry+30,1)
    desc=(1925-slot)*512;d[desc:desc+4]=b'SFEX';struct.pack_into('<H',d,desc+6,1);struct.pack_into('<HH',d,desc+12,500,128)
    header=bytearray(512);header[:4]=b'QJ02';header[4]=slot
    struct.pack_into('<I',header,8,393217);struct.pack_into('<Q',header,16,1);struct.pack_into('<I',header,24,3);struct.pack_into('<Q',header,32,fnv(b'old'));struct.pack_into('<Q',header,504,fnv(header[:504]))
    d[500*512+32768:500*512+32768+512]=header;d[500*512+32768+512:500*512+32768+515]=b'old'
    return bytes(d)

class DocumentWindowSeed(unittest.TestCase):
    def test_all_payload_bounds_preserve_old_bank_metadata_and_other_bytes(self):
        original=fixture()
        for n in [0,1,12287,12288]:
            payload=b'x'*n;new,info=seed(original,5,payload)
            self.assertEqual(recover(new,5)['payload'],payload);self.assertEqual(info['generation'],2)
            self.assertEqual(new[:500*512],original[:500*512]);self.assertEqual(new[(500+128)*512:],original[(500+128)*512:])
            self.assertEqual(new[500*512+32768:(500+128)*512],original[500*512+32768:(500+128)*512])
            self.assertEqual(len(new),len(original))
    def test_oversize_or_wrong_object_fails(self):
        for slot,payload in [(1,b'x'),(15,b'x'),(5,b'x'*12289)]:
            with self.assertRaises(ValueError):seed(fixture(),slot,payload)
    def test_overlapping_or_short_mapping_fails(self):
        for start,length in [(500,1),(2047,128)]:
            d=bytearray(fixture());desc=1920*512;struct.pack_into('<H',d,desc+6,2);struct.pack_into('<HH',d,desc+16,start,length)
            with self.assertRaises(ValueError):seed(bytes(d),5,b'x')
    def test_not_a_native_journal_fails(self):
        d=bytearray(fixture());d[500*512+32768]=0
        with self.assertRaises(ValueError):seed(bytes(d),5,b'x')
if __name__=='__main__':unittest.main()
