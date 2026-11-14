#!/usr/bin/env python3
"""Quil, the code editor program: installed as a file, it opens a large file lazily, jumps to
its end, edits both ends and saves it byte for byte; finds a file with Go to File; types with
three cursors; deletes a line with Vim's dd and saves with :w; and searches the project folder."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
DEMO=b'fn main() {\n    let needle = 1;\n    println!("{}", needle);\n}\n'
OTHER=b'// another file\npub fn helper() -> u32 { 2 }\n'
FILES={'/quil':(ROOT/'iso_root/programs/quil').read_bytes(),'/demo.rs':DEMO,'/other.rs':OTHER}
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
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','\n':'ret',':':None,'_':None}
def fnv(data):
 h=0xcbf29ce484222325
 for b in data:h=((h^b)*0x100000001b3)&0xFFFFFFFFFFFFFFFF
 return h
def large_expected(mb):
 n=mb<<20;line=bytes(10 if i%80==79 else 97+i%26 for i in range(80*26))
 body=(line*(n//len(line)+1))[:n]
 return b'// top\n'+body+b'quil was here\n'
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--large-mb',type=int,default=int(os.environ.get('QUIL_LARGE_MB','200')));a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(2*1024**3)
 g=boot(a.iso,disk,a.out,'quil',a.resolution);t=g['text'];q=g['q']
 def send(*names):q.cmd('send-key',{'keys':[{'type':'qcode','data':n}for n in names],'hold-time':80});time.sleep(.08)
 def keys(text):
  for k in text:
   if k==':':send('shift','semicolon')
   elif k=='_':send('shift','minus')
   elif k=='>':send('shift','dot')
   elif k in '(){}':send('shift',{'(':'9',')':'0','{':'bracket_left','}':'bracket_right'}[k])
   elif k.isupper():send('shift',k.lower())
   else:q.key(KEYS.get(k,k));time.sleep(.06)
 def line(text):
  g['focus'](153);time.sleep(.5);keys(text);q.key('ret')
 def expect(text,pattern,timeout=60):
  s=len(t());line(text);return g['wait'](pattern,s,timeout)
 def ctrl(*k):send('ctrl',*k)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  m=expect(f'fetch http://10.0.2.2:{PORT}/quil /bin/quil',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  # The first run makes Quil's folder; the fixtures go into it for the second.
  s=len(t());line('run quil');m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['wait'](rf'\[files.program.bound\] target={m[1]} task={m[2]} root=/data/quil',s,60);g['wait'](r'\[quil.code.ready\]',s,60);time.sleep(1)
  g['focus'](int(m[3]));time.sleep(.5);s=len(t());q.key('f11');g['wait'](rf'\[quil.code.closed\] target={m[1]} task={m[2]}',s,60)
  g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s,60);time.sleep(1)
  for name in ('demo.rs','other.rs'):
   m=expect(f'fetch http://10.0.2.2:{PORT}/{name} /data/quil/{name}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),(name,m[1])
  s=len(t());line('run quil');m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  target,task,sid=m[1],m[2],int(m[3])
  g['wait'](r'\[quil.code.ready\] .*keymap=vscode',s,60);g['wait'](rf'\[display.app.title\] target={target} surface={sid}',s,60);time.sleep(2)
  g['focus'](sid);time.sleep(.5)
  # Go to File finds demo.rs among the folder's files.
  s=len(t());ctrl('p');g['wait'](r'\[quil.code.quickopen\] files=(\d+)',s,60);keys('demo');q.key('ret')
  g['wait'](rf'\[quil.code.opened\] path=demo.rs bytes={len(DEMO)} lazy=0',s,60);time.sleep(.5)
  # Three cursors from Ctrl+Shift+Down, typing on every line at once.
  s=len(t());ctrl('home');send('ctrl','shift','down');send('ctrl','shift','down');keys('// ');ctrl('s')
  want=b''.join(b'// '+l+b'\n' if i<3 else l+b'\n' for i,l in enumerate(DEMO.rstrip(b'\n').split(b'\n')))
  g['wait'](rf'\[quil.code.saved\] path=demo.rs bytes={len(want)} hash={fnv(want):016x}',s,60)
  # Vim mode from the command palette; dd deletes the first line, :w saves.
  s=len(t());q.key('esc');send('ctrl','shift','p');time.sleep(.3);keys('vim mode');q.key('ret');g['wait'](r'\[quil.code.keymap\] vim',s,30)
  s=len(t());keys('gg');keys('dd');keys(':w');q.key('ret')
  want2=want.split(b'\n',1)[1]
  g['wait'](rf'\[quil.code.saved\] path=demo.rs bytes={len(want2)} hash={fnv(want2):016x}',s,60)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'quil-vim.ppm')});time.sleep(.5)
  # Search the project folder (Ctrl+Shift+F works in Vim mode too).
  s=len(t());send('ctrl','shift','f');time.sleep(.3);keys('needle');q.key('ret')
  m=g['wait'](r'\[quil.code.search\] matches=(\d+) files=(\d+) ms=(\d+)',s,120);assert int(m[1])==2,m[0]
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'quil-search.ppm')});time.sleep(.5);q.key('esc')
  # Back to VS Code keys, then the large-file check from the palette.
  s=len(t());send('ctrl','shift','p');time.sleep(.3);keys('vs code');q.key('ret');g['wait'](r'\[quil.code.keymap\] vscode',s,30)
  s=len(t());send('ctrl','shift','p');time.sleep(.3);keys(f'large {a.large_mb}');q.key('ret')
  big=large_expected(a.large_mb)
  m=g['wait'](rf'\[quil.code.large\] mb={a.large_mb} write_ms=(\d+) open_ms=(\d+) pieces=(\d+)',s,1200)
  print('large',m[0],flush=True)
  assert int(m[2])<5000,f'opening took {m[2]} ms'
  g['wait'](rf'\[quil.code.saved\] path=large-check.txt bytes={len(big)} hash={fnv(big):016x}',s,1200)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'quil-large.ppm')});time.sleep(.5)
  s=len(t());q.key('f11');g['wait'](rf'\[quil.code.closed\] target={target} task={task}',s,60)
  assert 'KERNEL PANIC'not in t() and 'panicked' not in t() and '[quil.code.fatal]' not in t()
 finally:g['stop']()
 print({'ok':True,'large_mb':a.large_mb})
if __name__=='__main__':main()
