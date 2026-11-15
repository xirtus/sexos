#!/usr/bin/env python3
"""Polyamour, the 3D suite (Blender's workspace, built for x86_64-sex-std.json):

* `run polyamour` opens the window on the Layout workspace with the startup scene.
* Ctrl+Page Down switches to Modeling and Ctrl+Page Up back to Layout (workspace tabs).
* Ctrl+Shift+R (F12's stand-in: the Shell keeps F12) path-traces the startup scene with sex-3d and shows the Rendering workspace.
* Ctrl+S saves the scene as untitled.poly (polyops text) in the program's folder.
* Ctrl+Page Up twice goes from Rendering through Animation to Shading, whose Shader Editor shows the startup material's node tree.
* F11 closes it.

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
 elf=(ROOT/'iso_root/programs/polyamour').read_bytes();assert elf[:4]==b'\x7fELF'
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('polyamour',elf)])
 g=boot(a.iso,disk,a.out,'polyamour',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  for k in line:q.key({' ':'spc','.':'dot'}.get(k,k));time.sleep(.05)
 def chord(*keys):
  q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,keys),(False,keys[::-1]))for x in ks]})
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  s=len(t());g['focus'](153);time.sleep(.5);type_line('run polyamour');q.key('ret')
  o=g['wait'](r'\[polyamour\.ready\] target=(\d+) task=(\d+) size=(\d+)x(\d+)\n',s,900)
  assert int(o[3])>=640 and int(o[4])>=400,o[0]
  g['focus'](SURFACE[int(o[1])]);time.sleep(1.5)
  q.cmd('screendump',{'filename':str(a.out/'polyamour-layout.ppm')})
  s=len(t());chord('ctrl','pgdn');g['wait'](r'\[polyamour\.workspace\] Modeling\n',s,120)
  s=len(t());chord('ctrl','pgup');g['wait'](r'\[polyamour\.workspace\] Layout\n',s,120)
  s=len(t());chord('ctrl','shift','r');g['wait'](r'\[polyamour\.op\] render\.render\n',s,120)
  g['wait'](r'\[polyamour\.report\] Rendered 960x540, \d+ samples(?:, \d+ pass(?:es)?)?\n',s,900)
  g['wait'](r'\[polyamour\.workspace\] Rendering\n',s,60)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'polyamour-render.ppm')})
  s=len(t());chord('ctrl','s');g['wait'](r'\[polyamour\.report\] Saved "untitled\.poly"\n',s,120)
  s=len(t());chord('ctrl','pgup');g['wait'](r'\[polyamour\.workspace\] Animation\n',s,120)
  s=len(t());chord('ctrl','pgup');g['wait'](r'\[polyamour\.workspace\] Shading\n',s,120)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'polyamour-shading.ppm')})
  s=len(t());q.key('f11');g['wait'](rf'\[app\.private\.retired\] target={o[1]} task={o[2]} ',s,180)
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True})
 finally:g['stop']()
if __name__=='__main__':main()
