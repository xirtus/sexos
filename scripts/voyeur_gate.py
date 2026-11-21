#!/usr/bin/env python3
"""Voyeur (system search) on a booted desktop: Super opens the launcher;
`2+2` answers 4 first (Abacus), `bell` is answered by more than one provider,
and a setting name puts its settings page first. With Loom (files) and
Kaleidoscope (browser) running as program providers: a word inside a text file
puts that file first (from Loom's index), Tab picks its other actions and
reveal shows it selected in Loom; a word of a visited page's title puts that
history entry first, and copy is done by the Shell. Reads [voyeur.query]
serial records; screens are saved for review."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
WORD='xylophone'
NOTES=f'Tuesday {WORD} lessons: practice scales first.\n'.encode()
PAGE=b'<!doctype html><html><head><title>Platypus Field Notes</title></head><body><h1>Platypus Field Notes</h1><p>Seen at dawn by the river.</p></body></html>'
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  body={'/files':lambda:(ROOT/'iso_root/programs/loom').read_bytes(),'/web':lambda:(ROOT/'iso_root/programs/kaleidoscope').read_bytes(),
        '/notes.txt':lambda:NOTES,'/site/platypus.html':lambda:PAGE}.get(self.path)
  if body is None:self.send_response(404);self.end_headers();return
  body=body();self.send_response(200);self.send_header('Content-Length',str(len(body)))
  if self.path.endswith('.html'):self.send_header('Content-Type','text/html')
  self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
SHIFTED={':':'semicolon','+':'equal'}
QUERY=r'\[voyeur.query\] ticket=\d+ query={} providers=(\d+) answered=(\S+) results=(\d+) first=([^\n]+?) first_kind=(\S+) first_provider=(\S+)'
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(max(1024**3,disk.stat().st_size))
 g=boot(a.iso,disk,a.out,'voyeur',a.resolution);t=g['text'];q=g['q']
 base=f'http://10.0.2.2:{PORT}'
 def send(down,keys):q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}for k in keys]});time.sleep(.08)
 def tap(k,shift=False):
  if shift:send(True,['shift'])
  send(True,[k]);send(False,[k])
  if shift:send(False,['shift'])
 def typed(text):
  for k in text:
   if k in SHIFTED:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':SHIFTED[k]}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def spindle(text,pattern,timeout=60):g['focus'](153);time.sleep(.5);s=len(t());typed(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def fetch(src,dest):m=spindle(f'fetch {base}/{src} {dest}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',300);assert m[1].startswith('Saved'),(src,m[1])
 def query(keys,text):
  s=len(t())
  for k in keys:tap(*k) if isinstance(k,tuple) else tap(k)
  m=g['wait'](QUERY.format(re.escape(text)),s,30)
  return s,{'providers':int(m[1]),'answered':m[2],'results':int(m[3]),'first':m[4].strip(),'kind':m[5],'provider':m[6]}
 def first(s,text,title,kind,provider):
  # Programs answer after the Shell's own merge: wait for the merge that has theirs.
  m=g['wait'](QUERY.format(re.escape(text)).replace(r'first=([^\n]+?) first_kind=(\S+) first_provider=(\S+)',f'first={re.escape(title)} first_kind={kind} first_provider={provider}'),s,30)
  return {'providers':int(m[1]),'answered':m[2],'results':int(m[3]),'first':title,'kind':kind,'provider':provider}
 def clear(n):
  for _ in range(n):tap('backspace')
 def launcher():s=len(t());tap('meta_l');g['wait'](r'\[shell.palette.open\]',s,30);time.sleep(1)
 def shot(name):time.sleep(1.5);q.cmd('screendump',{'filename':str(a.out/f'{name}.ppm')});time.sleep(.5)
 results={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(3)
  # The programs that hold the data: Loom (files and their words) and Kaleidoscope (history).
  spindle('mkdir voyeur',r'\[spindle.fs.result\] kind=3 status=0')
  fetch('files','/bin/loom');fetch('notes.txt','voyeur/notes.txt');fetch('web','/bin/kaleidoscope')
  s=len(t());spindle('files',r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)')
  g['wait'](r'\[voyeur.provider.register\] pd=\d+ name=loom status=Ok ',s,60)
  m=g['wait'](r'\[files.app.indexed\] base=/ files=(\d+) looked=(\d+) read=(\d+) ',s,300);results['indexed']=m[0]
  s=len(t());spindle(f'run +net kaleidoscope {base}/site/platypus.html',r'\[web.ready\] target=\d+ task=\d+',300)
  g['wait'](r'\[voyeur.provider.register\] pd=\d+ name=kaleidoscope status=Ok ',s,60)
  g['wait'](r'\[web.loaded\] bytes_ok=1 ',s,120);time.sleep(2)
  launcher()
  _,r=query(['2',('equal',True),'2'],'2+2');assert (r['first'],r['kind'],r['provider'])==('4','calculator','abacus'),r;results['2+2']=r
  shot('01-two-plus-two');clear(3)
  _,r=query(['b','e','l','l'],'bell');assert r['providers']>=2 and r['first']=='Open Bell',r;results['bell']=r
  shot('02-bell-merged');clear(4)
  _,r=query(list('wallpaper'),'wallpaper');assert (r['first'],r['kind'])==('Wallpaper','setting'),r;results['wallpaper']=r
  s=len(t());tap('ret');g['wait'](r'\[voyeur.activate\] provider=settings kind=setting',s,30)
  # A word inside a file: Loom's index answers; Tab picks open with, then reveal.
  launcher()
  s,_=query(list(WORD),WORD);results[WORD]=first(s,WORD,'notes.txt','content','loom')
  shot('03-file-word')
  s=len(t());tap('tab');g['wait'](r'\[voyeur.action\] index=0 title=notes.txt action=open_with',s,30)
  s=len(t());tap('tab');g['wait'](r'\[voyeur.action\] index=0 title=notes.txt action=reveal',s,30)
  s=len(t());tap('ret')
  g['wait'](r'\[voyeur.activate\] provider=loom kind=content action=reveal title=notes.txt ok=1 detail=program',s,30)
  g['wait'](r'\[files.app.voyeur\] action=3 path=/voyeur/notes.txt ok=1',s,30)
  g['wait'](r'\[files.app.list\] dir=/voyeur count=1\b',s,30);shot('04-revealed-in-loom')
  # A visited page's title: Kaleidoscope's history answers; Tab picks copy, which the Shell does.
  launcher()
  s,_=query(list('platypus'),'platypus');results['platypus']=first(s,'platypus','Platypus Field Notes','history','kaleidoscope')
  shot('05-history')
  s=len(t());tap('tab');g['wait'](r'\[voyeur.action\] index=0 title=Platypus Field Notes action=copy',s,30)
  s=len(t());tap('ret');g['wait'](r'\[voyeur.activate\] provider=kaleidoscope kind=history action=copy title=Platypus Field Notes ok=1 detail=copy',s,30)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,**{k:str(v) for k,v in results.items()}})
 finally:g['stop']()
if __name__=='__main__':main()
