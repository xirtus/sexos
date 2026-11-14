#!/usr/bin/env python3
"""Lurk, the VNC viewer, shows a host VNC server's screen and sends it keys.

1. A pure-stdlib RFB 3.8 server on this host serves a 320x240 test pattern: four
   coloured quadrants (red, green, blue, yellow) and a white band reading LURK. The
   first update sends the top half Raw and the bottom half ZRLE (zlib from the
   standard library, one stream for the connection, sync-flushed per rectangle).
2. `run +net lurk 10.0.2.2:PORT` in SexOS (QEMU's user network shows the host as
   10.0.2.2): the gate waits for `[lurk.connected] name=lurk-gate size=320x240` and
   `[lurk.frame] updates=1`.
3. A QMP screendump must hold all four colours, laid out as quadrants (red left of
   green, red above blue, yellow right of blue and below green).
4. `a` typed into Lurk's window reaches the server as KeyEvent down keysym 0x61.

Only None security is used here: the standard library has no DES. VNC Authentication
is covered byte-exactly by `cargo test -p sex-rfb` (OpenSSL golden)."""
import argparse,os,shutil,socket,struct,threading,time,zlib
from pathlib import Path
from install_program import install
ROOT=Path(__file__).resolve().parents[1]
W,H=320,240
RED,GREEN,BLUE,YELLOW,WHITE,BLACK=0xFF0000,0x00FF00,0x0000FF,0xFFFF00,0xFFFFFF,0x000000
# 5x7 letters for the band.
FONT={'L':['10000']*6+['11111'],'U':['10001']*6+['01110'],'R':['11110','10001','10001','11110','10100','10010','10001'],'K':['10001','10010','10100','11000','10100','10010','10001']}

def pattern():
 px=[0]*(W*H)
 for y in range(H):
  for x in range(W):px[y*W+x]=(RED if x<W//2 else GREEN) if y<H//2 else (BLUE if x<W//2 else YELLOW)
 # The band is off the quadrant boundary's middle row so the quadrants stay large.
 by,bh=100,40
 for y in range(by,by+bh):
  for x in range(60,260):px[y*W+x]=WHITE
 for i,ch in enumerate('LURK'):
  for r,row in enumerate(FONT[ch]):
   for c,bit in enumerate(row):
    if bit=='1':
     for dy in range(4):
      for dx in range(4):px[(by+6+r*4+dy)*W+80+i*44+c*4+dx]=BLACK
 return px

def raw(px,x,y,w,h):
 return b''.join(struct.pack('<I',px[r*W+c]) for r in range(y,y+h) for c in range(x,x+w))

def zrle_tiles(px,x,y,w,h):
 out=bytearray()
 for ty in range(y,y+h,64):
  for tx in range(x,x+w,64):
   tw,th=min(64,x+w-tx),min(64,y+h-ty)
   cells=[px[r*W+c] for r in range(ty,ty+th) for c in range(tx,tx+tw)]
   if len(set(cells))==1:out+=b'\x01'+struct.pack('<I',cells[0])[:3]
   else:out+=b'\x00'+b''.join(struct.pack('<I',p)[:3] for p in cells)
 return bytes(out)

class Server:
 """One client at a time: handshake (None), ServerInit, then the pattern on the first
 full request; incremental requests are held (nothing changes). Records KeyEvents."""
 def __init__(self):
  self.sock=socket.socket();self.sock.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);self.sock.bind(('127.0.0.1',0));self.sock.listen(4)
  self.port=self.sock.getsockname()[1];self.px=pattern();self.keys=[];self.encodings=[];self.format=None;self.sent=0;self.error=None
  threading.Thread(target=self.serve,daemon=True).start()
 def serve(self):
  while True:
   try:
    c,_=self.sock.accept()
   except OSError:return
   try:self.session(c)
   except Exception as e:self.error=repr(e)
   finally:c.close()
 def session(self,c):
  def need(n):
   b=b''
   while len(b)<n:
    d=c.recv(n-len(b))
    if not d:raise EOFError
    b+=d
   return b
  c.sendall(b'RFB 003.008\n');v=need(12);assert v==b'RFB 003.008\n',v
  c.sendall(b'\x01\x01');assert need(1)==b'\x01'
  c.sendall(struct.pack('>I',0));need(1)
  name=b'lurk-gate'
  fmt=bytes([32,24,0,1,0,255,0,255,0,255,16,8,0,0,0,0])
  c.sendall(struct.pack('>HH',W,H)+fmt+struct.pack('>I',len(name))+name)
  z=zlib.compressobj(6)
  while True:
   t=need(1)[0]
   if t==0:need(3);self.format=need(16)
   elif t==2:need(1);n=struct.unpack('>H',need(2))[0];self.encodings=list(struct.unpack(f'>{n}i',need(4*n)))
   elif t==3:
    inc,x,y,w,h=struct.unpack('>BHHHH',need(9))
    if inc:continue
    assert self.format==fmt,('client must ask for RGB32 first',self.format)
    use_zrle=16 in self.encodings
    top=struct.pack('>HHHHi',0,0,W,H//2,0)+raw(self.px,0,0,W,H//2)
    if use_zrle:
     data=z.compress(zrle_tiles(self.px,0,H//2,W,H-H//2))+z.flush(zlib.Z_SYNC_FLUSH)
     bottom=struct.pack('>HHHHi',0,H//2,W,H-H//2,16)+struct.pack('>I',len(data))+data
    else:bottom=struct.pack('>HHHHi',0,H//2,W,H-H//2,0)+raw(self.px,0,H//2,W,H-H//2)
    c.sendall(b'\x00\x00'+struct.pack('>H',2)+top+bottom);self.sent+=1
   elif t==4:down,_,sym=struct.unpack('>BHI',need(7));self.keys.append((down,sym))
   elif t==5:need(5)
   elif t==6:need(3);need(struct.unpack('>I',need(4))[0])
   else:raise ValueError(f'client message {t}')

def read_ppm(path):
 data=Path(path).read_bytes();fields=[];i=0
 while len(fields)<4:
  while data[i:i+1].isspace():i+=1
  if data[i:i+1]==b'#':i=data.index(b'\n',i);continue
  j=i
  while not data[j:j+1].isspace():j+=1
  fields.append(data[i:j]);i=j
 assert fields[0]==b'P6' and fields[3]==b'255',fields[:4]
 w,h=int(fields[1]),int(fields[2]);return w,h,data[i+1:i+1+w*h*3]

def quadrants(path):
 """Pixel count and centre of each test colour on the screen."""
 w,h,rgb=read_ppm(path);want={RED:'red',GREEN:'green',BLUE:'blue',YELLOW:'yellow'};acc={n:[0,0,0] for n in want.values()}
 for i in range(0,len(rgb),3):
  n=want.get(rgb[i]<<16|rgb[i+1]<<8|rgb[i+2])
  if n:p=i//3;a=acc[n];a[0]+=1;a[1]+=p%w;a[2]+=p//w
 return {n:(a[0],a[1]/max(a[0],1),a[2]/max(a[0],1)) for n,a in acc.items()}

def check_quadrants(q):
 small=[n for n,(k,_,_) in q.items() if k<3000]
 if small:return f'too few pixels of {small}: {q}'
 r,g,b,y=(q[n][1:] for n in('red','green','blue','yellow'))
 if not(r[0]<g[0] and r[1]<b[1] and b[0]<y[0] and g[1]<y[1]):return f'colours are not laid out as quadrants: {q}'
 return None

SERVER=Server()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','.':'dot','-':'minus','/':'slash'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 elf=(ROOT/'iso_root/programs/lurk').read_bytes();assert elf[:4]==b'\x7fELF'
 install(disk,[('lurk',elf)])
 g=boot(a.iso,disk,a.out,'lurk',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   elif k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.05)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  before=a.out/'before.ppm';q.cmd('screendump',{'filename':str(before)});time.sleep(.5)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net lurk 10.0.2.2:{SERVER.port}');q.key('ret')
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,120)
  c=g['wait'](r'\[lurk.(connected\] [^\n]*|failed\] [^\n]*)\n',s,180)[0]
  assert c.startswith('[lurk.connected] name=lurk-gate size=320x240 version=3.8 security=1'),(c,SERVER.error)
  f=g['wait'](r'\[lurk.(frame\] updates=1 [^\n]*|failed\] [^\n]*)\n',s,180)[0]
  assert f.startswith('[lurk.frame] updates=1 rects=2 area=0,0,320x240'),(f,SERVER.error)
  assert SERVER.sent>=1 and 16 in SERVER.encodings,(SERVER.sent,SERVER.encodings)
  # Display may take a few frames under TCG: look until the pattern is there.
  shot=a.out/'lurk.ppm';problem='no screenshot';deadline=time.monotonic()+60
  while problem and time.monotonic()<deadline:
   time.sleep(2);q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
   problem=check_quadrants(quadrants(shot))
  assert not problem,problem
  # The colours came from Lurk's window, not from the desktop.
  assert check_quadrants(quadrants(before)),'test colours were on screen before Lurk ran'
  g['focus'](int(m[3]));time.sleep(.5);k=len(t());q.key('a')
  g['wait'](r'\[lurk.key\] keysym=0x61 control=0',k,60)
  deadline=time.monotonic()+30
  while (1,0x61) not in SERVER.keys and time.monotonic()<deadline:time.sleep(.2)
  assert (1,0x61) in SERVER.keys and (0,0x61) in SERVER.keys,SERVER.keys
  q.key('f11');g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',k,120)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[lurk.fatal]' not in t()
  print({'ok':True,'connected':c.strip(),'frame':f.strip(),'quadrants':quadrants(shot),'keys':SERVER.keys})
 finally:
  g['stop']()
if __name__=='__main__':main()
