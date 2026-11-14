#!/usr/bin/env python3
"""A picture is shown: View, a program, decodes a PNG, a JPEG and a WebP it was given, plays an animated GIF and an animated WebP frame by frame, and Display composites their pixels (the PNG exactly); a picture larger than the window is shrunk to fit; a file that is not a picture is refused in words.

Looking at it: R turns the PNG a quarter clockwise (every sampled pixel where a
clockwise turn puts it), Shift+R turns it back, + zooms one step and 0 fits again,
and a zoomed picture pans. Given a folder, View steps through its pictures with N and
P, and Space runs a timed slideshow that moves on by itself."""
import argparse,http.server,io,os,re,shutil,threading,time
from pathlib import Path
from PIL import Image
from browser_gate_viewport import full_viewport
from web_surface_geometry import latest_frame,insets
ROOT=Path(__file__).resolve().parents[1]
# The window frame around the pixel area: left (and right, bottom) and top, in pixels.
L,T=insets(ROOT)
def png(w,h):
 img=Image.new('RGB',(w,h))
 img.putdata([((x*255)//w,(y*255)//h,(x^y)&255) for y in range(h) for x in range(w)])
 b=io.BytesIO();img.save(b,'PNG');return img,b.getvalue()
PIC,PIC_BYTES=png(300,200)
_,BIG_BYTES=png(900,700)
def jpeg(w,h):
 img=Image.new('RGB',(w,h))
 img.putdata([((x*255)//w,(y*255)//h,96) for y in range(h) for x in range(w)])
 b=io.BytesIO();img.save(b,'JPEG',quality=95,subsampling=0);return img,b.getvalue()
JPG,JPG_BYTES=jpeg(240,160)
def webp(img):
 b=io.BytesIO();img.save(b,'WEBP',lossless=True,exact=True);return b.getvalue()
WEBP_BYTES=webp(PIC)
ANIM_COLOURS=[(220,40,40),(40,200,60),(40,80,230)]
def anim():
 frames=[Image.new('RGB',(120,80),c) for c in ANIM_COLOURS]
 b=io.BytesIO();frames[0].save(b,'GIF',save_all=True,append_images=frames[1:],duration=400,loop=0);return b.getvalue()
GIF_BYTES=anim()
def anim_webp():
 frames=[Image.new('RGB',(120,80),c) for c in ANIM_COLOURS]
 b=io.BytesIO();frames[0].save(b,'WEBP',save_all=True,append_images=frames[1:],duration=300,loop=0,lossless=True);return b.getvalue()
WEBP_ANIM_BYTES=anim_webp()
FILES={'/view':(ROOT/'iso_root/programs/view').read_bytes(),'/pic.png':PIC_BYTES,'/big.png':BIG_BYTES,'/pic.jpg':JPG_BYTES,'/pic.webp':WEBP_BYTES,'/anim.gif':GIF_BYTES,'/anim.webp':WEBP_ANIM_BYTES,'/note.txt':b'not a picture\n'}
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  body=FILES.get(self.path)
  if body is None:self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'view',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k=='@':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'2'}],'hold-time':80})
   elif k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def expect(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def opened(name,w,h):
  # The desktop may tile a new window smaller than the picture: zoom it through Shell's
  # palette first, then View shows the picture whole at its new size.
  s=len(t());expect(f'run view @{name}',r'\[view.ready\]')
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60);time.sleep(1)
  full_viewport(g,ROOT,int(m[3]),w,h)
  g['wait'](rf'\[view.shown\] w={w} h={h} bytes=\d+ scaled=0',s,120)
  return m,s
 def fetched(url,name):
  m=expect(f'fetch http://10.0.2.2:{PORT}{url} {name}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',300);assert m[1].startswith('Saved'),m[1]
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  fetched('/view','/bin/view');fetched('/pic.png','pic.png');fetched('/big.png','big.png');fetched('/pic.jpg','pic.jpg');fetched('/pic.webp','pic.webp');fetched('/anim.gif','anim.gif');fetched('/anim.webp','anim.webp');fetched('/note.txt','note.txt')
  m,s=opened('pic.png',300,200);g['wait'](r'\[display.app.pixels\] target=\d+ surface=\d+ w=300 h=200',s,60);time.sleep(3)
  shot=a.out/'view.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  x,y,w,h=latest_frame(t(),int(m[3]))
  img=Image.open(shot).convert('RGB');bx,by=x+L,y+T
  checked=bad=0
  for py in range(0,min(200,h-T-L),7):
   for px in range(0,min(300,w-2*L),11):
    if 636<=bx+px<660 and 398<=by+py<432:continue
    checked+=1;bad+=img.getpixel((bx+px,by+py))!=PIC.getpixel((px,py))
  assert checked>100 and bad==0,f'{bad} of {checked} pixels differ'
  def close(m):
   s2=len(t());g['focus'](int(m[3]));time.sleep(.3);q.key('f11');g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s2,90)
  def press(k,pattern,shift=False):
   s2=len(t())
   if shift:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':k}],'hold-time':80})
   else:q.key(k)
   return g['wait'](pattern,s2,60)
  # A quarter turn clockwise: 200 wide, 300 high, each pixel where the turn puts it.
  g['focus'](int(m[3]));time.sleep(.5)
  press('r',r'\[view.rotated\] deg=90\n');g['wait'](r'\[view.drawn\] w=200 h=300 deg=90 pct=0\n',s,60);time.sleep(2)
  shot=a.out/'view-rotated.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  img=Image.open(shot).convert('RGB');turned=PIC.transpose(Image.Transpose.ROTATE_270)
  rchecked=rbad=0
  for py in range(0,min(300,h-T-L),7):
   for px in range(0,min(200,w-2*L),11):
    if 636<=bx+px<660 and 398<=by+py<432:continue
    rchecked+=1;rbad+=img.getpixel((bx+px,by+py))!=turned.getpixel((px,py))
  assert rchecked>100 and rbad==0,f'turned: {rbad} of {rchecked} pixels differ'
  # Shift+R turns it back: the plain picture again, as first shown.
  s=len(t());press('r',r'\[view.rotated\] deg=0\n',shift=True);g['wait'](r'\[view.shown\] w=300 h=200 ',s,60)
  # One zoom step up from 100 %, then 0 fits again.
  s=len(t());press('equal',r'\[view.zoom\] pct=125\n');g['wait'](r'\[view.drawn\] w=375 h=250 deg=0 pct=125\n',s,60)
  s=len(t());press('0',r'\[view.zoom\] pct=0\n');g['wait'](r'\[view.shown\] w=300 h=200 ',s,60)
  close(m)
  # JPEG: lossy, so each sampled pixel within a small distance of the source.
  m,s=opened('pic.jpg',240,160);g['wait'](r'\[display.app.pixels\] target=\d+ surface=\d+ w=240 h=160',s,60);time.sleep(3)
  shot=a.out/'view-jpeg.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  x,y,w,h=latest_frame(t(),int(m[3]))
  img=Image.open(shot).convert('RGB');bx,by=x+L,y+T
  jchecked=worst=0
  for py in range(0,min(160,h-T-L),7):
   for px in range(0,min(240,w-2*L),11):
    if 636<=bx+px<660 and 398<=by+py<432:continue
    jchecked+=1;worst=max(worst,max(abs(c-d) for c,d in zip(img.getpixel((bx+px,by+py)),JPG.getpixel((px,py)))))
  assert jchecked>100 and worst<=8,f'JPEG pixels off by up to {worst} over {jchecked}'
  close(m)
  # 900x700 is shrunk to fit the window's pixel area, and fitted again when Shell
  # gives the window its place: the last picture fits the last size Shell reported.
  s=len(t());expect('run view @big.png',r'\[view.shown\] w=\d+ h=\d+ bytes=\d+ scaled=1')
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60);time.sleep(4)
  resized=re.findall(rf'\[shell.program.resize\] pd={m[1]} task={m[2]} w=(\d+) h=(\d+) status=0',t()[s:])
  assert resized,'View was never told its window size'
  aw,ah=map(int,resized[-1])
  bw,bh,fw,fh=map(int,re.findall(r'\[view.shown\] w=(\d+) h=(\d+) bytes=\d+ scaled=1 fit=(\d+)x(\d+)',t()[s:])[-1])
  tw,th=latest_frame(t(),int(m[3]))[2:]
  assert (fw,fh)==(aw,ah)==(tw-16,th-T-L),f'fit {fw}x{fh}, resize {aw}x{ah}, tile {tw}x{th}'
  assert bw<=aw and bh<=ah and (bw==aw or bh==ah),f'{bw}x{bh} does not fit {aw}x{ah}'
  big=(bw,bh,aw,ah)
  # At 100 % it is larger than the window: the arrows pan across it.
  g['focus'](int(m[3]));time.sleep(.5)
  s=len(t());press('1',r'\[view.zoom\] pct=100\n');d=g['wait'](r'\[view.drawn\] w=(\d+) h=(\d+) deg=0 pct=100\n',s,60)
  assert (int(d[1]),int(d[2]))==(min(900,aw),min(700,ah)),d[0]
  s=len(t());press('right',r'\[view.drawn\] w=\d+ h=\d+ deg=0 pct=100\n')
  close(m)
  # Lossless WebP: exact, like the PNG.
  m,s=opened('pic.webp',300,200);g['wait'](r'\[display.app.pixels\] target=\d+ surface=\d+ w=300 h=200',s,60);time.sleep(3)
  shot=a.out/'view-webp.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  x,y,w,h=latest_frame(t(),int(m[3]))
  img=Image.open(shot).convert('RGB');bx,by=x+L,y+T
  wchecked=wbad=0
  for py in range(0,min(200,h-T-L),7):
   for px in range(0,min(300,w-2*L),11):
    if 636<=bx+px<660 and 398<=by+py<432:continue
    wchecked+=1;wbad+=img.getpixel((bx+px,by+py))!=PIC.getpixel((px,py))
  assert wchecked>100 and wbad==0,f'WebP: {wbad} of {wchecked} pixels differ'
  close(m)
  # Animated GIF: three frames, 400 ms each, looping forever; each colour reaches the screen.
  s=len(t());expect('run view @anim.gif',r'\[view.animated\] frames=3 loops=-1')
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['wait'](r'\[view.frame\] n=4 delay=400',s,60)
  x,y,w,h=latest_frame(t(),int(m[3]))
  seen=set()
  for i in range(12):
   shot=a.out/f'view-gif-{i}.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.25)
   c=Image.open(shot).convert('RGB').getpixel((x+L+60,y+T+40))
   for k,want in enumerate(ANIM_COLOURS):
    if max(abs(p-q2) for p,q2 in zip(c,want))<=2:seen.add(k)
  assert seen=={0,1,2},f'animation colours seen: {sorted(seen)}'
  close(m)
  # Animated WebP: three frames, 300 ms each, looping; the fourth frame is the first again.
  s=len(t());expect('run view @anim.webp',r'\[view.animated\] frames=3 loops=-1')
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  for n in range(1,5):g['wait'](rf'\[view.frame\] n={n} delay=300',s,60)
  close(m)
  # A folder: its pictures in name order, N and P, then a slideshow moving by itself.
  expect('mkdir slides',r'\[spindle.fs.result\] kind=\d+ status=0')
  fetched('/pic.png','slides/a.png');fetched('/pic.webp','slides/b.webp');fetched('/anim.gif','slides/c.gif');fetched('/note.txt','slides/d.txt')
  s=len(t());expect('run view @slides',r'\[view.slide\] index=1 of=3 name=a.png\n')
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60);time.sleep(1)
  g['focus'](int(m[3]));time.sleep(.5)
  press('n',r'\[view.slide\] index=2 of=3 name=b.webp\n')
  press('p',r'\[view.slide\] index=1 of=3 name=a.png\n')
  press('p',r'\[view.slide\] index=3 of=3 name=c.gif\n')
  s=len(t());press('spc',r'\[view.slideshow\] on=1 ms=2500\n')
  g['wait'](r'\[view.slide\] index=1 of=3 name=a.png\n',s,60);g['wait'](r'\[view.slide\] index=2 of=3 name=b.webp\n',s,60)
  press('spc',r'\[view.slideshow\] on=0 ms=2500\n')
  close(m)
  expect('run view @note.txt',r'\[view.failed\] That file is not a PNG, JPEG, WebP or GIF picture.')
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'big':big,'pixels_checked':checked,'webp_pixels_checked':wchecked,'gif_colours':len(seen),'jpeg_pixels_checked':jchecked,'jpeg_worst':worst})
 finally:g['stop']()
if __name__=='__main__':main()
