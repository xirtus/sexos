import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_spindle_relaunch_pixels import verify
from check_desktop_pixels import font_from_source,glyph
FONT=font_from_source((Path(__file__).resolve().parents[2]/'servers/sexdisplay/src/main.rs').read_text())
def fixture():
    width,height=1280,800;data=bytearray(width*height*3);x,y,w,h=40,60,640,400;cw,ch=(w-24)//2,(h-52)//2
    text=''
    for i,sid in enumerate([302,303,304,305]):
        row=(('spindle17'.ljust(20)+'>' if sid==304 else 'GRID'+str(sid))).ljust(120)
        text+=f'[spindle.grid.text] target=17 sid={sid} text={row}\n'
        sx,sy=x+8+(i%2)*(cw+8),y+36+(i//2)*(ch+8)
        text+=f'[shell.app.viewport] target=17 sid={sid} x={sx} y={sy} w={cw} h={ch}\n'
        for offset,char in enumerate(row):
            for gy,bits in enumerate(glyph(FONT,char)):
                for gx in range(5):
                    if bits&(1<<(4-gx)):
                        for dy in range(2):
                            for dx in range(2):
                                p=((sy+12+(offset//20)*18+gy*2+dy)*width+sx+8+(offset%20)*12+gx*2+dx)*3
                                data[p:p+3]=b'\xe8\xff\xff'
    return text,b'P6\n1280 800\n255\n'+data
class Check(unittest.TestCase):
    def test_synthetic_four_grid_pixels(self):text,ppm=fixture();self.assertEqual(verify(text,ppm,FONT),[])
    def test_blank_or_moved_frame_fails(self):
        text,ppm=fixture();self.assertTrue(verify(text,ppm[:ppm.index(b'255\n')+4]+bytes(1280*800*3),FONT));self.assertTrue(verify(text.replace('x=48','x=50'),ppm,FONT))
    def test_old_ids_missing_grid_and_bad_payload_fail(self):
        text,ppm=fixture()
        for bad in [text.replace('sid=302','sid=154'),text.replace('target=17','target=12'),text.replace('text=GRID303','text=GRID30')]:self.assertTrue(verify(bad,ppm,FONT))
if __name__=='__main__':unittest.main()
