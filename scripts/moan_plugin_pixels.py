"""Visible Weft model + fresh status-strip scanout, separate from DSP telemetry."""
import re
from check_disk_persistence import ppm_rgb
PAINT=re.compile(r'^\[moanrack.paint\] target=(\d+) task=(\d+) surface=(\d+) frame=(\d+) width=(\d+) height=(\d+) x=(\d+) y=(\d+) w=(\d+) h=(\d+) hash=(0x[0-9a-f]{16}) text_hex=([0-9a-f]*)\r?$',re.M)
def expected_status(mode):
    if mode==4:return 'VST3 gain 0.25 verified'
    if mode in (0,6):return 'CLAP stereo swap verified'
    if mode in (1,2,5):return 'Child failed or cancelled; parent remains alive'
    raise ValueError('unknown qualification mode')
def model_view(trace,parent,task,surface,viewport,expected,after=0):
    candidates=[m for m in PAINT.finditer(trace,after)if tuple(map(int,m.groups()[:3]))==(parent,task,surface)]
    if not candidates:raise ValueError('missing actual visible-node paint receipt')
    m=candidates[-1];frame,width,height,x,y,w,h=map(int,m.groups()[3:10]);text=bytes.fromhex(m[12]).decode('utf8')
    if text!=expected:raise ValueError('actual visible label differs from expected status')
    if frame<1 or (width,height)!=tuple(viewport):raise ValueError('stale/wrong viewport paint')
    if w<1 or h<1 or x+w>width or y+h>height:raise ValueError('visible label outside client')
    return dict(frame=frame,width=width,height=height,x=x,y=y,w=w,h=h,hash=int(m[11],16),text=text,offset=m.start())
def visible_pixels(ppm,client,view):
    width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1);ox,oy,cw,ch=client
    if (cw,ch)!=(view['width'],view['height']):raise ValueError('client differs from actual paint')
    x,y=ox+view['x'],oy+view['y'];w,h=view['w'],view['h']
    if min(x,y)<0 or x+w>width or y+h>height:raise ValueError('label outside screenshot')
    value=0xcbf29ce484222325;colours=set()
    for row in range(y,y+h):
        data=pixels[(row*width+x)*3:(row*width+x+w)*3]
        colours.update(data[i:i+3]for i in range(0,len(data),3))
        for byte in data:value=((value^byte)*0x100000001b3)&((1<<64)-1)
    if len(colours)<=2:raise ValueError('visible status strip has no qualified text ink')
    if value!=view['hash']:raise ValueError('fresh visible status pixels differ from actual Weft paint')
    return dict(ok=True,checked_pixels=w*h,colours=len(colours),text=view['text'],frame=view['frame'],hash=f'{value:016x}')
