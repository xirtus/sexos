#!/usr/bin/env python3
"""Web over the network: `run +net web URL` follows a redirect, fetches the page, its
screen style sheet (not the print one) and its PNG, and draws exactly what the host
renders from the same files. Right focuses a link, Enter opens the second page, and
Backspace comes back."""
import argparse,http.server,os,re,shutil,subprocess,threading,time
from pathlib import Path
from PIL import Image
from browser_gate_viewport import live_viewport
from web_surface_geometry import canvas_box
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site'
W,H=410,330
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path=='/web':body=(ROOT/'iso_root/programs/web').read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  if self.path=='/start':self.send_response(302);self.send_header('Location','/site/index.html');self.end_headers();return
  f=SITE/self.path.removeprefix('/site/') if self.path.startswith('/site/') else None
  if f is None or not f.is_file():self.send_response(404);self.end_headers();return
  body=f.read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def reference(out,page,bar,name):
 path=out/f'reference-{name}.ppm'
 subprocess.run(['cargo','run','-q','-p','sex-web','--example','page','--',str(page),str(path),str(W),str(H),'0','-1',bar],cwd=ROOT,check=True)
 return Image.open(path).convert('RGB')
def main():
 global W,H
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 base=f'http://10.0.2.2:{PORT}'
 g=boot(a.iso,disk,a.out,'webnet',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def compare(name,ref,tile):
  shot=a.out/f'{name}.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5)
  img=Image.open(shot).convert('RGB');bx,by,vw,vh=canvas_box(ROOT,tile,W,H)
  assert bx>=0 and by>=0 and bx+vw<=img.width and by+vh<=img.height,('canvas not visible',tile,W,H)
  checked=bad=worst=0
  for py in range(0,vh,3):
   for px in range(0,vw,5):
    got,want=img.getpixel((bx+px,by+py)),ref.getpixel((px,py))
    d=max(abs(i-j)for i,j in zip(got,want));checked+=1;worst=max(worst,d);bad+=d>2
  img.crop((bx,by,bx+vw,by+vh)).save(a.out/f'{name}.png')
  assert checked>5000 and bad==0,f'{name}: {bad} of {checked} pixels differ (worst {worst})'
  return checked,worst
 loaded=r'\[web.loaded\] bytes_ok=1 items=(\d+) links=(\d+) height=(\d+) sheets=(\d+) images=(\d+) fetch_ticks=(\d+) layout_ticks=(\d+)'
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  # A run with arguments starts from a file: fetch the program to /bin/web first.
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/web');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net web {base}/start');q.key('ret')
  first=g['wait'](loaded,s,300);assert first[4]=='1' and first[5]=='1',('sheets/images',first[4],first[5])
  m=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,120);target,task=m[1],m[2]
  launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  assert f'[net.program.bound] target={target} task={task}' in t()[s:],'the run was not bound to the network'
  time.sleep(3)
  tile,W,H=live_viewport(g,ROOT,surface)
  refs={'index':reference(a.out,SITE/'index.html',f'Field notes — {base}/site/index.html','index'),'two':reference(a.out,SITE/'two.html',f'Second page — {base}/site/two.html','two')}
  index=compare('web-index',refs['index'],tile)
  # Right focuses the first link in view ("Home"), twice more reaches "Second page".
  g['focus'](surface);time.sleep(.3)
  for _ in range(2):s=len(t());q.key('right');g['wait'](r'\[web.shown\] .*focus=(\d+)',s,60)
  s=len(t());q.key('ret');second=g['wait'](loaded,s,300);time.sleep(3)
  two=compare('web-two',refs['two'],tile)
  s=len(t());q.key('backspace');back=g['wait'](loaded,s,300);time.sleep(3)
  again=compare('web-back',refs['index'],tile)
  s=len(t());q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}',s,60)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'first':first[0][12:],'second':second[0][12:],'checked':[index,two,again]})
 finally:g['stop']()
if __name__=='__main__':main()
