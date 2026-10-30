"""Synthetic framebuffer regressions for the native DNS pixel oracle."""
import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from dns_query_gate import load_pixel_contract,dns_status_geometry,verify_dns_status
from check_desktop_pixels import glyph
ROOT=Path(__file__).resolve().parents[2]

class DnsPixels(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.contract=load_pixel_contract(ROOT)
 def trace(self,length=105,w=520,h=640,view='Unavailable'):
  return f'[shell.tile.apply] sid=300 x=600 y=100 w={w} h={h}\n[browser.preview.layout] bytes={length} newline=1 cached_or_sample=1\n[browser.net.job.view] state={view} web=0\n'
 def framebuffer(self,text,view='Answer',shift=0):
  geometry=dns_status_geometry(text,view,self.contract);pixels=bytearray(1280*800*3);scale=geometry['scale']
  for row in geometry['rows']:
   for column,char in enumerate(row['text']):
    for gy,bits in enumerate(glyph(self.contract['font'],char)):
     for gx in range(5):
      if not bits&(1<<(4-gx)):continue
      for dy in range(scale):
       for dx in range(scale):
        x=row['x']+(column*self.contract['advance']+gx)*scale+dx;y=row['y']+gy*scale+dy+shift
        pixels[(y*1280+x)*3:(y*1280+x)*3+3]=b'\xcd\xd6\xf4'
  return b'P6\n1280 800\n255\n'+pixels
 def corrupt(self,ppm,x,y):
  data=bytearray(ppm);offset=len(b'P6\n1280 800\n255\n')+(y*1280+x)*3;data[offset:offset+3]=b'\xcd\xd6\xf4';return bytes(data)
 def test_complete_answer_and_failure_glyphs(self):
  for view in ('Answer','Unavailable','Cancelled','Uncertain'):
   with self.subTest(view=view):
    text=self.trace();result=verify_dns_status(text,self.framebuffer(text,view),view,self.contract);self.assertTrue(result['ok'],result)
 def test_missing_address_or_wrong_status_rejected(self):
  text=self.trace();self.assertFalse(verify_dns_status(text,self.framebuffer(text,'Unavailable'),'Answer',self.contract)['ok'])
 def test_stale_suffix_and_footer_erasure_rejected(self):
  text=self.trace();ppm=self.framebuffer(text);g=dns_status_geometry(text,'Answer',self.contract)
  for row in (g['rows'][1],g['rows'][-1]):
   bad=self.corrupt(ppm,row['x']+(g['columns']-1)*self.contract['advance']*g['scale'],row['y'])
   self.assertFalse(verify_dns_status(text,bad,'Answer',self.contract)['ok'])
 def test_wrong_row_rejected(self):
  text=self.trace();self.assertFalse(verify_dns_status(text,self.framebuffer(text,shift=18),'Answer',self.contract)['ok'])
 def test_geometry_uses_dynamic_preview_width_and_tcp_status(self):
  geometries=[]
  for length,width,view in ((60,520,'Unavailable'),(125,520,'Unavailable'),(125,310,'Verified'),(125,250,'Cancelled')):
   text=self.trace(length,width,view=view);g=dns_status_geometry(text,'Answer',self.contract);geometries.append(g)
   self.assertEqual(g['dns_offset'],length+64)
   self.assertTrue(verify_dns_status(text,self.framebuffer(text),'Answer',self.contract)['ok'])
  self.assertNotEqual(geometries[0]['rows'][0]['row'],geometries[1]['rows'][0]['row'])
  self.assertNotEqual(geometries[1]['columns'],geometries[2]['columns'])
  self.assertEqual(geometries[-1]['scale'],1)
 def test_clipped_or_excess_extent_rejected(self):
  for text in (self.trace(h=180),self.trace(length=129)):
   self.assertFalse(verify_dns_status(text,self.framebuffer(self.trace()),'Answer',self.contract)['ok'])
 def test_blank_separator_row_is_checked(self):
  text=self.trace();g=dns_status_geometry(text,'Answer',self.contract);row=g['rows'][0]
  self.assertEqual(row['text'],'')
  self.assertFalse(verify_dns_status(text,self.corrupt(self.framebuffer(text),row['x'],row['y']),'Answer',self.contract)['ok'])
 def test_latest_tile_and_explicit_tcp_context_required(self):
  text=self.trace()+'[shell.tile.apply] sid=300 x=600 y=100 w=310 h=640\n'
  self.assertEqual(dns_status_geometry(text,'Answer',self.contract)['tile']['w'],310)
  self.assertTrue(verify_dns_status(text,self.framebuffer(text),'Answer',self.contract)['ok'])
  for missing in ('',self.trace().replace('[browser.net.job.view] state=Unavailable web=0\n','')):
   self.assertFalse(verify_dns_status(missing,b'','Answer',self.contract)['ok'])

if __name__=='__main__':unittest.main()
