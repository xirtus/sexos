#!/usr/bin/env python3
"""Signed packages, installed through Files, and the question a package's first run asks.

The host builds chello (sex-libc's C program) into packages with tools/sexpkg, signed
with the gates' public test key, and serves them over HTTP. In Spindle:

* `pkg install URL` of a good package is refused while its publisher is not trusted;
* `pkg trust KEY`, then a package with one changed byte is refused (signature);
* the good package installs into /bin; `run chello one two` asks "chello asks to use the
  network"; the answer `no` runs it without the network: its own check that connect()
  fails with EACCES passes (21 checks);
* `pkg allow chello network`: `run chello net HOST PORT` now reaches the echo server
  without `+net` and without a question (23 checks);
* 1.1 upgrades it, 1.0 is then refused as a downgrade, `pkg list` shows one package,
  `pkg remove chello` removes it and `pkg list` shows none."""
import argparse,functools,http.server,os,re,shutil,socket,socketserver,subprocess,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]

class Echo(socketserver.BaseRequestHandler):
 def handle(self):
  while True:
   b=self.request.recv(65536)
   if not b:break
   self.request.sendall(b)
  self.request.shutdown(socket.SHUT_WR)
class EchoServer(socketserver.ThreadingTCPServer):daemon_threads=True;allow_reuse_address=True
class Quiet(http.server.SimpleHTTPRequestHandler):
 def log_message(self,*a):pass
ECHO=EchoServer(('127.0.0.1',0),Echo);ECHO_PORT=ECHO.server_address[1]
threading.Thread(target=ECHO.serve_forever,daemon=True).start()
UDP=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);UDP.bind(('127.0.0.1',ECHO_PORT))
def udp_echo():
 while True:
  b,a=UDP.recvfrom(65536);UDP.sendto(b,a)
threading.Thread(target=udp_echo,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
from quil_autosave_gate import boot

SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}
KEYS={' ':'spc','.':'dot','/':'slash','-':'minus','=':'equal',',':'comma'}
SHIFTED={':':'semicolon','_':'minus','+':'equal'}

def sexpkg(*args):
 return subprocess.run(['cargo','run','-q','--release','--manifest-path',str(ROOT/'tools/sexpkg/Cargo.toml'),'--',*map(str,args)],check=True,capture_output=True,text=True).stdout.strip()

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True)
 elf=ROOT/'iso_root/programs/chello'
 data=elf.read_bytes();assert data[:4]==b'\x7fELF'
 if b'This build has no C compiler output' in data:raise SystemExit('chello is the no-compiler fallback: install musl headers and rebuild')
 serve=a.out/'serve';shutil.rmtree(serve,ignore_errors=True);serve.mkdir()
 (serve/'README').write_text('chello from a package\n')
 for v in ['1.0','1.1']:
  (serve/f'm{v}').write_text(f'name=chello\nversion={v}\nsummary=C library checks\nlicense=GPL-3.0\ngrant=network\n')
  sexpkg('build','test',serve/f'm{v}',serve/f'chello-{v}.sexpkg',f'bin/chello={elf}',f'data/README={serve/"README"}')
 good=(serve/'chello-1.0.sexpkg').read_bytes();bad=bytearray(good);bad[len(good)//2]^=1;(serve/'bad.sexpkg').write_bytes(bad)
 key=sexpkg('public','test');assert re.fullmatch(r'[0-9a-f]{64}',key)
 handler=functools.partial(Quiet,directory=str(serve))
 web=http.server.ThreadingHTTPServer(('127.0.0.1',0),handler);threading.Thread(target=web.serve_forever,daemon=True).start()
 base=f'http://10.0.2.2:{web.server_address[1]}/'
 disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 g=boot(a.iso,disk,a.out,'pkg',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  g['focus'](153);time.sleep(.5)
  for k in line:
   if k in SHIFTED:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':SHIFTED[k]}],'hold-time':80})
   elif k.isupper():q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':k.lower()}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.04)
  q.key('ret')
 def do(line,pattern,timeout=600):
  s=len(t());type_line(line);return g['wait'](pattern,s,timeout),s
 def checks(score,least,s):
  m=g['wait'](score,s,600)
  if m[1]!='ok':time.sleep(1);raise AssertionError('chello checks failed: '+''.join(re.findall(r'\[std\.out\] FAIL[^\n]*\n',t()[s:])))
  assert int(m[3])>=least and m[2]==m[3],f'not every check ran: {m[0]}'
  e=g['wait'](r'\[std\.exit\] target=(\d+) task=(\d+) status=0\n',s,60)
  s2=len(t());g['focus'](SURFACE[int(e[1])]);time.sleep(.5);q.key('f11')
  g['wait'](rf'\[app\.private\.retired\] target={e[1]} task={e[2]} ',s2,180)
  return int(m[2])
 out={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  m,_=do(f'pkg install {base}chello-1.0.sexpkg',r'\[files\.pkg\.(refused|installed)\] ([^\n]*)\n')
  assert m[1]=='refused' and 'does not trust' in m[2],f'an untrusted package was not refused: {m[0]}'
  do(f'pkg trust {key}',r'\[files\.pkg\.trusted\] key='+key[:16])
  m,_=do(f'pkg install {base}bad.sexpkg',r'\[files\.pkg\.(refused|installed)\] ([^\n]*)\n')
  assert m[1]=='refused','a changed package was installed: '+m[0]
  out['tampered']=m[2]
  m,_=do(f'pkg install {base}chello-1.0.sexpkg',r'\[files\.pkg\.(refused|installed)\] ([^\n]*)\n')
  assert m[1]=='installed' and 'name=chello version=1.0 program=chello upgrade=0' in m[2],m[0]
  # First run: the question, answered no. The program's own check sees EACCES.
  m,s=do('run chello one two',r'\[spindle\.pkg\.question\] program=chello grants=1\n')
  do('no',r'\[spindle\.pkg\.run\] program=chello network=0\n')
  out['denied_checks']=checks(r'\[chello\.(ok|fail)\] passed=(\d+)/(\d+)\n',21,s)
  # Allowed now: no question, and the network works without +net.
  do('pkg allow chello network',r'\[files\.pkg\.consent\] program=chello answered=1 allowed=1\n')
  m,s=do(f'run chello net 10.0.2.2 {ECHO_PORT}',r'\[spindle\.pkg\.note\] program=chello asked=1 answered=1 allowed=1\n')
  out['allowed_checks']=checks(r'\[chello\.(ok|fail)\] passed=(\d+)/(\d+)\n',23,s)
  assert '[spindle.pkg.question]' not in t()[s:],'asked again after an answer'
  m,_=do(f'pkg install {base}chello-1.1.sexpkg',r'\[files\.pkg\.(refused|installed)\] ([^\n]*)\n')
  assert m[1]=='installed' and 'version=1.1 program=chello upgrade=1' in m[2],m[0]
  m,_=do(f'pkg install {base}chello-1.0.sexpkg',r'\[files\.pkg\.(refused|installed)\] ([^\n]*)\n')
  assert m[1]=='refused' and 'Not downgraded' in m[2],m[0]
  do('pkg list',r'\[files\.pkg\.list\] count=1\n')
  do('pkg remove chello',r'\[files\.pkg\.removed\] name=chello program=chello\n')
  do('pkg list',r'\[files\.pkg\.list\] count=0\n')
  text=t();assert 'KERNEL PANIC' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,**out})
 finally:
  g['stop']();web.shutdown()
if __name__=='__main__':main()
