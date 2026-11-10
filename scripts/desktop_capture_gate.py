#!/usr/bin/env python3
"""Actual person-triggered native PNG, saved by Spindle and extracted from SexFS.
QMP screenshots are external comparison evidence; the guest never gets scanout.
"""
import argparse,hashlib,json,re,shutil,subprocess,time
from pathlib import Path
from PIL import Image
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def main():
 p=argparse.ArgumentParser();[p.add_argument('--'+n,required=True,type=Path)for n in('iso','source-disk','out')];p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True)
 if any(a.out.iterdir()):raise RuntimeError('fresh evidence directory required')
 disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b')as f:f.truncate(1024**3)
 width,height=map(int,a.resolution.split('x'));assert width*height<=(16*1024*1024-4096)//4
 extractor=a.out/'sexfs-extract';subprocess.run(['rustc','--edition=2021','-O','-A','dead_code','-A','unused_attributes',str(ROOT/'scripts/sexfs_extract.rs'),'-o',str(extractor)],check=True)
 g=boot(a.iso.resolve(),disk,a.out,'capture',a.resolution+(',vgamem_mb=64'if width>1280 else''));text=g['text'];q=g['q'];failure=None
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,180);g['focus'](153);time.sleep(1)
  start=len(text())
  for key in 'capture shot.png':q.key({' ':'spc','.':'dot'}.get(key,key));time.sleep(.06)
  q.key('ret')
  ready=g['wait'](r'\[display.capture.ready\] target=12 task=(\d+) sequence=(\d+) epoch=(\d+) generation=(\d+) width=(\d+) height=(\d+) native=1 night=(\d+) bytes=(\d+)',start,120)
  assert tuple(map(int,(ready[5],ready[6])))==(width,height),ready[0]
  # Freeze only the external test VM, after actual publication/copy. The saved
  # PNG still comes exclusively from Spindle's capability-bound capture bank.
  q.cmd('stop')
  try:q.cmd('screendump',{'filename':str(a.out/'published.ppm')})
  finally:q.cmd('cont')
  saved=g['wait'](rf'\[spindle.capture.saved\] task={ready[1]} sequence={ready[2]} epoch={ready[3]} bytes=(\d+) path=/shot.png synced=1 released=1',start,240)
  g['wait'](rf'\[display.capture.released\] target=12 task={ready[1]} sequence={ready[2]} scrubbed=1',start,60)
  assert 'KERNEL PANIC'not in text()and 'serial.record.overflow'not in text()
  proof=dict(ok=True,mode=a.resolution,epoch=int(ready[3]),generation=int(ready[4]),png_bytes=int(saved[1]),iso_sha256=sha(a.iso),capture_runner_sha256=sha(Path(__file__)),helper_sha256=sha(ROOT/'scripts/quil_autosave_gate.py'),extractor_source_sha256=sha(ROOT/'scripts/sexfs_extract.rs'),fs_source_sha256=sha(ROOT/'crates/sexfs/src/lib.rs'))
 except Exception as e:failure=repr(e);proof=dict(ok=False,error=failure,iso_sha256=sha(a.iso))
 finally:g['stop']()
 if failure:
  (a.out/'result.json').write_text(json.dumps(proof,indent=2)+'\n');raise RuntimeError(failure)
 png=a.out/'shot.png';subprocess.run([str(extractor),str(disk),'/shot.png',str(png)],check=True)
 with Image.open(png)as img:assert img.format=='PNG';assert img.size==(width,height);pixels=img.convert('RGB').tobytes()
 with Image.open(a.out/'published.ppm')as img:assert img.size==(width,height);scanout=img.convert('RGB').tobytes()
 # The independent serial poll can trail a clock repaint. Exclude only the
 # global bar from this comparison; compare the complete remaining desktop.
 begin=width*64*3;total=(width*height*3-begin)//3
 matched=sum(pixels[i:i+3]==scanout[i:i+3]for i in range(begin,len(pixels),3));ratio=matched/total
 assert ratio>=.99,dict(matched=matched,total=total,ratio=ratio)
 assert len(set(tuple(pixels[i:i+3])for i in range(0,len(pixels),3)))>16,'blank or placeholder PNG'
 proof.update(png_sha256=sha(png),scanout_sha256=sha(a.out/'published.ppm'),matching_pixels=matched,compared_pixels=total,match_ratio=ratio)
 (a.out/'result.json').write_text(json.dumps(proof,indent=2)+'\n');print(proof)
if __name__=='__main__':main()
