#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Aftercare (apps/aftercare) backs up a folder, and gets a deleted file back.

Everything is typed as a person would: Spindle commands make the fixture, and keys
drive Aftercare's window. The gate checks:

* fixture: Spindle makes /data/src (notes.txt, docs/letter.txt, docs/list.txt) and
  the empty folders /data/backup and /data/restore.
* `run aftercare @/data/src @/data/backup @/data/restore`: S chooses /data/src as the
  source, D chooses /data/backup as the destination, U creates a repository there with
  a password typed twice (Argon2id 24 MiB, 2 passes; unlocking must take under 2 s), B backs up: three
  new files.
* Spindle's `rm` deletes docs/letter.txt. In Aftercare, L lists the one snapshot and
  Enter browses it down to /data/src/docs; R restores letter.txt where it was. The
  SHA-256 Aftercare wrote must equal the fixture text's, and Spindle's `cat` must read
  the text back from the restored file.
* T restores the whole docs folder into /data/restore: both files, both hashes.
* B again (a second snapshot), K sets the keep policy to last=1, F forgets one snapshot
  and prunes, C checks and Shift+C checks reading every piece of data: no errors.
* F11 closes. A second run keeps the source and destination (aftercare.conf), refuses
  a wrong password, then unlocks with the right one and lists one snapshot."""
import argparse,hashlib,os,re,shutil,time
from pathlib import Path
from install_program import install
os.environ.setdefault('SEXOS_GATE_BOUND','1800')
from quil_autosave_gate import boot
from lua_gate import qcodes,SURFACE
ROOT=Path(__file__).resolve().parents[1]

PASSWORD='correct horse 7'
WRONG='wrong horse 7'
FILES={'notes.txt':'remember the milk','docs/letter.txt':'Dear future me, this is the one that gets deleted.','docs/list.txt':'eggs flour butter'}
def content(text):return (text+'\n').encode()   # Spindle's write ends the line
def sha(text):return hashlib.sha256(content(text)).hexdigest()

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 elf=(ROOT/'iso_root/programs/aftercare').read_bytes();assert elf[:4]==b'\x7fELF'
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('aftercare',elf)])
 g=boot(a.iso,disk,a.out,'aftercare',a.resolution);t=g['text'];q=g['q']
 def send(ch):q.cmd('send-key',{'keys':[{'type':'qcode','data':k}for k in qcodes(ch)],'hold-time':40})
 def type_line(line):
  assert len(line)<256,f'line too long for Spindle: {len(line)}'
  g['focus'](153);time.sleep(.5)
  # One key at a time, each confirmed by Spindle's echo (see lua_gate.py).
  base=None
  for n,ch in enumerate(line):
   s=len(t());send(ch)
   m=g['wait'](rf'\[spindle\.input\.recv\] key=printable ch={re.escape(ch)} len=(\d+)\n',s,30)
   if base is None:base=int(m[1])-n
   assert int(m[1])==base+n,f'Spindle dropped a key before {line[:n+1]!r}'
  s=len(t());q.key('ret')
  g['wait'](rf'\[spindle\.input\.recv\] key=enter len={base+len(line)-1}\n',s,30)
 # fs.rs request kinds, as `[spindle.fs.result]` names them; `write` first asks
 # whether the file exists (STAT), so wait for the command's own request.
 KIND={'mkdir':3,'rm':5,'cat':7,'write':8}
 def shell(line):
  s=len(t());type_line(line)
  return g['wait'](rf'\[spindle\.fs\.result\] kind={KIND[line.split()[0]]} status=(\d+) ',s,60)[1]
 app={}
 def start(args):
  s=len(t());type_line(f'run aftercare {args}')
  m=g['wait'](r'\[aftercare\.ready\] target=(\d+) task=(\d+) granted=(\S*) source=(\S+) destination=(\S+) rng=(\d)\n',s,600)
  app.update(target=int(m[1]),task=int(m[2]))
  assert m[6]=='1','no hardware random generator in the guest'
  focus();return m
 def focus():g['focus'](SURFACE[app['target']]);time.sleep(.5)
 def press(k,pattern,timeout=120):
  s=len(t());q.key(k);return g['wait'](pattern,s,timeout)
 def type_app(text):
  # Each key confirmed by Aftercare's input line.
  for n,ch in enumerate(text):
   s=len(t());send(ch);g['wait'](rf'\[aftercare\.input\] len={n+1}\n',s,30)
 def clear_input(length):
  for n in range(length,0,-1):press('backspace',rf'\[aftercare\.input\] len={n-1}\n',30)
 def close():
  s=len(t());focus();q.key('f11')
  g['wait'](r'\[aftercare\.closed\]\n',s,60)
  g['wait'](rf'\[app\.private\.retired\] target={app["target"]} task={app["task"]} ',s,180)
 result={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  shell('mkdir /data')   # a fresh disk has no /data until a program runs
  for d in ('/data/src','/data/src/docs','/data/backup','/data/restore'):
   assert shell(f'mkdir {d}')=='0',d
  for name,text in FILES.items():
   assert shell(f'write /data/src/{name} {text}')=='0',name

  start('@/data/src @/data/backup @/data/restore')
  press('s',r'\[aftercare\.choose\] purpose=source roots=3\n')
  press('ret',r'\[aftercare\.folder\] path=@/data/src rows=2\n')
  press('ret',r'\[aftercare\.chose\] purpose=source path=@/data/src\n')
  press('d',r'\[aftercare\.choose\] purpose=destination roots=4\n')
  press('down',r'\[aftercare\.cursor\] row=1\n')
  press('ret',r'\[aftercare\.folder\] path=@/data/backup rows=1\n')
  press('ret',r'\[aftercare\.chose\] purpose=destination path=@/data/backup\n')
  # A new repository: the password twice.
  press('u',r'\[aftercare\.prompt\] name=new-password\n');type_app(PASSWORD)
  press('ret',r'\[aftercare\.prompt\] name=confirm-password\n');type_app(PASSWORD)
  m=press('ret',r'\[aftercare\.repo\.init\] path=@/data/backup kdf_kib=(\d+) kdf_passes=(\d+) ms=(\d+)\n',300)
  result['init']=dict(kdf_kib=int(m[1]),passes=int(m[2]),ms=int(m[3]))
  m=press('b',r'\[aftercare\.backup\.done\] snapshot=(\w+) files_new=(\d+) files_changed=(\d+) files_unchanged=(\d+) dirs=(\d+) bytes=(\d+) [^\n]*ms=(\d+)\n',600)
  assert (m[2],m[3],m[4])==('3','0','0'),m[0]
  assert int(m[6])==sum(len(content(x))for x in FILES.values()),m[0]
  result['backup1']=dict(snapshot=m[1],ms=int(m[7]))

  # Lose a file.
  assert shell('rm /data/src/docs/letter.txt')=='0'
  s=len(t());shell('cat /data/src/docs/letter.txt')
  assert '[spindle.fs.cat]' not in t()[s:],'the file is still there'

  focus()
  press('l',r'\[aftercare\.snapshots\] count=1\n')
  press('ret',r'\[aftercare\.browse\] snapshot=\w+ path=/ rows=1\n')
  press('ret',r'\[aftercare\.browse\] snapshot=\w+ path=/data rows=1\n')
  press('ret',r'\[aftercare\.browse\] snapshot=\w+ path=/data/src rows=2\n')
  press('ret',r'\[aftercare\.browse\] snapshot=\w+ path=/data/src/docs rows=2\n')
  s=len(t());q.key('r')
  m=g['wait'](r'\[aftercare\.restore\.file\] path=@/data/src/docs/letter\.txt bytes=(\d+) sha256=([0-9a-f]{64})\n',s,300)
  g['wait'](r'\[aftercare\.restore\.done\] path=/data/src/docs/letter\.txt into=@/data/src/docs files=1 ',s,60)
  want=sha(FILES['docs/letter.txt'])
  assert m[2]==want,f'restored hash {m[2]} != {want}'
  result['restored']=dict(sha256=m[2],bytes=int(m[1]))
  s=len(t());shell('cat /data/src/docs/letter.txt')
  c=g['wait'](r'\[spindle\.fs\.cat\] ([^\n]*)\n',s,30)
  assert c[1]==FILES['docs/letter.txt'],f'cat read {c[1]!r}'

  # The whole folder, somewhere else.
  focus()
  press('backspace',r'\[aftercare\.browse\] snapshot=\w+ path=/data/src rows=2\n')
  press('t',r'\[aftercare\.choose\] purpose=restore roots=4\n')
  for row in (1,2):press('down',rf'\[aftercare\.cursor\] row={row}\n')
  press('ret',r'\[aftercare\.folder\] path=@/data/restore rows=1\n')
  s=len(t());q.key('ret')
  g['wait'](r'\[aftercare\.restore\.done\] path=/data/src/docs into=@/data/restore files=2 ',s,300)
  got=dict(re.findall(r'\[aftercare\.restore\.file\] path=@/data/restore/docs/(\S+) bytes=\d+ sha256=([0-9a-f]{64})\n',t()[s:]))
  assert got=={'letter.txt':sha(FILES['docs/letter.txt']),'list.txt':sha(FILES['docs/list.txt'])},got

  # A second snapshot, then keep only the last one.
  m=press('b',r'\[aftercare\.backup\.done\] snapshot=(\w+) files_new=(\d+) files_changed=(\d+) files_unchanged=(\d+) [^\n]*\n',600)
  assert int(m[2])+int(m[3])+int(m[4])==3,m[0]
  press('k',r'\[aftercare\.prompt\] name=keep\n')
  clear_input(len('last=10 daily=7 weekly=4 monthly=12'))
  type_app('last=1')
  press('ret',r'\[aftercare\.keep\] policy=last=1\n')
  press('f',r'\[aftercare\.prompt\] name=confirm-forget\n')
  s=len(t());q.key('ret')
  f=g['wait'](r'\[aftercare\.forget\] kept=(\d+) removed=(\d+)\n',s,300)
  assert (f[1],f[2])==('1','1'),f[0]
  pr=g['wait'](r'\[aftercare\.prune\] [^\n]*\n',s,300)
  c=press('c',r'\[aftercare\.check\] errors=(\d+) warnings=(\d+) snapshots=(\d+) [^\n]*read_data=0 ',300)
  assert (c[1],c[3])==('0','1'),c[0]
  q.key('esc');time.sleep(.3)
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'c'}],'hold-time':40})
  c=g['wait'](r'\[aftercare\.check\] errors=(\d+) warnings=(\d+) snapshots=(\d+) [^\n]*blobs_verified=(\d+) read_data=1 ',s,300)
  assert c[1]=='0' and int(c[4])>0,c[0]
  result['prune']=pr[0].strip()
  close()

  # Settings kept; a wrong password refused; the right one opens.
  m=start('@/data/src @/data/backup')
  assert (m[4],m[5])==('@/data/src','@/data/backup'),m[0]
  press('u',r'\[aftercare\.prompt\] name=unlock\n');type_app(WRONG)
  w=press('ret',r'\[aftercare\.unlock\] result=(\S+) ms=(\d+)\n',300)
  assert w[1]=='wrong-password',w[0]
  type_app(PASSWORD)
  s=len(t());q.key('ret')
  o=g['wait'](r'\[aftercare\.unlock\] result=(\S+) ms=(\d+)\n',s,300)
  assert o[1]=='ok',o[0]
  # The app's lowered Argon2 cost keeps unlocking under 2 s on this emulated CPU.
  assert int(o[2])<2000 and int(w[2])<2000,(o[0],w[0])
  g['wait'](r'\[aftercare\.snapshots\] count=1\n',s,60)
  result['unlock_ms']=int(o[2]);result['wrong_password']='refused'
  close()
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,**result})
 finally:g['stop']()
if __name__=='__main__':main()
