#!/usr/bin/env python3
"""Real Web window geometry, JS resize events and full document edge pixels."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
from browser_gate_viewport import painted_viewport
from web_surface_geometry import canvas_box
from quil_workflow_gate import palette_commands,palette_select
from PIL import Image
import hashlib,json
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site'
BAR=30
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path=='/web':body=(ROOT/'iso_root/programs/kaleidoscope').read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
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
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 rw,rh=map(int,a.resolution.split('x'))
 base=f'http://10.0.2.2:{PORT}';url=f'{base}/site/viewport.html'
 g=boot(a.iso,disk,a.out,'webviewport',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {url}');q.key('ret')
  g['wait'](r'\[web.console\] level=0 viewport size \d+x\d+ count=\d+',s,300)
  m=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,120);target,task=m[1],m[2]
  launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  commands=palette_commands(ROOT)
  samples=[]
  def inspect(name,start):
   tile,pw,ph=painted_viewport(g,ROOT,surface)
   bx,by,_,_=canvas_box(ROOT,tile,pw,ph)
   # The script must observe the actual document size, excluding its toolbar.
   matches=re.findall(r'\[web.console\] level=0 viewport size (\d+)x(\d+) count=(\d+)',t()[start:])
   assert matches and tuple(map(int,matches[-1][:2]))==(pw,ph-BAR),('JS viewport mismatch',matches,pw,ph)
   # Move actual cursor out of the canvas; no pixel masks or clipped references.
   q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':100}},{'type':'abs','data':{'axis':'y','value':100}}]})
   # Client paint/resize receipts precede Display's actual scanout commit.
   # Wait for the complete independent pixel oracle, never just a log or a
   # fixed sleep. Keep every prior mismatch image and fail within 12 seconds.
   began=time.monotonic();deadline=began+12;attempts=[]
   while True:
    shot=a.out/f'{name}-attempt-{len(attempts)+1:02}.ppm'
    q.cmd('screendump',{'filename':str(shot)})
    img=Image.open(shot).convert('RGB')
    assert bx>=0 and by>=0 and bx+pw<=img.width and by+ph<=img.height
    checked=0;mismatch=None
    # Full document pixels, including right/bottom beyond old410x330.
    for py in range(ph-BAR):
     for px in range(pw):
      want=(147,132,200) if px<48 and py<48 else (113,201,176) if px>=pw-48 and py>=ph-BAR-48 else (23,58,94)
      got=img.getpixel((bx+px,by+BAR+py))
      if max(abs(g-w)for g,w in zip(got,want))>2:
       mismatch=dict(x=px,y=py,got=got,want=want);break
      checked+=1
     if mismatch is not None:break
    attempts.append(dict(image=shot.name,checked=checked,mismatch=mismatch,observed_ms=round((time.monotonic()-began)*1000)))
    (a.out/f'{name}-attempts.json').write_text(json.dumps(attempts,indent=2)+'\n')
    if mismatch is None:break
    assert time.monotonic()<deadline and len(attempts)<32,('scanout did not settle',name,attempts)
    time.sleep(.25)
   assert checked>5000
   img.crop((bx,by,bx+pw,by+ph)).save(a.out/f'{name}.png')
   item=dict(name=name,width=pw,height=ph,resize_count=int(matches[-1][2]),checked=checked,scanout_attempts=len(attempts))
   samples.append(item);return item
  first=inspect('initial',s)
  assert first['width']>0 and first['height']>BAR,first
  # Real Shell input, repeated both ways. Every resize must run page handlers,
  # fit the actual program area and repaint the newly visible right/bottom edge.
  for turn in range(3):
   for zoomed in (True,False):
    g['focus'](surface)
    palette_select(g['key'],'ZoomToggle',commands)
    before=len(t());g['key']('ret',rf'\[shell.window.action\] action=ZoomToggle frame=\d+ sid={surface} ok=1')
    g['wait'](r'\[web.viewport\].* applied=1',before,120)
    item=inspect(f"{turn}-{'max' if zoomed else 'restore'}",before)
    assert item['resize_count']>samples[-2]['resize_count'],samples[-2:]
    if zoomed:
     assert item['width']>first['width'] and item['height']>first['height'],item
     assert item['width']>410 and item['height']>330,item
    else:assert (item['width'],item['height'])==(first['width'],first['height']),item
  s=len(t());q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}',s,60)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  result=dict(ok=True,iso_sha256=hashlib.sha256(a.iso.read_bytes()).hexdigest(),web_sha256=hashlib.sha256((ROOT/'iso_root/programs/kaleidoscope').read_bytes()).hexdigest(),samples=samples)
  (a.out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(result)
 finally:g['stop']()
if __name__=='__main__':main()
