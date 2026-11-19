#!/usr/bin/env python3
"""Boot time (#309): host-side milliseconds from QEMU start to each boot milestone, over
several cold boots, written to boot-times.json. Milestones: kernel entry, Shell ready,
SexDrive ready, Files ready, the boot program (target 41) done, and the desktop painted
with its terminal ([linen.fast_paint] ... remote_ready, the person can type). It fails
only if a boot never reaches the desktop. Numbers are TCG wall time: compare runs on the
same runner, not with real hardware."""
import argparse,json,os,re,shutil,statistics,threading,time
from pathlib import Path
os.environ.setdefault('SEXOS_GATE_BOUND','600')
from quil_autosave_gate import boot
MILESTONES=[
 ('kernel_entry',r'\[boot\.kernel_entry\] tsc='),
 ('shell_ready',r'\[silk-shell\.ready\]'),
 ('drive_storage_up',r'\[sexdrive\.boot\.storage\]'),
 ('drive_probed',r'\[sexdrive\.boot\.probed\]'),
 ('drive_frame',r'\[sexdrive\.boot\.frame\]'),
 ('drive_serving',r'\[sexdrive\.boot\.serving\]'),
 ('files_ready',r'\[sexfiles\.ready\]'),
 ('boot_program_done',r'\[app\.private\.retired\] target=41 '),
 ('desktop_ready',r'\[linen\.fast_paint\] sid=200[^\n]*reason=remote_ready'),
]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path)
 p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--boots',type=int,default=3)
 a=p.parse_args();a.out.mkdir(parents=True,exist_ok=True);runs=[]
 for n in range(a.boots):
  disk=a.out/f'nvme-{n}.img';shutil.copyfile(a.source_disk,disk)
  serial=a.out/f'boot{n}-serial.log';serial.unlink(missing_ok=True);seen={};done=threading.Event()
  def watch():
   # The boot helper returns only after Files published; time the log as it grows.
   while not done.is_set():
    try:txt=serial.read_text(errors='replace')
    except FileNotFoundError:txt=''
    now=round((time.monotonic()-t0)*1000)
    for name,pattern in MILESTONES:
     if name not in seen and re.search(pattern,txt):seen[name]=now
    # Storage and Files may come up after the desktop: keep timing them (60 s at most).
    if len(seen)==len(MILESTONES) or ('desktop_ready' in seen and now>seen['desktop_ready']+60000):return
    time.sleep(.01)
  t0=time.monotonic();w=threading.Thread(target=watch,daemon=True);w.start();g=None
  try:
   g=boot(a.iso,disk,a.out,f'boot{n}',a.resolution)
   w.join(300);assert 'desktop_ready' in seen,f'boot {n} never reached the desktop'
   for name,_ in MILESTONES:seen.setdefault(name,None)
   assert 'KERNEL PANIC' not in g['text']()
   runs.append(dict(seen))
  finally:
   done.set()
   if g:g['stop']()
   disk.unlink(missing_ok=True)
 median={name:statistics.median([r[name] for r in runs if r[name] is not None]) if any(r[name] is not None for r in runs) else None for name,_ in MILESTONES}
 (a.out/'boot-times.json').write_text(json.dumps({'runs':runs,'median_ms':median},indent=2))
 print({'ok':True,'cores':os.environ.get('SEXOS_GATE_CORES','1'),'median_ms':median})
if __name__=='__main__':main()
