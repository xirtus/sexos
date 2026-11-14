#!/usr/bin/env python3
"""Term, the terminal: a VT100/xterm screen that draws what it was tested to draw, its own
command line over its folder, and a WebAssembly command-line program (Rust std built for
wasm32-wasip1) running behind a terminal stream, reading lines typed into it.

* `vtcheck`: the vttest-style fixture from crates/sex-vt (`check::FIXTURE`) drawn on the
  running system; the screen model, the colours and the drawn pixels must match the
  expectations the host tests prove (frame=1 colors=1 pixels=1).
* `write`, `cat`, `mkdir`, `cd`, `ls`: files in Term's own folder, seen on screen.
* `greet Andreas`: the guest prints, reads words until an empty line, counts them and
  exits 5; its output reaches the screen through the stream.
* Closing the window ends Term; a second run starts clean.
* `run stdhello vt`: an ordinary Rust std program prints escape sequences, so its console
  window becomes the same terminal; the cursor position report it asks for comes back on
  its standard input."""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}
KEYS={' ':'spc','.':'dot','/':'slash','-':'minus','=':'equal',',':'comma',';':'semicolon'}
SHIFTED={'>':'dot',':':'semicolon','_':'minus','!':'1','@':'2'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 term=(ROOT/'iso_root/programs/term').read_bytes();assert term[:4]==b'\x7fELF'
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 std=(ROOT/'iso_root/programs/stdhello').read_bytes()
 install(disk,[('term',term),('stdhello',std)])
 g=boot(a.iso,disk,a.out,'term',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k in SHIFTED or k.isupper():q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':SHIFTED.get(k,k.lower())}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.05)
 def spindle(line):g['focus'](153);time.sleep(.5);keys(line);q.key('ret')
 def term_line(line):keys(line);q.key('ret')
 def snapshot():
  s=len(t());term_line('snapshot');g['wait'](r'\[term\.snapshot\] rows=\d+',s,60)
  return dict((int(r),row) for r,row in re.findall(r'\[term\.row\] r=(\d+) ([^\n]*)\n',t()[s:]))
 def start():
  s=len(t());spindle('run term')
  m=g['wait'](r'\[term\.ready\] target=(\d+) task=(\d+) cols=(\d+) rows=(\d+)',s,240)
  target,task,cols,rows=map(int,m.groups());assert cols>=40 and rows>=10,m[0]
  g['wait'](rf'\[display\.app\.title\] target={target} ',s,60);time.sleep(1)
  g['focus'](SURFACE[target]);time.sleep(.5)
  return target,task
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  target,task=start()
  # The fixture, checked inside the running system against the host-proven screen.
  s=len(t());term_line('vtcheck')
  m=g['wait'](r'\[term\.vtcheck\] frame=(\d) colors=(\d) pixels=(\d) cols=(\d+) rows=(\d+)',s,60)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'term-vtcheck.ppm')})
  assert m.group(1,2,3)==('1','1','1'),m[0]
  s=len(t());term_line('colors');g['wait'](r'\[term\.command\] colors',s,30);time.sleep(1)
  q.cmd('screendump',{'filename':str(a.out/'term-colors.ppm')})
  # Files in Term's folder, through its own commands.
  term_line('clear');term_line('mkdir notes');term_line('cd notes');term_line('write todo.txt buy milk')
  term_line('cat todo.txt');term_line('cd ..');term_line('ls notes')
  rows=snapshot();screen='\n'.join(rows.values())
  assert 'buy milk' in screen and 'todo.txt' in screen and '~/notes >' in screen,screen
  # A Wasm program behind the stream: Rust std's println and stdin.
  s=len(t());term_line('greet Andreas');g['wait'](r'\[term\.job\.start\] name=greet',s,60);time.sleep(3)
  term_line('b a b');time.sleep(1);q.key('ret')
  m=g['wait'](r'\[term\.job\.exit\] name=greet code=(-?\d+)',s,120);assert m[1]=='5',m[0]
  time.sleep(1);rows=snapshot();screen='\n'.join(rows.values())
  for want in ['Hello, Andreas, from Rust on Wasm.','a: 1','b: 2','[greet exited with 5]']:assert want in screen,(want,screen)
  q.cmd('screendump',{'filename':str(a.out/'term-greet.ppm')})
  s=len(t());q.key('f11');g['wait'](r'\[term\.closed\] how=window',s,60);g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s,120)
  # A second run starts clean.
  target,task=start();rows=snapshot();assert any('Type help' in r or 'Type' in r for r in rows.values()),rows
  s=len(t());q.key('f11');g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s,120)
  # A std program's console as a terminal.
  s=len(t());spindle('run stdhello vt')
  g['wait'](r'\[std\.out\] \[stdhello\.vt\] reply=ESC\[3;14R\n',s,300)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'std-console-vt.ppm')})
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[term.fatal]' not in t()
 finally:g['stop']()
 print({'ok':True,'vtcheck':m.group(0) if m else None})
if __name__=='__main__':main()
