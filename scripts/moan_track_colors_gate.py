#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Actual ordered track colors across audio regions, headers and mixer; no new audio claim."""
import sys,os,ast,re,json,shutil,hashlib,time,argparse,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];sys.path.insert(0,str(ROOT/'scripts'))
from moan_workstation_gate import Probe,sha
from moan_gate_geometry import ready,viewport,origin
from quil_autosave_gate import boot
from install_program import install
p=argparse.ArgumentParser(description=__doc__)
for name in('iso','source-disk','out'):p.add_argument('--'+name,type=Path,required=True)
p.add_argument('--label-oracle',type=Path,required=True)
a=p.parse_args();ORACLE=a.label_oracle.resolve();OUT=a.out.resolve();OUT.mkdir(exist_ok=False,parents=True)
inputs={str(p):sha(p)for p in [a.iso,a.source_disk,Path(__file__),ROOT/'scripts/moan_workspace_gate.py',*[ROOT/'iso_root/programs'/n for n in('moan','files')]]}
for d in('apps/moan','crates/moan-core','crates/weft','crates/sex-app','crates/sex-pdx','crates/sex-font','assets/fonts','scripts'):
 for f in(ROOT/d).rglob('*'):
  if f.is_file()and'__pycache__'not in f.parts and'target'not in f.relative_to(ROOT/d).parts:inputs[str(f)]=sha(f)
receipt=json.loads((ORACLE/'receipt.json').read_text())
for name,key in [('Cargo.toml','manifest_sha256'),('Cargo.lock','lock_sha256'),('src/main.rs','source_sha256'),('expected.jsonl','expected_sha256')]:
 assert sha(ORACLE/name)==receipt[key],('oracle receipt mismatch',name)
assert sha(ORACLE/'src/main.rs')==sha(ROOT/'scripts/moan_palette_label_oracle.rs'),'oracle must use explicit reviewed foreground/background generator'
assert all(sha(Path(name))==digest for name,digest in receipt['inputs'].items()),'oracle source/font inputs changed'
required={name:digest for name,digest in inputs.items()if any(name.startswith(str(ROOT/directory)+'/')for directory in('crates/weft','crates/sex-font','assets/fonts'))or name==str(ROOT/'scripts/moan_palette_label_oracle.rs')}
assert receipt['inputs']==required,'oracle receipt must bind complete current Weft/font source closure'
for file in ('Cargo.toml','Cargo.lock','src/main.rs','expected.jsonl','receipt.json'):
 f=ORACLE/file;inputs[str(f)]=sha(f)
(OUT/'inputs.json').write_text(json.dumps(inputs,indent=2)+'\n');(OUT/'source-commit.txt').write_text(subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True))
# Reuse the exact tested physical-input class without executing its gate body.
source=ROOT/'scripts/moan_workspace_gate.py';node=next(n for n in ast.parse(source.read_text()).body if isinstance(n,ast.ClassDef)and n.name=='Driver')
wav=OUT/'unused-audio-capture.wav';ns=dict(Probe=Probe,re=re,time=time,wav=wav,viewport=viewport,origin=origin);exec(compile(ast.Module(body=[node],type_ignores=[]),str(source),'exec'),ns);Driver=ns['Driver']

def park(c,surface,target):
 # Top-left large jumps are deliberately rejected by Shell's tablet guard.
 # A midpoint of the global bar is outside app pixels without that rejection.
 mid=c.rw//2
 accepted=list(re.finditer(r'\[shell.abs.normalize\] raw_x=\d+ raw_y=\d+ sx=(\d+) sy=(\d+) accepted=1 reason=ok',c.text()))
 old=tuple(map(int,accepted[-1].groups()))if accepted else None
 if old not in[(mid-1,9),(mid,9),(mid-1,10),(mid,10)]:
  since=len(c.text());c.move(mid,10)
  c.g['wait'](rf'\[shell.abs.normalize\] raw_x=\d+ raw_y=\d+ sx=(?:{mid-1}|{mid}) sy=(?:9|10) accepted=1 reason=ok',since,60)
 fresh=c.refresh(surface)
 c.g['wait'](rf'\[display.app.pixels\] target={target} surface={surface} w={fresh[2][0]} h={fresh[2][1]}',fresh[0],120)
 return fresh

disk=OUT/'nvme.img';shutil.copyfile(a.source_disk,disk);install(disk,[(n,(ROOT/'iso_root/programs'/n).read_bytes())for n in('moan','files')])
os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0';os.environ['SEXOS_GATE_BOUND']='900'
g=boot(a.iso,disk,OUT,'track-colors','1280x800');shots=[]
ACCENTS=(0x7da6ff,0x55d1e3,0x73d69a,0xd5d76b,0xf1b46b,0xf18a83,0xe58ac8,0xb49aee)
FILLS=(0x384a72,0x265e66,0x336045,0x5f6030,0x6c5130,0x6c3e3a,0x673e5a,0x51456b)
def blend(a,b,n,d):return sum(((((a>>v)&255)*(d-n)+((b>>v)&255)*n)//d)<<v for v in(16,8,0))
try:
 c=Driver(g,OUT,'1280x800');g['wait'](r'\[app.private.retired\] target=41 ',0,180)
 start,target,task,surface=c.launch('moan');g['wait'](r'\[moan.ready\]',start,180);ready(g,surface)
 for i in range(4):
  st=c.command(surface,'ctrl','shift','t');c.command(surface,'ctrl','shift','l');g['wait'](rf'\[moan.project\] tracks={i+1} frames=96000',st,120)
 c.command(surface,'ctrl','t');st,(ox,oy),size=park(c,surface,target)
 m=list(re.finditer(r'\[moan.timeline\] x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) start=0 span=192000',c.text()[st:]))[-1];x,y,w,h=map(int,m.groups());assert h==244
 def region_pixels(width,height,rgb):
  assert(width,height)==(1280,800)
  for i in range(4):
   xx=ox+x+round(w*.2);yy=oy+y+20+i*56+4;actual=int.from_bytes(rgb[(yy*width+xx)*3:(yy*width+xx)*3+3],'big');expected=blend(FILLS[i],0xffffff,1,12)if i==3 else FILLS[i]
   assert actual==expected,('audio region',i,xx,yy,hex(actual),hex(expected))
   xx=ox+x-142+2;yy=oy+y+20+i*56+28;actual=int.from_bytes(rgb[(yy*width+xx)*3:(yy*width+xx)*3+3],'big');assert actual==ACCENTS[i],('header swatch',i,hex(actual),hex(ACCENTS[i]))
   ink=blend(ACCENTS[i],0xffffff,1,5);ink=blend(ink,0xffffff,1,12)if i==3 else ink
   found=0
   for yy in range(oy+y+20+i*56+8,oy+y+20+i*56+48):
    for xx in range(ox+x+8,ox+x+round(w*.45)):
     found+=int.from_bytes(rgb[(yy*width+xx)*3:(yy*width+xx)*3+3],'big')==ink
   assert found>20,('actual waveform ink',i,hex(ink),found)
 shots.append(c.shot(surface,'arrange-four-distinct-audio-tracks',region_pixels,dict(tracks=4,selected=3,region_fills=[hex(v)for v in FILLS[:4]],headers=[hex(v)for v in ACCENTS[:4]],actual_waveform_ink=True)))
 for _ in range(4):c.command(surface,'ctrl','shift','t')
 c.command(surface,'ctrl','m');c.click(surface,'Master');st,(ox,oy),size=park(c,surface,target)
 records=[]
 for i in range(8):
  m=list(re.finditer(rf'\[moan.mixer.track\] track={i} name=volume x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) value=100',c.text()[st:]))[-1];records.append(tuple(map(int,m.groups())))
 def mixer_pixels(width,height,rgb):
  assert len({(x,y)for x,y,_,_ in records})==8
  for i,(x,y,w,h)in enumerate(records):
   assert 0<=x and 0<=y and x+w<=1264 and y+h<=730
   expected=list(map(json.loads,(ORACLE/'expected.jsonl').read_text().splitlines()))[i]
   assert expected['foreground']==hex(ACCENTS[i])and expected['background']=='0x35313b'and expected['text']==f'{i+1} Audio'
   assert(expected['x'],expected['y'],expected['w'],expected['h'])==(x,y-77,w,19)
   value=0xcbf29ce484222325
   for yy in range(oy+expected['y'],oy+expected['y']+expected['h']):
    for byte in rgb[(yy*width+ox+x)*3:(yy*width+ox+x+w)*3]:value=((value^byte)*0x100000001b3)&((1<<64)-1)
   assert value==int(expected['hash'],16),('fullAA mixer label RGB',i,hex(value),expected)
 shots.append(c.shot(surface,'mix-eight-distinct-track-identities',mixer_pixels,dict(tracks=8,accents=[hex(v)for v in ACCENTS],master=True)))
 c.close(surface,target,task,dirty=True);c.q.cmd('quit');g['proc'].wait(30)
finally:g['stop']()
assert all(sha(Path(p))==v for p,v in inputs.items()),'input source mutated'
result=dict(ok=True,scope=__doc__,source_commit=(OUT/'source-commit.txt').read_text().strip(),inputs_sha256=sha(OUT/'inputs.json'),screenshots=shots)
(OUT/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(dict(ok=True,screenshots=len(shots))))
