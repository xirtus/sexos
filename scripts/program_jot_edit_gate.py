#!/usr/bin/env python3
"""Jot as an editor: the undo tree, save and reopen, find and replace, tabs, syntax
colouring and spelling, in the running system.

* Five words typed are five undo steps: undo all five, redo two, save; the file read back
  after Jot is closed and started again has the same bytes (FNV-1a hash in the records).
* Ctrl+F finds a word; Ctrl+H replaces every match in one step.
* A misspelt word is marked; Ctrl+. replaces it with the dictionary's first suggestion.
* Ctrl+N opens a second tab and Ctrl+PageUp goes back.
* `edit main.rs` colours Rust (screenshot)."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}
KEYS={' ':'spc','.':'dot','/':'slash','-':'minus','=':'equal',',':'comma',';':'semicolon','\n':'ret'}
SHIFTED={'(':'9',')':'0','{':'bracket_left','}':'bracket_right','!':'1','"':'apostrophe',':':'semicolon'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 jot=(ROOT/'iso_root/programs/jot').read_bytes();assert jot[:4]==b'\x7fELF'
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[('jot',jot)])
 g=boot(a.iso,disk,a.out,'jotedit',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k in SHIFTED or k.isupper():q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':SHIFTED.get(k,k.lower())}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.05)
 def chord(*ks):q.cmd('send-key',{'keys':[{'type':'qcode','data':k} for k in ks],'hold-time':80});time.sleep(.15)
 def spindle(line):g['focus'](153);time.sleep(.5);keys(line);q.key('ret')
 def start(line='run jot'):
  s=len(t());spindle(line)
  m=g['wait'](r'\[shell\.program\.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,240)
  loaded=g['wait'](r'\[jot\.loaded\] bytes=(\d+)(?: fnv=([0-9a-f]+))?',s,120)
  g['wait'](r'\[jot\.ready\] target=\d+ task=\d+ tabs=\d+',s,120);time.sleep(1.5)
  g['focus'](int(m[3]));time.sleep(.5)
  return int(m[1]),int(m[2]),loaded
 def close(target,task):
  s=len(t());q.key('f11');g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s,120)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  target,task,loaded=start();assert loaded[1]=='0',loaded[0]
  s=len(t());keys('one two three four five ')
  for _ in range(5):chord('ctrl','z')
  g['wait'](r'\[jot\.undo\] depth=0 bytes=0\n',s,60)
  chord('ctrl','y');chord('ctrl','y')
  g['wait'](r'\[jot\.redo\] depth=2 bytes=8\n',s,60)
  s=len(t());chord('ctrl','s');saved=g['wait'](r'\[jot\.saved\] bytes=8 fnv=([0-9a-f]+)\n',s,60)
  # Find, then replace all.
  s=len(t());chord('ctrl','f');keys('two');q.key('ret');g['wait'](r'\[jot\.find\] term=two found=1 line=1\n',s,60);q.key('esc')
  s=len(t());chord('ctrl','h');keys('o');q.key('ret');keys('0');chord('ctrl','a');g['wait'](r'\[jot\.replaced\] n=2\n',s,60);q.key('esc')
  # Spelling: a misspelt word, fixed with F7.
  s=len(t());chord('ctrl','end');keys('\nrecieve ');time.sleep(2);q.key('left');chord('ctrl','dot')
  fixed=g['wait'](r'\[jot\.spell\.fixed\] from=recieve to=(\S+) others=\d+\n',s,240);assert fixed[1]=='receive',fixed[0]
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'jot-spelling.ppm')})
  # Tabs.
  s=len(t());chord('ctrl','n');g['wait'](r'\[jot\.tab\] index=1 of=2 name=untitled\.txt\n',s,60)
  chord('ctrl','pgup');g['wait'](r'\[jot\.tab\] index=0 of=2 name=notes\.txt\n',s,60)
  close(target,task)
  # Reopened, the saved bytes come back exactly.
  target,task,loaded=start();assert (loaded[1],loaded[2])==('8',saved[1]),(loaded[0],saved[0])
  close(target,task)
  # Rust, coloured.
  s=len(t());spindle('write main.rs fn main() { let x = 42; } // hi');g['wait'](r'\[spindle\.fs\.result\] kind=8 status=0',s,60)
  target,task,loaded=start('edit main.rs')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'jot-rust.ppm')})
  close(target,task)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[jot.fatal]' not in t()
 finally:g['stop']()
 print({'ok':True,'saved':saved[0].strip(),'spelling':fixed[0].strip()})
if __name__=='__main__':main()
