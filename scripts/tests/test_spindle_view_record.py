import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from spindle_view_record import pane

def record(paint,data,base=0,prev=None,seq=2):
 head=f'[spindle.view.text] target=12 sid=154 sequence={seq} paint={paint} cols=1 rows=1 bytes={len(data)}'+(f' base={base}' if base is not None else '')+'\n'
 for off in range(0,len(data),48):
  part=data[off:off+48]
  if prev is not None and prev[off:off+48]==part:continue
  head+=f'[spindle.view.bytes] target=12 sid=154 seq={seq} paint={paint} off={off} hex={part.hex()}\n'
 return head

class Record(unittest.TestCase):
 def test_full_and_delta(self):
  a=bytes(range(200));b=bytearray(a);b[100]=0xff;b=bytes(b);c=bytearray(b);c[5]=1;c=bytes(c)
  trace=record(1,a)+record(2,b,1,a)+record(3,c,2,b)
  self.assertEqual(trace.count('view.bytes'),5+1+1)
  self.assertEqual(pane(trace,12,154,2,1),a);self.assertEqual(pane(trace,12,154,2,2),b);self.assertEqual(pane(trace,12,154,2,3),c)
 def test_older_record_without_base(self):
  a=b'hello\n'*20;self.assertEqual(pane(record(4,a,None),12,154,2,4),a)
 def test_missing_base_or_chunk(self):
  a=bytes(100);b=bytes([1])*100
  with self.assertRaises(ValueError):pane(record(2,b,1,a),12,154,2,2)
  with self.assertRaises(ValueError):pane(record(1,a).splitlines()[0]+'\n',12,154,2,1)
 def test_unchanged_paint_has_no_chunks(self):
  a=bytes(range(96));trace=record(1,a)+record(2,a,1,a)
  self.assertEqual(pane(trace,12,154,2,2),a)

if __name__=='__main__':unittest.main()
