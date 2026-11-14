#!/usr/bin/env python3
"""Assuit, the word processor (WordCraft's engine, built for x86_64-sex-std.json):

* `run assuit` writes welcome.docx (hand-written WordprocessingML, apps/assuit/make_welcome.py)
  into its folder, opens it and draws page 1 with ink on it.
* End, then typing " today" adds to the title; the record shows the new first line.
* Shift+Home and Ctrl+B run WordCraft's caret.home (extending) and format.bold.
* Ctrl+S writes the docx; the window closes.
* A second `run assuit` reads the saved file back with the edit in its first line.

No kernel panic, page fault or std abort anywhere in the record."""
import argparse,os,re,shutil,time
from pathlib import Path
from install_program import install
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 elf=(ROOT/'iso_root/programs/assuit').read_bytes();assert elf[:4]==b'\x7fELF'
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('assuit',elf)])
 g=boot(a.iso,disk,a.out,'assuit',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  for k in line:
   q.key({' ':'spc','.':'dot'}.get(k,k));time.sleep(.05)
 def opened():
  s=len(t());g['focus'](153);time.sleep(.5);type_line('run assuit');q.key('ret')
  o=g['wait'](r'\[assuit\.open\] target=(\d+) task=(\d+) name=welcome\.docx pages=1\n',s,900)
  m=g['wait'](r'\[assuit\.page\] page=1 pages=1 width=(\d+) height=(\d+) ink=(\d+) ms=(\d+)\n',s,900)
  assert int(m[3])>3000,f'page 1 looks empty: {m[0]}'
  g['focus'](SURFACE[int(o[1])]);time.sleep(.8)
  return o
 def close(o):
  s=len(t());q.key('f11');g['wait'](r'\[assuit\.closed\]\n',s,120)
  g['wait'](rf'\[app\.private\.retired\] target={o[1]} task={o[2]} ',s,180)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  o=opened()
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'assuit-opened.ppm')})
  s=len(t());q.key('end');type_line(' today')
  g['wait'](r'\[assuit\.text\] chars=\d+ first=Welcome to Assuit today\n',s,300)
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'home'}],'hold-time':80})
  g['chord']('ctrl','b',r'\[assuit\.command\] format\.bold\n')
  m=g['chord']('ctrl','s',r'\[assuit\.saved\] name=welcome\.docx bytes=(\d+)\n')
  assert int(m[1])>2000,f'saved file is too small: {m[0]}'
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'assuit-edited.ppm')})
  close(o)
  o=opened()
  s=len(t());q.key('end');q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'1'}],'hold-time':80})
  g['wait'](r'\[assuit\.text\] chars=\d+ first=Welcome to Assuit today!\n',s,300)
  close(o)
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'saved_bytes':int(m[1])})
 finally:g['stop']()
if __name__=='__main__':main()
