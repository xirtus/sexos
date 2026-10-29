import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_spindle_viewport_pixels import verify,layout
from check_desktop_pixels import font_from_source,glyph
FONT=font_from_source((Path(__file__).resolve().parents[2]/'servers/sexdisplay/src/main.rs').read_text())
def fixture(w=1280,h=134,target=12):
    width,height=1280,800;data=bytearray(width*height*3);x,y=0,50;panes,cols,rows=layout(w,h);sids=[154,160,161,162]if target==12 else[302,303,304,305]
    text=f'[spindle.view.accepted] target={target} task={target} sequence=2 w={w} h={h} cols={cols} top_rows={rows[0]} bottom_rows={rows[1]} sender=3\n'
    prompt=b'> echo spindle17'.ljust(cols*2)
    for i,(sid,(px,py,pw,ph))in enumerate(zip(sids,panes)):
        r=rows[i//2];lines=[b' '*cols for _ in range(r)]
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
if __name__=='__main__':unittest.main()
