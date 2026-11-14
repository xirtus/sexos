#!/usr/bin/env python3
"""Files: thumbnails. A USB stick holds 50 pictures (PNG, 640x480, each its own colour)
plus a PDF and a text file. Files, given that stick, shows it as a grid (Ctrl+G): every
picture gets a thumbnail within the budget (THUMBS_BUDGET_MS, default 60 s on QEMU's
software CPU), the PDF and text file get kind tiles, and the screen shows the first
pictures' colours. A second Files run opens the same folder from the thumbnail cache:
nothing is decoded again, and it is faster."""
import argparse,os,re,shutil,struct,subprocess,time,zlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
MT={**os.environ,'MTOOLS_SKIP_CHECK':'1'}
N=50
def colour(i):return ((i*67+40)%256,(i*151+90)%256,(i*29+170)%256)
def png(w,h,rgb):
 row=b'\0'+bytes(rgb)*w
 # A darker band across the middle, so it is a picture and not one flat colour.
 band=b'\0'+bytes(c//2 for c in rgb)*w
 raw=b''.join(band if h//3<=y<2*h//3 else row for y in range(h))
 def chunk(t,d):return struct.pack('>I',len(d))+t+d+struct.pack('>I',zlib.crc32(t+d))
 return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',w,h,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(raw,6))+chunk(b'IEND',b'')
def make_stick(path,scratch):
 subprocess.run(['truncate','-s','64M',str(path)],check=True)
 subprocess.run(['mkfs.vfat','-F','32','-n','THUMBS',str(path)],check=True,capture_output=True)
 names=[]
 for i in range(N):
  n=f'IMG{i:02d}.PNG';(scratch/n).write_bytes(png(640,480,colour(i)));names.append(n)
 (scratch/'PAPER.PDF').write_bytes(b'%PDF-1.4\n%%EOF\n');(scratch/'NOTES.TXT').write_bytes(b'notes\n');names+=['PAPER.PDF','NOTES.TXT']
 subprocess.run(['mcopy','-i',str(path),*[str(scratch/n) for n in names],'::'],check=True,env=MT)
def ppm(path):
 data=path.read_bytes();parts=data.split(maxsplit=4);w,h=int(parts[1]),int(parts[2]);return w,h,parts[4]
def seen(path,colours):
 w,h,px=ppm(path);count={}
 for i in range(0,len(px)-2,3):
  c=(px[i],px[i+1],px[i+2])
  if c in colours:count[c]=count.get(c,0)+1
 return {c:n for c,n in count.items() if n>=300}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 budget=int(os.environ.get('THUMBS_BUDGET_MS','60000'))
 shutil.rmtree(a.out,ignore_errors=True);a.out.mkdir(parents=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 scratch=a.out/'host';scratch.mkdir();stick=a.out/'thumbs.img';make_stick(stick,scratch)
 import http.server,threading
 FILES={'/files':(ROOT/'iso_root/programs/loom').read_bytes()}
 class Handler(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   body=FILES.get(self.path)
   if body is None:self.send_response(404);self.end_headers();return
   self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def log_message(self,*x):pass
 class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
 server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1];threading.Thread(target=server.serve_forever,daemon=True).start()
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 os.environ.pop('SEXOS_GATE_DEVICES',None);os.environ.setdefault('SEXOS_GATE_BOUND','900')
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,a.out,'thumbs',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   shift=k.isupper() or k in ':_'
   code={':':'semicolon','_':'minus'}.get(k,KEYS.get(k,k.lower()))
   if shift:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':code}],'hold-time':80})
   else:q.key(code)
   time.sleep(.06)
 def line(text):g['focus'](153);time.sleep(.5);keys(text);q.key('ret')
 def expect(text,pattern,timeout=60):s=len(t());line(text);return g['wait'](pattern,s,timeout)
 def ctrl(k):q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':k}],'hold-time':80})
 def open_files(folder,count):
  s=len(t());line(f'files {folder}');m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['wait'](rf'\[files.app.list\] dir={folder} count={count}\b',s,60);g['wait'](rf'\[display.text.committed\] target={m[1]} ',s,60);time.sleep(1)
  g['focus'](int(m[3]));time.sleep(.5);return int(m[1]),int(m[3])
 colours={colour(i):i for i in range(N)}
 result={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  m=expect(f'fetch http://10.0.2.2:{PORT}/files /bin/loom',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  q.cmd('blockdev-add',{'driver':'raw','node-name':'thumbsdrive','file':{'driver':'file','filename':str(stick)}})
  s=len(t())
  for _ in range(40):
   try:q.cmd('device_add',{'driver':'usb-storage','bus':'xhci.0','drive':'thumbsdrive','id':'thumbs1'});break
   except RuntimeError:time.sleep(.5)
  g['wait'](r'\[files\.media\.mounted\] name=THUMBS format=FAT32 [^\n]*source=usb',s,120)
  for run in ('first','again'):
   target,surface=open_files('/media/THUMBS',N+2)
   s=len(t());ctrl('g')
   g['wait'](r'\[files.app.grid\] on=1 dir=/media/THUMBS',s,30)
   m=g['wait'](r'\[files.app.thumbs\] dir=/media/THUMBS pictures=(\d+) made=(\d+) cached=(\d+) failed=(\d+) ms=(\d+)',s,budget/1000+120)
   pics,made,cached,failed,ms=map(int,m.groups())
   result[run]={'made':made,'cached':cached,'failed':failed,'ms':ms}
   assert pics==N and failed==0,m[0]
   if run=='first':assert made==N and ms<=budget,m[0]
   else:assert cached==N and made==0 and ms<result['first']['ms'],m[0]
   g['wait'](rf'\[display.app.pixels\] target={target} surface={surface} w=\d+ h=\d+',s,30)
   time.sleep(2);shot=a.out/f'thumbs-{run}.ppm';q.cmd('screendump',{'filename':str(shot)})
   found=seen(shot,colours);result[run]['colours_on_screen']=sorted(colours[c] for c in found)
   # The first row of tiles is IMG00, IMG01, ... (folders first, then by name).
   assert {0,1,2}<=set(colours[c] for c in found),result[run]
   s=len(t());q.key('f11');g['wait'](r'\[files.app.closed\]',s,30);time.sleep(1)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[files.app.fatal]' not in t()
 finally:g['stop']()
 result['ok']=True;print(result)
if __name__=='__main__':main()
