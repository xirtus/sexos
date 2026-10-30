"""Synthetic framebuffer tests of the real runner's source-bound pixel matcher."""
import sys
import unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from outbound_job_gate import load_pixel_contract,probe_status_geometry,verify_probe_status
from check_desktop_pixels import glyph
ROOT=Path(__file__).resolve().parents[2]

class ProbePixels(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.contract=load_pixel_contract(ROOT)
 def trace(self,length=105,w=520,h=500):
  return f'[shell.tile.apply] sid=300 x=600 y=100 w={w} h={h}\n[browser.preview.layout] bytes={length} newline=1 cached_or_sample=1\n'
 def framebuffer(self,trace,view='Verified',shift=0):
  geometry=probe_status_geometry(trace,view,self.contract);width=1280;height=800
  pixels=bytearray(width*height*3);scale=geometry['scale']
  for row in geometry['rows']:
   for column,char in enumerate(row['text']):
    for gy,bits in enumerate(glyph(self.contract['font'],char)):
     for gx in range(5):
      if not bits&(1<<(4-gx)):continue
      for dy in range(scale):
       for dx in range(scale):
        x=row['x']+(column*6+gx)*scale+dx;y=row['y']+gy*scale+dy+shift
        pixels[(y*width+x)*3:(y*width+x)*3+3]=b'\xcd\xd6\xf4'
  return b'P6\n1280 800\n255\n'+pixels
 def test_verified_and_unavailable_full_regions(self):
  for view in ('Verified','Unavailable'):
   with self.subTest(view=view):
    text=self.trace();result=verify_probe_status(text,self.framebuffer(text,view),view,self.contract)
    self.assertTrue(result['ok'],result)
 def test_prefix_without_complete_status_cannot_pass(self):
  text=self.trace();ppm=self.framebuffer(text,'Unavailable')
  self.assertFalse(verify_probe_status(text,ppm,'Verified',self.contract)['ok'])
 def test_stale_suffix_is_rejected(self):
  text=self.trace();ppm=bytearray(self.framebuffer(text));g=probe_status_geometry(text,'Verified',self.contract)
  row=next(row for row in g['rows']if 'bytes verified'in row['text']);x=row['x']+(g['columns']-1)*6*g['scale'];y=row['y']
  offset=len(b'P6\n1280 800\n255\n')+(y*1280+x)*3;ppm[offset:offset+3]=b'\xcd\xd6\xf4'
  self.assertFalse(verify_probe_status(text,bytes(ppm),'Verified',self.contract)['ok'])
 def test_incorrect_row_is_rejected(self):
  text=self.trace()
  self.assertFalse(verify_probe_status(text,self.framebuffer(text,shift=18),'Verified',self.contract)['ok'])
 def test_dynamic_body_length_and_tiled_width_derive_wrapping(self):
  geometries=[]
  for length,width in ((60,520),(139,520),(139,310)):
   text=self.trace(length,width);g=probe_status_geometry(text,'Verified',self.contract);geometries.append(g)
   self.assertTrue(verify_probe_status(text,self.framebuffer(text),'Verified',self.contract)['ok'])
  self.assertNotEqual(geometries[0]['rows'][0]['row'],geometries[1]['rows'][0]['row'])
  self.assertNotEqual(geometries[1]['columns'],geometries[2]['columns'])
 def test_clipped_status_cannot_pass(self):
  text=self.trace(h=110)
  self.assertFalse(verify_probe_status(text,self.framebuffer(self.trace()),'Verified',self.contract)['ok'])
 def test_missing_geometry_or_preview_cannot_pass(self):
  for text in ('','[shell.tile.apply] sid=300 x=600 y=100 w=520 h=500'):
   self.assertFalse(verify_probe_status(text,b'','Verified',self.contract)['ok'])
 def test_blank_row_and_footer_erasure_are_checked(self):
  text=self.trace();ppm=bytearray(self.framebuffer(text));g=probe_status_geometry(text,'Verified',self.contract)
  row=g['rows'][0];offset=len(b'P6\n1280 800\n255\n')+(row['y']*1280+row['x'])*3
  ppm[offset:offset+3]=b'\xcd\xd6\xf4'
  self.assertFalse(verify_probe_status(text,bytes(ppm),'Verified',self.contract)['ok'])

if __name__=='__main__':unittest.main()
