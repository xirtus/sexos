"""Host admission checks for the diagnostic CI seed; no native boot claim."""
import hashlib
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'scripts'))
sys.path.insert(0,str(Path(__file__).resolve().parent))

class ControlledDiagnosticSeed(unittest.TestCase):
 """Synthetic host checks of CI copy admission; never a native boot claim."""
 def run_preparation(self,data):
  import json
  directory=tempfile.TemporaryDirectory();self.addCleanup(directory.cleanup)
  root=Path(directory.name);(root/'diagnostic-build').mkdir();(root/'frozen/source').mkdir(parents=True)
  (root/'frozen/source/scripts').symlink_to(ROOT/'scripts',target_is_directory=True)
  (root/'diagnostic-build/nvme.img').write_bytes(data)
  (root/'diagnostic-build/serial.log').write_text('synthetic host test')
  (root/'diagnostic-build/report.md').write_text('synthetic host test')
  (root/'frozen/tested.iso').write_bytes(b'synthetic host test')
  source=(ROOT/'.github/workflows/ci.yml').read_text()
  body=source.split("<<'PYSEED'\n",1)[1].split('        PYSEED',1)[0]
  code='\n'.join(line[8:]for line in body.splitlines())
  seed=root/'seed.img'
  with patch.object(sys,'argv',['seed',str(root),str(seed)]):exec(compile(code,'CI seed preparation','exec'),{})
  return root,seed,json.loads((root/'controlled-seed.json').read_text())
 def test_copy_preserves_all_bytes_and_records_readonly_identity(self):
  from test_https_import_gate import disks
  data,_=disks();root,seed,receipt=self.run_preparation(data)
  self.assertEqual(seed.read_bytes(),data);self.assertEqual(seed.stat().st_mode&0o777,0o444)
  self.assertEqual((root/'diagnostic-build/nvme.img').read_bytes(),data)
  self.assertEqual(receipt['source_sha256'],hashlib.sha256(data).hexdigest());self.assertEqual(receipt['seed_sha256'],hashlib.sha256(data).hexdigest())
  self.assertEqual(receipt['workspace']['rows'][0]['name'],'old-note')
 def test_blank_or_corrupt_workspace_is_rejected_without_repair(self):
  from test_https_import_gate import disks
  data,_=disks();bad=bytearray(data);bad[2046*512+16+2*32+28]^=1
  for malformed in(bytes(len(data)),bytes(bad)):
   with self.subTest(),self.assertRaises(AssertionError):self.run_preparation(malformed)

if __name__=='__main__':unittest.main()
