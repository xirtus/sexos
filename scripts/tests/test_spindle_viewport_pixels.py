import sys,unittest,tempfile
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_spindle_viewport_pixels import verify,layout
from history_boot_fixture import boot
from check_desktop_pixels import font_from_source,glyph
FONT=font_from_source((Path(__file__).resolve().parents[2]/'servers/sexdisplay/src/main.rs').read_text())
def fixture(w=1280,h=134,target=12,task=None,caption=None):
    task=target if task is None else task
    width,height=1280,800;data=bytearray(width*height*3);x,y=0,50;panes,cols,rows=layout(w,h);sids={12:[154,160,161,162],17:[302,303,304,305],24:[313,315,316,317]}[target]
    text=boot({target:task})+f'[spindle.view.accepted] target={target} task={task} sequence=2 w={w} h={h} cols={cols} top_rows={rows[0]} bottom_rows={rows[1]} sender=3\n'
    prompt=b'> echo spindle17'.ljust(cols*2)
    for i,(sid,(px,py,pw,ph))in enumerate(zip(sids,panes)):
        r=rows[i//2];lines=[b' '*cols for _ in range(r)]
        if r and i//2==0 and caption is not None:lines[0]=caption.encode().ljust(cols*2)[(i%2)*cols:(i%2+1)*cols]
        if r and i//2==1:lines[-1]=prompt[(i%2)*cols:(i%2+1)*cols]
        payload=b''.join(line+b'\n'for line in lines);c=cols if r else 0
        text+=f'[shell.app.viewport] target={target} sid={sid} x={x+px} y={y+py} w={pw} h={ph}\n'
        text+=f'[spindle.view.text] target={target} sid={sid} sequence=2 paint={i+1} cols={c} rows={r} bytes={len(payload)}\n'
        for off in range(0,len(payload),48):text+=f'[spindle.view.bytes] target={target} sid={sid} seq=2 paint={i+1} off={off} hex={payload[off:off+48].hex()}\n'
        scale=2 if pw>=256 else 1
        for row,line in enumerate(lines):
            for col,char in enumerate(line):
                for gy,bits in enumerate(glyph(FONT,chr(char))):
                    for gx in range(5):
                        if bits&(1<<(4-gx)):
                            for dy in range(scale):
                                for dx in range(scale):
                                    xx=x+px+8+col*6*scale+gx*scale+dx;yy=y+py+12+row*9*scale+gy*scale+dy;p=(yy*width+xx)*3;data[p:p+3]=b'\xe8\xff\xff'
    return text,b'P6\n1280 800\n255\n'+data
class Check(unittest.TestCase):
    def test_compact_resolution_is_explicit_and_exact(self):
        text,ppm=fixture(1024,134)
        rgb=ppm.split(b'\n',3)[3]
        compact=b'P6\n1024 768\n255\n'+b''.join(rgb[y*1280*3:y*1280*3+1024*3]for y in range(768))
        self.assertEqual(verify(text,compact,FONT,12,'> echo spindle17',False,False,resolution=(1024,768)),[])
        self.assertTrue(verify(text,compact,FONT,12,'> echo spindle17',False,False))
        self.assertTrue(verify(text,compact,FONT,12,'> echo spindle17',False,False,resolution=(1280,800)))
    def test_small_tile_and_wide_resize_keep_real_prompt_ink(self):
        for w,h in [(1280,134),(640,134),(1280,670),(640,80)]:
            text,ppm=fixture(w,h);self.assertEqual(verify(text,ppm,FONT,12,'> echo spindle17',True,False),[])
    def test_blank_frame_is_not_readable_prompt_evidence(self):
        text,ppm=fixture();blank=ppm[:ppm.index(b'255\n')+4]+bytes(1280*800*3);self.assertTrue(verify(text,blank,FONT,12,'> echo spindle17',True,False))
    def test_stale_layout_wrong_owner_or_geometry_cannot_pass(self):
        text,ppm=fixture()
        for old,new in [('sid=161 sequence=2','sid=161 sequence=1'),('task=12 sequence=2','task=17 sequence=2'),('bottom_rows=1','bottom_rows=6'),('h=41','h=42')]:self.assertTrue(verify(text.replace(old,new),ppm,FONT,12,'> echo spindle17',True,False),(old,new))
    def test_empty_or_partial_hex_payload_is_not_a_paint(self):
        text,ppm=fixture();self.assertTrue(verify(text.replace('bytes=21','bytes=20'),ppm,FONT,12,None,False,False));self.assertTrue(verify(text.replace('hex=3e','hex=0'),ppm,FONT,12,None,False,False))
    def test_all_three_catalog_grids_and_unequal_held_task_have_exact_current_pixels(self):
        for target,task in [(12,12),(17,17),(24,24),(24,240)]:
            text,ppm=fixture(640,134,target,task,caption='NET / verified')
            self.assertEqual(verify(text,ppm,FONT,target,'> echo spindle17',False,False,require_text='NET / verified'),[])
        text,ppm=fixture(640,134,24,240,caption='NET / verified across panes')
        self.assertEqual(verify(text,ppm,FONT,24,None,False,False,require_text='NET / verified across panes'),[])

    def test_third_grid_wrong_task_layout_sequence_unknown_and_one_pixel_reject(self):
        text,ppm=fixture(640,134,24,240,caption='NET / verified')
        for changed in [text.replace('task=240 sequence=2','task=24 sequence=2'),
                        text.replace('sid=313','sid=302'),text.replace('sid=315 sequence=2','sid=315 sequence=1'),
                        text.replace('task=240 sequence=2','task=240 sequence=0')]:
            self.assertTrue(verify(changed,ppm,FONT,24,None,False,False))
        self.assertTrue(verify(text,ppm,FONT,25,None,False,False))
        header,rgb=ppm.split(b'255\n',1);changed=bytearray(rgb);at=changed.index(b'\xe8\xff\xff');changed[at]=0xe7
        self.assertTrue(verify(text,header+b'255\n'+changed,FONT,24,None,False,False))
        # An unused physical text column may not retain old foreground ink.
        changed=bytearray(rgb);offset=((50+36+12)*1280+(8+8+20*12))*3;changed[offset:offset+3]=b'\xe8\xff\xff'
        self.assertTrue(verify(text,header+b'255\n'+changed,FONT,24,None,False,False))

    def test_required_caption_comes_only_from_latest_current_pane_payload(self):
        old,_=fixture(640,134,24,240,caption='NET / verified')
        current,ppm=fixture(640,134,24,240,caption='NET / unavailable')
        old_paints=old[old.index('[spindle.view.accepted]'):]
        # Same-layout repaint with newer paint IDs replaces every pane.
        current_paints=current[current.index('[spindle.view.accepted]'):]
        for i in range(4,0,-1):current_paints=current_paints.replace(f'paint={i}',f'paint={i+4}')
        prefix=current[:current.index('[spindle.view.accepted]')]+old_paints+current_paints+'[old.event] NET / verified\n'
        self.assertTrue(verify(prefix,ppm,FONT,24,None,False,False,require_text='NET / verified'))
        self.assertEqual(verify(prefix,ppm,FONT,24,None,False,False,require_text='NET / unavailable'),[])
        for caption in ['', 'not visible']:
            self.assertTrue(verify(current,ppm,FONT,24,None,False,False,require_text=caption))

    def test_boot_provenance_and_literal_unique_grid_are_required(self):
        text,ppm=fixture(640,134,24,240,caption='NET / verified')
        self.assertTrue(verify(text[text.index('[spindle.view.accepted]'):],ppm,FONT,24,None,False,False))
        self.assertTrue(verify(text.replace('index=2 target=24 task=240','index=2 target=24 task=24'),ppm,FONT,24,None,False,False))
        root=Path(__file__).resolve().parents[2];source=(root/'crates/sex-pdx/src/app_catalog.rs').read_text()
        for children in ['315, 316, 316','315, 316','315, 316, 318']:
            with tempfile.TemporaryDirectory() as temporary:
                custom=Path(temporary);path=custom/'crates/sex-pdx/src/app_catalog.rs';path.parent.mkdir(parents=True)
                path.write_text(source.replace('children: &[315, 316, 317]','children: &['+children+']'))
                self.assertTrue(verify(text,ppm,FONT,24,None,False,False,root=custom))

if __name__=='__main__':unittest.main()
