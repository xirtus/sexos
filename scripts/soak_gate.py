#!/usr/bin/env python3
"""A working session, repeated for hours: files, downloads, the Browser, programs side by side, a notepad, background programs. Any crash, hang or leak ends it with evidence.

Each round also reads free program memory from Monitor (a leak shows as a fall of more
than --leak-pages below the best of the first rounds), times one key typed into Spindle
(--max-key-ms), and reads back every note saved so far. With --inject (an ISO built
with SEXOS_CRASH_PROBE=1) each round stops one restartable service in turn, as a fault
would, and the session must carry on: the service comes back, saved notes are intact
and the workload continues. soak.json is rewritten after every round. The 24 h run:
  SEXOS_GATE_CORES=2 python3 scripts/soak_gate.py ... --hours 24 --inject"""
import argparse,http.server,json,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
# The system monitor (Mesh, formerly Monitor); it logs [monitor.*] either way.
MESH='mesh' if (ROOT/'iso_root/programs/mesh').exists() else 'monitor'
PAGE=b'<html><head><title>Soak</title></head><body><p>Still here.</p><a href="/">again</a></body></html>'
FILES={'/':PAGE,'/jot':(ROOT/'iso_root/programs/jot').read_bytes(),'/fortune':(ROOT/'iso_root/programs/fortune').read_bytes(),'/hum':(ROOT/'iso_root/programs/hum').read_bytes(),'/'+MESH:(ROOT/'iso_root/programs'/MESH).read_bytes(),'/blob':bytes(i%251 for i in range(200_000))}
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
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--hours',type=float,default=2.0)
 p.add_argument('--inject',action='store_true');p.add_argument('--leak-pages',type=int,default=256);p.add_argument('--max-key-ms',type=float,default=3000.0);p.add_argument('--warmup',type=int,default=3);a=p.parse_args()
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
 def key_ms():
  g['focus'](153);time.sleep(1);s=len(t());t0=time.monotonic();q.key('x')
  while not re.search(r'\[display\.text\.committed\] target=12 ',t()[s:]):
   assert time.monotonic()-t0<30,'a key never reached the screen';time.sleep(.0005)
  ms=(time.monotonic()-t0)*1000;time.sleep(.2);q.key('backspace');return round(ms,1)
 def free_pages():
  s=len(t());target,task,sid=started('run '+MESH,'monitor')
  m=g['wait'](r'\[monitor\.sample\] [^\n]*free_pages=(\d+)',s,60)
  g['focus'](sid);time.sleep(.3);s=len(t());q.key('f11');g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,90)
  return int(m[1])
 # Services that start again in place, and their crash_log names.
 VICTIMS=[('Files',11),('Network',13),('Bell',10),('Input',4),('Shell',3),('Display',1)]
 def inject(n):
  name,pd=VICTIMS[(n-1)%len(VICTIMS)];s=len(t())
  expect(f'crash {name.lower()}',rf'\[spindle.crash.probe\] target={pd}\b')
  g['wait'](rf'\[service.restarted\] pd={pd} task=\d+ restarts=\d+ [^\n]*admitted=1',s,180)
  if pd==3:g['wait'](r'\[shell.restarted\]',s,180)
  time.sleep(8);return name
 notes={};record={'rounds':[]};out=a.out/'soak.json'
 def save(ok,why=''):record.update(ok=ok,why=why,inject=a.inject,cores=os.environ.get('SEXOS_GATE_CORES','1'),minutes=round((time.monotonic()-began)/60,1));out.write_text(json.dumps(record))
 rounds=[];began=time.monotonic()
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  for name in ('jot','fortune','hum',MESH):fetched('/'+name,'/bin/'+name)
  census0=None
  while time.monotonic()-began<a.hours*3600:
   n=len(rounds)+1;r0=time.monotonic()
   # Files: write, read back, list, remove.
   info={'n':n}
   if a.inject:info['crashed']=inject(n)
   expect(f'write note{n%5}.txt round {n}',r'\[spindle.fs.result\] kind=8 status=0');notes[n%5]=n
   # Every note saved so far, through any service restarts since.
   for k,v in sorted(notes.items()):expect(f'cat note{k}.txt',rf'\[spindle.fs.cat\] round {v}\b',120)
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
   info['key_ms']=key_ms();info['free_pages']=free_pages();rounds.append(round(time.monotonic()-r0,1));info['seconds']=rounds[-1]
   record['rounds'].append(info);save(True)
   assert info['key_ms']<=a.max_key_ms,f"round {n}: a key took {info['key_ms']} ms"
   best=max(r['free_pages'] for r in record['rounds'][:a.warmup])
   if n>a.warmup:assert best-info['free_pages']<=a.leak_pages,f"round {n}: free program memory {info['free_pages']} pages, {best-info['free_pages']} below the first rounds' {best}"
   print(f'[soak] round={n} seconds={rounds[-1]} elapsed_min={round((time.monotonic()-began)/60,1)}',flush=True)
  q.cmd('screendump',{'filename':str(a.out/'end.ppm')});time.sleep(.5)
  print({'ok':True,'rounds':len(rounds),'hours':round((time.monotonic()-began)/3600,2),'slowest':max(rounds),'fastest':min(rounds)})
 except BaseException as e:
  save(False,repr(e));raise
 finally:g['stop']()
if __name__=='__main__':main()
