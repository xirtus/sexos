#!/usr/bin/env python3
"""A program file on disk, in no boot image, runs from Spindle: it opens, shows text, closes and runs again, and bad files are refused."""
import argparse,re,shutil,struct,time
from pathlib import Path
import program_launch_gate as launch
from install_program import install,fnv
from quil_autosave_gate import boot
from program_role_facts import ordinary_rows
from quil_workflow_gate import palette_select,palette_commands
from check_desktop_pixels import font_from_source
ROOT=Path(__file__).resolve().parents[1]
FORTUNE=(ROOT/'apps/fortune/src/main.rs').read_text()
# Every program row but the boot image's own.
HELLO_TEXT=launch.TEXT
FORTUNE_TEXT=re.search(r'const TEXT: &\[u8\] = b"(.*?)";',FORTUNE,re.S)[1]
def writable_code(elf):
 bad=bytearray(elf);phoff=struct.unpack_from('<Q',bad,32)[0];n=struct.unpack_from('<H',bad,56)[0]
 for i in range(n):
  at=phoff+i*56;kind,flags=struct.unpack_from('<II',bad,at)
  if kind==1 and flags&1:struct.pack_into('<I',bad,at+4,flags|2);return bytes(bad)
 raise AssertionError('no code segment')
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--runs',type=int,default=5);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elf=(ROOT/'iso_root/programs/fortune').read_bytes();assert elf[:4]==b'\x7fELF'
 install(disk,[('fortune',elf),('badhash',elf,fnv(elf)^1),('wxcode',writable_code(elf))])
 font=font_from_source((ROOT/'servers/sexdisplay/src/main.rs').read_text())
 g=boot(a.iso,disk,a.out,'disk',a.resolution);t=g['text'];q=g['q'];rows=len(ordinary_rows(ROOT,t()));commands=palette_commands(ROOT)
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:
   q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def started(line):
  s=len(t());run(line);m=g['wait'](r'\[fortune.ready\] target=(\d+) task=(\d+) surface=(\d+) bytes=',s,60)
  g['wait'](rf'\[display.text.committed\] target={m[1]} ',s,60);time.sleep(2)
  return m[1],int(m[2]),m[3]
 def drawn(path,sid,text):
  launch.TEXT=text
  tile=re.findall(rf'\[shell.tile.apply\] sid={sid} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t());assert tile,'no tile';x,y,w,h=map(int,tile[-1])
  ink,missing=launch.text_ink(path,(x,y,w,h),font);assert ink>100 and missing==0,f'text ink {ink} missing {missing}'
  return ink
 def close(target,task,word):
  s=len(t());q.key('f11');g['wait'](rf'\[{word}.closed\] target={target} task={task}',s,60)
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,60);time.sleep(1)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  text=FORTUNE_TEXT
  runs=[]
  for _ in range(a.runs):
   target,task,sid=started('run fortune')
   path=a.out/f'fortune-{task}.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(.5)
   runs.append(dict(target=int(target),task=task,ink=drawn(path,sid,text)));close(target,task,'fortune')
  tasks=[r['task']for r in runs];assert len(set(tasks))==len(tasks),'a run reused an old task identity'
  assert len({r['target']for r in runs})<len(runs),'no row was ever reused'
  # Bad files stop at the check, and nothing is left half done.
  for line,evidence in (('run nosuch',r'\[shell.program.denied\] reason=store error=1'),('run badhash',r'\[files.program.corrupt\]'),('run wxcode',r'\[app.program.rejected\][^\n]*reason=Permissions')):
   s=len(t());run(line);g['wait'](evidence,s,60);g['wait'](r'\[shell.program.denied\]',s,60);assert '[fortune.ready]'not in t()[s:],line
  # Disk programs and the boot image's own program share rows.
  palette_select(g['key'],'FocusProgram',commands);s=len(t());q.key('ret')
  m=g['wait'](r'\[hello.ready\] target=(\d+) task=(\d+) surface=(\d+) bytes=',s,60);g['wait'](rf'\[display.text.committed\] target={m[1]} ',s,60);time.sleep(2)
  path=a.out/'hello-after.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(.5);hello=drawn(path,m[3],HELLO_TEXT);close(m[1],int(m[2]),'hello')
  # All rows at once, and one more refused rather than displacing any of them.
  live=[started('run fortune') for _ in range(rows)]
  assert len({row[0]for row in live})==rows,'programs were not on separate rows'
  s=len(t());run('run fortune');g['wait'](r'\[shell.program.denied\] reason=no_free_row',s,60)
  path=a.out/'side-by-side.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(1)
  side=[drawn(path,sid,text) for _,_,sid in live]
  for target,task,sid in reversed(live):
   g['focus'](int(sid));close(target,task,'fortune')
  target,task,sid=started('run fortune');close(target,task,'fortune')
  assert 'KERNEL PANIC'not in t()
 finally:g['stop']()
 # A disk with no store past its end gets a plain answer, not a hang or a fault.
 bare=a.out/'bare.img';shutil.copyfile(a.source_disk,bare)
 g=boot(a.iso,bare,a.out,'bare',a.resolution);t=g['text'];q=g['q'];rows=len(ordinary_rows(ROOT,t()))
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  for attempt in range(2):
   s=len(t());run('run fortune');g['wait'](r'\[shell.program.denied\] reason=store error=\d+',s,60)
  assert '[fortune.ready]'not in t()
 finally:g['stop']()
 print({'ok':True,'runs':runs,'hello_after':hello,'side_by_side':side})
if __name__=='__main__':main()
