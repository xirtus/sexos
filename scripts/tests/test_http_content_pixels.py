"""Synthetic regressions for the actual composed Browser HTTP/body oracle."""
import sys,unittest,hashlib
from unittest.mock import patch
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from http_content_gate import BODY,load_pixel_contract,http_status_geometry,verify_http_pixels
from check_desktop_pixels import glyph
ROOT=Path(__file__).resolve().parents[2]

class HttpPixels(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.contract=load_pixel_contract(ROOT)
 def trace(self,length=105,w=640,h=640):
  return f'[shell.tile.apply] sid=300 x=600 y=100 w={w} h={h}\n[browser.preview.layout] bytes={length} newline=1 cached_or_sample=1\n[browser.net.job.view] state=Verified web=0\n'
 def framebuffer(self,text,view='Ready',body=BODY,shift=0):
  g=http_status_geometry(text,view,body,self.contract);pixels=bytearray(1280*800*3)
  for row in g['rows']:
   for column,char in enumerate(row['text']):
    for gy,bits in enumerate(glyph(self.contract['font'],char)):
     for gx in range(5):
      if not bits&(1<<(4-gx)):continue
      for dy in range(g['scale']):
       for dx in range(g['scale']):
        x=row['x']+(column*self.contract['advance']+gx)*g['scale']+dx;y=row['y']+gy*g['scale']+dy+shift;pixels[(y*1280+x)*3:(y*1280+x)*3+3]=self.contract['color'].to_bytes(3,'big')
  return b'P6\n1280 800\n255\n'+pixels
 def test_shared_codec_and_facade_are_both_in_the_pixel_source_closure(self):
  for name in ('crates/sex-http/src/codec.rs','crates/sex-http/src/lib.rs','crates/sex-http/Cargo.toml','apps/kaleidoscope/src/http_wire.rs','crates/sex-net-config/src/lib.rs'):
   self.assertEqual(self.contract['source_sha256'][name],hashlib.sha256((ROOT/name).read_bytes()).hexdigest())
  self.assertEqual(self.contract['wire_max_body'],12288);self.assertEqual(self.contract['wire_max_headers'],1024)
 def test_facade_cannot_silently_point_to_a_different_or_duplicate_codec(self):
  actual=Path.read_text;facade=(ROOT/'apps/kaleidoscope/src/http_wire.rs').resolve()
  for change in ('path','implementation'):
   def read(path,*args,**kwargs):
    source=actual(path,*args,**kwargs)
    if path.resolve()==facade:
     return source.replace('crates/sex-http/src/codec.rs','crates/other-http/src/codec.rs')if change=='path'else source+'\npub const MAX_BODY:usize=1;\n'
    return source
   with patch.object(Path,'read_text',read):
    with self.assertRaisesRegex(ValueError,'single shared codec'):load_pixel_contract(ROOT)
 def test_ready_full_body_and_failure_regions(self):
  for view in ('Ready','Unavailable','Cancelled','Uncertain'):
   with self.subTest(view=view):
    text=self.trace();result=verify_http_pixels(text,self.framebuffer(text,view),view,BODY,self.contract);self.assertTrue(result['ok'],result)
 def test_ready_prefix_without_full_body_cannot_pass(self):
  text=self.trace();self.assertFalse(verify_http_pixels(text,self.framebuffer(text,body=BODY[:-1]),'Ready',BODY,self.contract)['ok'])
 def test_previous_ready_body_must_be_erased_for_failure_or_cancel(self):
  text=self.trace();ready=self.framebuffer(text);header=len(b'P6\n1280 800\n255\n')
  for view in('Unavailable','Cancelled'):
   # Keep the correct failure/cancel label; inject only the old ready body's
   # pixels, so this regression specifically requires body-region erasure.
   g=http_status_geometry(text,view,BODY,self.contract);ghost=bytearray(self.framebuffer(text,view))
   for row in g['rows']:
    if not row['body']:continue
    for y in range(row['y'],row['y']+self.contract['line']*g['scale']):
     start=header+(y*1280+row['x'])*3;end=start+g['columns']*self.contract['advance']*g['scale']*3;ghost[start:end]=ready[start:end]
   self.assertFalse(verify_http_pixels(text,bytes(ghost),view,BODY,self.contract)['ok'])
 def test_plain_http_label_is_required_not_only_body(self):
  text=self.trace();g=http_status_geometry(text,'Ready',BODY,self.contract);data=bytearray(self.framebuffer(text));header=len(b'P6\n1280 800\n255\n');row=next(r for r in g['rows']if 'Plain HTTP'in r['text'])
  # Retain every correct body pixel and erase only the plaintext/no TLS label.
  for y in range(row['y'],row['y']+self.contract['line']*g['scale']):
   start=header+(y*1280+row['x'])*3;end=start+g['columns']*self.contract['advance']*g['scale']*3;data[start:end]=b'\0'*(end-start)
  self.assertFalse(verify_http_pixels(text,bytes(data),'Ready',BODY,self.contract)['ok'])
 def test_blank_separator_and_trailing_body_cells_checked(self):
  text=self.trace();g=http_status_geometry(text,'Ready',BODY,self.contract);raw=self.framebuffer(text)
  for row in(g['rows'][0],g['rows'][-1]):
   data=bytearray(raw);x=row['x']+(g['columns']-1)*self.contract['advance']*g['scale'];offset=len(b'P6\n1280 800\n255\n')+(row['y']*1280+x)*3;data[offset:offset+3]=self.contract['color'].to_bytes(3,'big');self.assertFalse(verify_http_pixels(text,bytes(data),'Ready',BODY,self.contract)['ok'])
 def test_dynamic_length_width_and_prior_dns_change_rows(self):
  values=[]
  for length,width in((60,640),(139,640),(60,310),(105,250)):
   text=self.trace(length,width)+'[browser.dns.view] state=Answer([100, 255, 0, 1]) web=0\n';g=http_status_geometry(text,'Ready',BODY,self.contract);values.append(g);self.assertEqual(g['http_offset'],length+128);self.assertTrue(verify_http_pixels(text,self.framebuffer(text),'Ready',BODY,self.contract)['ok'])
  self.assertNotEqual(values[0]['rows'][0]['row'],values[1]['rows'][0]['row']);self.assertNotEqual(values[1]['columns'],values[2]['columns']);self.assertEqual(values[-1]['scale'],1)
 def test_clip_missing_context_and_oversize_rejected(self):
  for text in('',self.trace(h=150),self.trace(length=255),self.trace(length=139,w=310),self.trace().replace('[browser.net.job.view] state=Verified web=0\n','')):self.assertFalse(verify_http_pixels(text,b'','Ready',BODY,self.contract)['ok'])
 def test_wrong_row_and_wrong_body_byte_rejected(self):
  text=self.trace();self.assertFalse(verify_http_pixels(text,self.framebuffer(text,shift=18),'Ready',BODY,self.contract)['ok']);self.assertFalse(verify_http_pixels(text,self.framebuffer(text,body=b'X'+BODY[1:]),'Ready',BODY,self.contract)['ok'])

if __name__=='__main__':unittest.main()
