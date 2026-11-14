#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Chastity, the password vault (apps/chastity), keeps a vault across a reboot.

First boot, typed as a person would:

* `run chastity probe` (into Spindle): the vault code on this machine: a KDBX4 file at
  the real key-derivation cost written, read back, refused with the wrong password,
  RFC 6238 codes, the generator, the program's folder. `[chastity.probe.ok]`.
* `run chastity`: no vault yet. In Chastity's window: a master password, repeated
  (`[chastity.created]`, the file saved). `a` adds an entry: title, user name, an empty
  password (Chastity generates one), URL, notes and a TOTP secret; each plain field's
  length is checked as Chastity recorded it, and the title by its fingerprint.
  `l` locks (zeroized), a wrong password is refused, and the window closes.

Second boot on the same disk:

* `run chastity`: the vault is there. A wrong password is refused; the right one unlocks
  it with one entry whose title has the same fingerprint. `/mail` finds it. `c` asks to
  copy the password and is told no (Shell's clipboard takes no copies from ordinary
  programs yet). Closing the window locks and zeroizes.

The serial record must never hold the master password, and the program must not abort."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
import os
# Argon2id at 32 MiB under emulation, five times, over two boots.
os.environ.setdefault('SEXOS_GATE_BOUND','2400')
from quil_autosave_gate import boot
from lua_gate import qcodes,SURFACE
ROOT=Path(__file__).resolve().parents[1]

MASTER='Corr3ct horse!'
WRONG='Corr3ct horse?'
ENTRY=[('title','Mail'),('username','andreas'),('password',''),('url','https://mail.example.org'),('notes','work'),('totp','GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ')]
UNLOCK=900

def fnv(text):
 h=0xcbf29ce484222325
 for b in text.encode():h=((h^b)*0x100000001b3)&(2**64-1)
 return h

def key_bytes_clean(path):
 import key_bytes
 return not key_bytes.scan(str(path))

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 program=ROOT/'iso_root/programs/chastity'
 elf=program.read_bytes();assert elf[:4]==b'\x7fELF'
 # The loader refuses a program whose code holds WRPKRU/XRSTOR bytes anywhere.
 assert key_bytes_clean(program),'chastity holds bytes the program loader refuses (scripts/key_bytes.py)'
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('chastity',elf)])
 results={}

 def session(tag):
  g=boot(a.iso,disk,a.out,tag,a.resolution);t=g['text'];q=g['q']
  def press(ch):
   q.cmd('send-key',{'keys':[{'type':'qcode','data':k}for k in qcodes(ch)],'hold-time':40});time.sleep(.12)
  def spindle(line):
   g['focus'](153);time.sleep(.5)
   # One key at a time, each confirmed by Spindle's echo (see lua_gate.py).
   base=None
   for n,ch in enumerate(line):
    s=len(t())
    q.cmd('send-key',{'keys':[{'type':'qcode','data':k}for k in qcodes(ch)],'hold-time':40})
    m=g['wait'](rf'\[spindle\.input\.recv\] key=printable ch={re.escape(ch)} len=(\d+)\n',s,30)
    if base is None:base=int(m[1])-n
    assert int(m[1])==base+n,f'Spindle dropped a key before {line[:n+1]!r}'
   q.key('ret')
  def typed(text,pattern,timeout=60):
   """Types into Chastity's window (focused) and Enter; waits for `pattern`."""
   s=len(t())
   for ch in text:press(ch)
   q.key('ret')
   return g['wait'](pattern,s,timeout)
  def start(vault):
   s=len(t());spindle('run chastity')
   w=g['wait'](r'\[chastity\.window\] target=(\d+) task=(\d+)\n',s,600)
   g['wait'](rf'\[chastity\.start\] vault={vault}\n',s,120)
   target,task=int(w[1]),int(w[2])
   time.sleep(1);g['focus'](SURFACE[target]);time.sleep(.5)
   return target,task
  def close(target,task,locked):
   s=len(t());g['focus'](SURFACE[target]);time.sleep(.5);q.key('f11')
   if locked:g['wait'](r'\[chastity\.locked\] reason=closed zeroized=1\n',s,60)
   g['wait'](r'\[chastity\.closed\]\n',s,60)
   g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s,180)
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  return g,t,q,press,spindle,typed,start,close

 # First boot: probe, create, add, lock.
 g,t,q,press,spindle,typed,start,close=session('chastity1')
 try:
  s=len(t());spindle('run chastity probe')
  m=g['wait'](r'\[chastity\.probe\.(ok|fail)\] passed=(\d+)/(\d+)\n',s,1200)
  assert m[1]=='ok' and m[2]==m[3] and int(m[3])>=6,f'probe: {m[0]} '+''.join(re.findall(r'\[chastity\.probe\.fail\][^\n]*\n',t()[s:]))
  kdf=g['wait'](r'\[chastity\.probe\.kdf\] memory_mib=(\d+) iterations=(\d+) save_ms=(\d+) open_ms=(\d+) bytes=(\d+)\n',s,10)
  results['kdf']={'memory_mib':int(kdf[1]),'save_ms':int(kdf[3]),'open_ms':int(kdf[4])}
  e=g['wait'](r'\[std\.exit\] target=(\d+) task=(\d+) status=0\n',s,60)
  s2=len(t());g['focus'](SURFACE[int(e[1])]);time.sleep(.5);q.key('f11')
  g['wait'](rf'\[app\.private\.retired\] target={e[1]} task={e[2]} ',s2,180)

  target,task=start('absent')
  s=len(t())
  for ch in MASTER:press(ch)
  q.key('ret');time.sleep(.5)
  typed(MASTER,r'\[chastity\.created\] entries=0\n',UNLOCK)
  g['wait'](r'\[chastity\.saved\] entries=0 bytes=\d+\n',s,10)
  s=len(t());press('a');time.sleep(.5)
  for name,value in ENTRY:
   if name=='password':typed(value,r'\[chastity\.field\] name=password generated=1\n')
   elif name=='totp':typed(value,r'\[chastity\.field\] name=totp set=1\n')
   else:
    m=typed(value,rf'\[chastity\.field\] name={name} len=(\d+)\n')
    assert int(m[1])==len(value),f'{name}: Chastity got {m[1]} characters, typed {len(value)}'
  m=g['wait'](r'\[chastity\.entry\.added\] entries=1 title=(0x[0-9a-f]+)\n',s,60)
  assert int(m[1],16)==fnv('Mail'),'the title Chastity stored is not the one typed'
  g['wait'](r'\[chastity\.saved\] entries=1 bytes=(\d+)\n',s,UNLOCK)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'chastity-list.ppm')})
  s=len(t());press('l');g['wait'](r'\[chastity\.locked\] reason=asked zeroized=1\n',s,30)
  typed(WRONG,r'\[chastity\.unlock\.refused\] reason=wrong_password\n',UNLOCK)
  close(target,task,locked=False)
  assert 'KERNEL PANIC' not in t() and '[std.abort]' not in t() and MASTER not in t()
 finally:g['stop']()

 # Second boot: the vault is still there and opens.
 g,t,q,press,spindle,typed,start,close=session('chastity2')
 try:
  target,task=start('present')
  typed(WRONG,r'\[chastity\.unlock\.refused\] reason=wrong_password\n',UNLOCK)
  m=typed(MASTER,r'\[chastity\.unlocked\] entries=(\d+) titles=(0x[0-9a-f]+)\n',UNLOCK)
  assert int(m[1])==1 and int(m[2],16)==fnv('Mail'),f'after the reboot: {m[0]}'
  s=len(t());press('/');time.sleep(.3)
  typed('mail',r'\[chastity\.search\] words=1 matches=1\n')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'chastity-unlocked.ppm')})
  s=len(t());press('c');m=g['wait'](r'\[chastity\.copy\] what=password ok=(\d)\n',s,30)
  results['clipboard']='copied' if m[1]=='1' else 'refused (ordinary programs)'
  close(target,task,locked=True)
  assert 'KERNEL PANIC' not in t() and '[std.abort]' not in t() and MASTER not in t()
 finally:g['stop']()
 print({'ok':True,**results})

if __name__=='__main__':main()
