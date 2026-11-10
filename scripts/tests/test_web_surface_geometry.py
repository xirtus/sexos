from pathlib import Path
import sys,tempfile,unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from web_surface_geometry import canvas_box,latest_paint,latest_frame

class SurfaceGeometry(unittest.TestCase):
    def root(self,temporary,top):
        root=Path(temporary)
        directory=root/'crates/sex-pdx/src';directory.mkdir(parents=True)
        (directory/'desktop_style.rs').write_text('pub const INSET:u32=8; pub const TITLE:u32=26; pub const RESIZE:u32=8;')
        (directory/'pixel_region.rs').write_text('pub const LEFT:u32=crate::desktop_style::INSET; pub const TOP:u32='+top+';')
        return root
    def test_actual_legacy_and_slim_source_contracts_have_distinct_literal_origins(self):
        with tempfile.TemporaryDirectory()as d:
            root=self.root(d,'28 + 8')
            self.assertEqual(canvas_box(root,(10,50,640,400),624,356),(18,86,624,356))
        with tempfile.TemporaryDirectory()as d:
            root=self.root(d,'crate::desktop_style::chrome::TITLE+crate::desktop_style::chrome::RESIZE')
            self.assertEqual(canvas_box(root,(10,50,640,400),624,358),(18,84,624,358))
            self.assertEqual(canvas_box(root,(10,50,1280,800),1248,716),(26,118,1248,716))
            with self.assertRaises(AssertionError):canvas_box(root,(10,50,640,400),624,356)
            with self.assertRaises(AssertionError):canvas_box(root,(10,50,1280,800),410,330)
    def test_latest_geometry_tracks_maximize_restore_and_ignores_other_surfaces(self):
        tiled='[shell.tile.apply] sid=77 x=10 y=50 w=640 h=400\n'
        zoom='[silk.lifecycle.zoom.active] frame=2 sid=77 zx=0 zy=50 zw=1280 zh=750\n'
        restore='[silk.lifecycle.zoom.restore] frame=2 sid=77 nx=10 ny=50 nw=640 nh=400\n'
        self.assertEqual(latest_frame(tiled+zoom,77),(0,50,1280,750))
        self.assertEqual(latest_frame(tiled+zoom+restore,77),(10,50,640,400))
        self.assertEqual(latest_frame(tiled+zoom+'[shell.tile.apply] sid=78 x=0 y=0 w=1 h=1\n',77),(0,50,1280,750))
        with self.assertRaises(ValueError):latest_frame(tiled.rstrip(),77)
    def test_only_complete_actual_paints_select_reference_size(self):
        first='[web.shown] w=624 h=358 scroll=0 focus=-1 items=3 ticks=9\n'
        partial='[web.shown] w=1264 h=706 scroll=0 focus=-1 items=4 ticks=10'
        self.assertEqual(latest_paint(first+partial),(624,358))
        with self.assertRaises(ValueError):latest_paint(partial)
        self.assertEqual(latest_paint(first+partial+' canvases=1\n'),(1264,706))

if __name__=='__main__':unittest.main()
