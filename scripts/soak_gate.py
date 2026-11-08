#!/usr/bin/env python3
"""A working session, repeated for hours: files, downloads, the Browser, programs side by side, a notepad, background programs. Any crash, hang or leak ends it with evidence."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
PAGE=b'<html><head><title>Soak</title></head><body><p>Still here.</p><a href="/">again</a></body></html>'
FILES={'/':PAGE,'/jot':(ROOT/'iso_root/programs/jot').read_bytes(),'/fortune':(ROOT/'iso_root/programs/fortune').read_bytes(),'/hum':(ROOT/'iso_root/programs/hum').read_bytes(),'/blob':bytes(i%251 for i in range(200_000))}
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  body=FILES.get(self.path)
  if body is None:self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Type','text/html');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--hours',type=float,default=2.0);a=p.parse_args()
 os.environ['SEXOS_GATE_BOUND']=str(int(a.hours*3600)+1800)
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'soak',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def expect(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def chord(mod,key):q.cmd('send-key',{'keys':[{'type':'qcode','data':mod},{'type':'qcode','data':key}],'hold-time':80})
 base=f'http://10.0.2.2:{PORT}'
 def fetched(url,name):
  m=expect(f'fetch {base}{url} {name}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',300);assert m[1].startswith('Saved'),m[1]
 def started(line,word):
  s=len(t());expect(line,rf'\[{word}.ready\] target=(\d+)');m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['wait'](rf'\[display.text.committed\] target={m[1]} ',s,60);return m[1],m[2],int(m[3])
 def close(target,task,sid,word):
  g['focus'](sid);time.sleep(.3);s=len(t());q.key('f11');g['wait'](rf'\[{word}.closed\] target={target} task={task}',s,60);g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,90)
 rounds=[];began=time.monotonic()
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  for name in ('jot','fortune','hum'):fetched('/'+name,'/bin/'+name)
  census0=None
  while time.monotonic()-began<a.hours*3600:
   n=len(rounds)+1;r0=time.monotonic()
   # Files: write, read back, list, remove.
   expect(f'write note{n%5}.txt round {n}',r'\[spindle.fs.result\] kind=8 status=0')
   expect(f'cat note{n%5}.txt',rf'\[spindle.fs.cat\] round {n}\b')
   expect('df',r'MiB free of')
   # A download, replaced each round.
   fetched('/blob','blob.bin')
   # The Browser.
   g['focus'](300);time.sleep(.5);chord('ctrl','l');time.sleep(.4)
   for _ in range(60):q.key('backspace');time.sleep(.005)
   keys(base+'/');time.sleep(.5);s=len(t());q.key('ret');g['wait'](r'\[browser.web.page\] source=\d+',s,180)
   # Programs: a notepad saves, then several side by side close.
   target,task,sid=started('run jot','jot');g['focus'](sid);time.sleep(.3);s=len(t());keys(f'r{n}');chord('ctrl','s');g['wait'](r'\[jot.saved\]',s,60);close(target,task,sid,'jot')
   live=[started('run fortune','fortune') for _ in range(4)]
   for target,task,sid in reversed(live):close(target,task,sid,'fortune')
   # Background programs come and go, and every page comes back.
   expect('spawn hum 10',r'\[shell.records.spawned\] live=10\b')
   g['focus'](153);time.sleep(.3);keys('stopall force');q.key('ret');time.sleep(5)
   m=expect('records',r'\[records.census\] live=(\d+) started=\d+ ended=\d+ capacity=\d+ pool_free=(\d+) pool_total=(\d+)')
   assert m[1]=='0' and m[2]==m[3],f'records left behind: {m[0]}'
   for bad in ('KERNEL PANIC','panicked at'):assert bad not in t(),bad
   # `stopall force` ends background records by taking their pages; nothing else may fault.
   killed=[int(p) for p in re.findall(r'fault.kill \[paging.fault.terminal\] pd=(\d+)',t()) if int(p)<1024];assert not killed,f'killed: {killed}'
   rounds.append(round(time.monotonic()-r0,1))
   print(f'[soak] round={n} seconds={rounds[-1]} elapsed_min={round((time.monotonic()-began)/60,1)}',flush=True)
  q.cmd('screendump',{'filename':str(a.out/'end.ppm')});time.sleep(.5)
  print({'ok':True,'rounds':len(rounds),'hours':round((time.monotonic()-began)/3600,2),'slowest':max(rounds),'fastest':min(rounds)})
 finally:g['stop']()
if __name__=='__main__':main()
