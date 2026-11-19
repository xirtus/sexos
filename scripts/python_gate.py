#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""CPython 3.13 (apps/python, vendor/cpython-3.13.18) runs on SexOS.

Each run is typed into Spindle as `run python ARGS`, exactly as a person would:

* `-c print(2**100)`: big integers.
* `-c "..."` (COMP): dict, list and set comprehensions, f-strings with format specs,
  `%` formatting, str methods, map/join.
* `-c "..."` (JSON): `json.dumps`/`json.loads` and `re.sub`, from the frozen standard
  library (json, re, enum, functools, collections...).
* `-c "..."` (FILES): open/write/read of a script file in the program's folder.
* `t.py x y`: that script file run with arguments (`sys.argv`), `os.listdir` of the
  folder, and `time.time()` against this host's clock.
* `-c "..."` (HASH): hashlib's built-in md5/sha1/sha256/sha3/blake2, zlib and gzip
  round trips (what Qupid's Kodi addon helper needs before any network).
* `run +net python -c "..."` (NET): the socket module against a TCP echo service on
  this host (QEMU's user network, 10.0.2.2): create_connection, sendall, recv, and
  select on the socket.
* `-c "..."` (SQL): sqlite3 on a database file in the program's folder (SQLite on
  apps/python/port/sqlite_vfs.c): create, insert with a parameter, commit, query.
* `run +net python -c "..."` (TLS): the ssl module (apps/python/lib/ssl.py, TLS done by
  the network service) against a TLS echo service whose certificate nothing trusts:
  the handshake runs and the certificate is refused with SSLCertVerificationError.
* `-c "..."` (RAISE): an uncaught exception: the traceback reaches standard error and
  the exit status is 1. The window interprets no escape sequences, so the
  traceback must come without colour.

Output must reach the serial record as `[std.out]`/`[std.err]` lines, and each
program must exit with the right status and close."""
import argparse,re,shutil,socket,socketserver,threading,time
from pathlib import Path
from install_program import install
import os
os.environ.setdefault('SEXOS_GATE_BOUND','1200')
# A NIC on QEMU's user network, so the NET run reaches this host's echo service (10.0.2.2).
os.environ.setdefault('SEXOS_GATE_NIC','-netdev user,id=n0 -device e1000e,netdev=n0')
from quil_autosave_gate import boot
from lua_gate import qcodes
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}

# Spindle's line holds 256 bytes; quotes group words, with \" for a quote inside
# (crates/sex-std/src/args.rs). Python's own escapes (\n, \') pass through.
COMP=("\"d={k:k*k for k in range(6)};l=[v for v in d.values() if v%2==0];s={c.upper() for c in 'abca'};"
      "print('[py.c]',d,l,sorted(s),f'{sum(l):05d}|{3.14159:8.3f}|{255:#x}|%s-%d'%('x',7),'-'.join(map(str,range(3))))\"")
JSON=("\"import json,re;print('[py.j]',json.dumps({'b':[1,2.5,None,True],'a':{'c':'x'}},sort_keys=True),"
      "json.loads('[1,{\\\"k\\\":[2,3]}]')[1]['k'][1],re.sub(r'[0-9]+','#','a1b22c'))\"")
FILES=("\"f=open('t.py','w');f.write('import sys,os,time\\nprint(\\'[py.s]\\',sys.argv[1:],\\'t.py\\' in os.listdir(\\'.\\'),int(time.time()))\\n');"
       "f.close();print('[py.f]',len(open('t.py').read().splitlines()))\"")
RAISE="\"import json;json.loads('{bad')\""
HASH=("\"import hashlib as h,zlib,gzip;d=b'q'*99;c=zlib.compress(d);print('[py.h]',*[h.new(n,b'abc').hexdigest()[:8]for n in('md5','sha1','sha256','sha3_256','blake2b')],"
      "zlib.crc32(d),zlib.decompress(c)==d,gzip.decompress(gzip.compress(d))==d)\"")
SQL=("\"import sqlite3;c=sqlite3.connect('q.db');c.execute('create table t(n)');c.execute('insert into t values(?)',(7,));c.commit();"
     "print('[py.q]',sqlite3.sqlite_version,c.execute('select sum(n)*6,count(*) from t').fetchone());c.close()\"")
def tls(port):
 return ("\"import socket,ssl;s=socket.create_connection(('10.0.2.2',%d),5);"
         "exec('try:ssl.create_default_context().wrap_socket(s,server_hostname=\\'10.0.2.2\\')\\nexcept ssl.SSLCertVerificationError as e:print(\\'[py.t]\\',type(e).__name__)')\""%port)
def net(port):
 return ("\"import socket,select;s=socket.create_connection(('10.0.2.2',%d),5);s.sendall(b'qupid');"
         "r=select.select([s],[],[],5)[0];print('[py.n]',bool(r),s.recv(64));s.close()\""%port)

class Echo(socketserver.BaseRequestHandler):
 def handle(self):
  while(d:=self.request.recv(4096)):self.request.sendall(d)
class EchoServer(socketserver.ThreadingTCPServer):daemon_threads=True;allow_reuse_address=True

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 elf=(ROOT/'iso_root/programs/python').read_bytes();assert elf[:4]==b'\x7fELF'
 if b'This build has no C compiler output (musl headers were missing).' in elf:
  raise SystemExit('python is the no-compiler fallback: install musl headers or set MUSL_INCLUDE, then rebuild before running this gate')
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('python',elf)])
 g=boot(a.iso,disk,a.out,'python',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  assert len(line)<256,f'line too long for Spindle: {len(line)}'
  g['focus'](153);time.sleep(.5)
  # One key at a time, each confirmed by Spindle's echo (see lua_gate.py).
  base=None
  for n,ch in enumerate(line):
   s=len(t())
   q.cmd('send-key',{'keys':[{'type':'qcode','data':k}for k in qcodes(ch)],'hold-time':40})
   m=g['wait'](rf'\[spindle\.input\.recv\] key=printable ch={re.escape(ch)} len=(\d+)\n',s,30)
   if base is None:base=int(m[1])-n
   assert int(m[1])==base+n,f'Spindle dropped a key before {line[:n+1]!r}'
  s=len(t());q.key('ret')
  g['wait'](rf'\[spindle\.input\.recv\] key=enter len={base+len(line)-1}\n',s,30)
 def run(args,expect,status=0,grant=''):
  s=len(t());type_line(f'run {grant}python {args}')
  found=[g['wait'](pattern,s,600) for pattern in expect]
  e=g['wait'](rf'\[std\.exit\] target=(\d+) task=(\d+) status={status}\n',s,300)
  target,task=int(e[1]),int(e[2])
  lines=re.findall(r'\[std\.(?:out|err)\][^\n]*',t()[s:])
  s2=len(t());g['focus'](SURFACE[target]);time.sleep(.5);q.key('f11')
  g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s2,180)
  return found,lines
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  report=[]
  _,l=run('-c print(2**100)',[r'\[std\.out\] 1267650600228229401496703205376\n']);report+=l
  _,l=run('-c '+COMP,[re.escape("[std.out] [py.c] {0: 0, 1: 1, 2: 4, 3: 9, 4: 16, 5: 25} [0, 4, 16] ['A', 'B', 'C'] 00020|   3.142|0xff|x-7 0-1-2\n")]);report+=l
  _,l=run('-c '+JSON,[re.escape('[std.out] [py.j] {"a": {"c": "x"}, "b": [1, 2.5, null, true]} 3 a#b#c\n')]);report+=l
  _,l=run('-c '+FILES,[r'\[std\.out\] \[py\.f\] 2\n']);report+=l
  (m,),l=run('t.py x y',[r"\[std\.out\] \[py\.s\] \['x', 'y'\] True (\d+)\n"]);report+=l
  assert abs(int(m[1])-time.time())<600,f'time.time() is off: {m[1]} vs {int(time.time())}'
  _,l=run('-c '+HASH,[re.escape('[std.out] [py.h] 90015098 a9993e36 ba7816bf 3a985da7 ba80a53f 2985838702 True True\n')]);report+=l
  _,l=run('-c '+SQL,[re.escape('[std.out] [py.q] 3.53.4 (42, 1)\n')]);report+=l
  echo=EchoServer(('127.0.0.1',0),Echo);threading.Thread(target=echo.serve_forever,daemon=True).start()
  _,l=run('-c '+net(echo.server_address[1]),[re.escape("[std.out] [py.n] True b'qupid'\n")],grant='+net ');report+=l
  import ssl,subprocess,tempfile
  tmp=tempfile.mkdtemp();key,crt=tmp+'/k.pem',tmp+'/c.pem'
  subprocess.run(['openssl','req','-x509','-newkey','ec','-pkeyopt','ec_paramgen_curve:prime256v1','-nodes','-keyout',key,'-out',crt,'-subj','/CN=10.0.2.2','-days','2'],check=True,capture_output=True)
  untrusted=EchoServer(('127.0.0.1',0),Echo);ctx=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);ctx.load_cert_chain(crt,key)
  untrusted.socket=ctx.wrap_socket(untrusted.socket,server_side=True,do_handshake_on_connect=False)
  threading.Thread(target=untrusted.serve_forever,daemon=True).start()
  _,l=run('-c '+tls(untrusted.server_address[1]),[re.escape('[std.out] [py.t] SSLCertVerificationError\n')],grant='+net ');report+=l
  _,l=run('-c '+RAISE,[r'\[std\.err\] Traceback \(most recent call last\):\n',r'\[std\.err\] json\.decoder\.JSONDecodeError: [^\n]*\n'],status=1);report+=l
  assert not any('\x1b' in x for x in l),'escape sequences reached the window (TERM=dumb)'
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  for line in report:print(line)
  print({'ok':True,'runs':10})
 finally:g['stop']()
if __name__=='__main__':main()
