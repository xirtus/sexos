"""Synthetic framebuffer tests of the real runner's source-bound pixel matcher."""
import sys
import tempfile
import shutil
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

class PublicationBinding(unittest.TestCase):
 def fixture(self):
  temporary=tempfile.TemporaryDirectory();self.addCleanup(temporary.cleanup);root=Path(temporary.name)
  for path in ['servers/sexdisplay/src/main.rs','apps/kaleidoscope/src/body_text.rs','apps/kaleidoscope/src/browser_view.rs','apps/kaleidoscope/src/main.rs','crates/sex-pdx/src/desktop_style.rs','servers/sexdisplay/src/surface_lifecycle.rs','apps/kaleidoscope/src/incarnation.rs','crates/sex-pdx/src/app_catalog.rs','crates/sex-pdx/src/app_lifecycle.rs']:
   dest=root/path;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(ROOT/path,dest)
  return root
 def mutate(self,root,path,old,new):
  dest=root/path;text=dest.read_text();self.assertIn(old,text);dest.write_text(text.replace(old,new,1))
 def test_current_held_sources_are_hashed_and_geometry_stays_native300(self):
  contract=load_pixel_contract(self.fixture())
  for path in ['apps/kaleidoscope/src/incarnation.rs','crates/sex-pdx/src/app_catalog.rs','crates/sex-pdx/src/app_lifecycle.rs']:
   self.assertIn(path,contract['source_sha256'])
  with self.assertRaises(ValueError):probe_status_geometry('[shell.tile.apply] sid=343 x=0 y=0 w=520 h=500\n[browser.preview.layout] bytes=105 newline=1 cached_or_sample=1','Verified',contract)
 def test_frozen_literal_root_needs_no_new_identity_files(self):
  root=self.fixture();(root/'apps/kaleidoscope/src/main.rs').write_text('browser_view::compose(self.preview.as_bytes()\nui:text_delivery::Outbox::atomic(300)')
  for path in ['apps/kaleidoscope/src/incarnation.rs','crates/sex-pdx/src/app_catalog.rs','crates/sex-pdx/src/app_lifecycle.rs']:(root/path).unlink()
  contract=load_pixel_contract(root);self.assertEqual(len(contract['source_sha256']),6)
 def test_archival_held_close_with_literal_output_remains_supported(self):
  root=self.fixture()
  self.mutate(root,'apps/kaleidoscope/src/main.rs','let surface=selected.surface();','')
  self.mutate(root,'apps/kaleidoscope/src/main.rs','ui:text_delivery::Outbox::atomic(surface)','ui:text_delivery::Outbox::atomic(300)')
  (root/'apps/kaleidoscope/src/incarnation.rs').unlink()
  self.assertEqual(len(load_pixel_contract(root)['source_sha256']),6)
 def test_missing_ambiguous_or_rewired_main_binding_rejects(self):
  changes=[
   ('let surface=selected.surface();','let surface=300;'),
   ('let selected=incarnation::select(identity).expect("BROWSER_EXACT_BOOT_IDENTITY");','let selected=fake_identity();'),
   ('catalog(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT)','catalog(0)'),
   ('Outbox::atomic(surface)','Outbox::atomic(343)'),
   ('Outbox::atomic(surface)','Outbox::atomic(300)'),
   ('let surface=selected.surface();','let surface=selected.surface();let surface=300;'),
   ('let surface=selected.surface();','let mut surface=selected.surface();surface=343;'),
   ('ui:text_delivery::Outbox::atomic(surface)','ui:text_delivery::Outbox::atomic(surface),other:text_delivery::Outbox::atomic(surface),ui:text_delivery::Outbox::atomic(surface)'),
  ]
  for old,new in changes:
   with self.subTest(new=new):
    root=self.fixture();self.mutate(root,'apps/kaleidoscope/src/main.rs',old,new)
    with self.assertRaises(ValueError):load_pixel_contract(root)
 def test_selector_or_native_catalog_rewiring_rejects(self):
  for path,old,new in [
   ('apps/kaleidoscope/src/incarnation.rs','surface: row.surface','surface: 300'),
   ('apps/kaleidoscope/src/incarnation.rs','if row.app != App::Browser','if row.app == App::Browser'),
   ('apps/kaleidoscope/src/incarnation.rs','{ self.surface }','{ 300 }'),
   ('crates/sex-pdx/src/app_catalog.rs','app: App::Browser, ordinal: 0, target: 15, surface: 300','app: App::Browser, ordinal: 0, target: 30, surface: 343'),
   ('crates/sex-pdx/src/app_lifecycle.rs','self.task_id!=0','self.task_id==0'),
  ]:
   with self.subTest(path=path,new=new):
    root=self.fixture();self.mutate(root,path,old,new)
    with self.assertRaises(ValueError):load_pixel_contract(root)

if __name__=='__main__':unittest.main()
