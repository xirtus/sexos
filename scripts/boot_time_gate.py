#!/usr/bin/env python3
"""Boot time (#309): host-side milliseconds from QEMU start to each boot milestone, over
several cold boots, written to boot-times.json. Milestones: kernel entry, the other cores
online, the kernel handing over to services, each driver and desktop service ready (Display,
Shell, Input, USB with its ports enumerated, Bell, ACPI, SexDrive, Files), the boot program
(target 41) done, and the desktop painted with its terminal ([linen.fast_paint] ...
remote_ready, the person can type). Each boot's timeline, in milestone order, is printed so
a CI log shows which service the desktop waited for. It fails only if a boot never reaches
the desktop. Numbers are TCG wall time unless SEXOS_GATE_ACCEL=kvm: compare runs on the
same runner and the same core count, not with real hardware."""
import argparse,json,os,re,shutil,statistics,threading,time
from pathlib import Path
os.environ.setdefault('SEXOS_GATE_BOUND','600')
from quil_autosave_gate import boot
MILESTONES=[
 ('kernel_entry',r'\[boot\.kernel_entry\] tsc='),
 ('cores_online',r'\[smp\.online\] cores='),
 ('services_handed_over',r'init: Ready for Scheduler\.'),
 ('display_ready',r'\[sexdisplay\.ready\]'),
 ('shell_ready',r'\[silk-shell\.ready\]'),
 ('input_ready',r'\[sexinput\.ready\]'),
 ('acpi_ready',r'\[sexacpi\.ready\]'),
 ('bell_ready',r'\[sexbell\.ready\]'),
 ('usb_ready',r'\[sexusb\.ready\]'),
 ('usb_enumerated',r'\[sexusb\.disks\.route\]'),
 ('drive_storage_up',r'\[sexdrive\.boot\.storage\]'),
 ('drive_probed',r'\[sexdrive\.boot\.probed\]'),
 ('drive_frame',r'\[sexdrive\.boot\.frame\]'),
 ('drive_serving',r'\[sexdrive\.boot\.serving\]'),
 ('files_ready',r'\[sexfiles\.ready\]'),
 ('boot_program_done',r'\[app\.private\.retired\] target=41 '),
 ('desktop_ready',r'\[linen\.fast_paint\] sid=200[^\n]*reason=remote_ready'),
]
COMPILED=[(name,re.compile(pattern)) for name,pattern in MILESTONES]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path)
 p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--boots',type=int,default=3)
 a=p.parse_args();a.out.mkdir(parents=True,exist_ok=True);runs=[]
 for n in range(a.boots):
  disk=a.out/f'nvme-{n}.img';shutil.copyfile(a.source_disk,disk)
  serial=a.out/f'boot{n}-serial.log';serial.unlink(missing_ok=True);seen={};done=threading.Event();window=[]
  def watch():
   # The boot helper returns only after Files published; time the log as it grows. Only
   # the new text is read each pass (with the unfinished last line carried over), so the
   # watcher does not compete with a single-core TCG guest for the host's time.
   offset,carry=0,''
   while not done.is_set():
    try:
     with open(serial,'rb') as f:f.seek(offset);chunk=f.read()
    except FileNotFoundError:chunk=b''
    now=round((time.monotonic()-t0)*1000)
    if chunk:
     offset+=len(chunk);text=carry+chunk.decode(errors='replace')
     lines=text.split('\n');carry=lines.pop()
     for line in lines:
      # Every line the kernel prints between the cores coming online and the hand-over
      # to services, with when it arrived: the widest gaps name the slow kernel step.
      if 'cores_online' in seen and 'services_handed_over' not in seen:window.append((now,line))
      for name,pattern in COMPILED:
       if name not in seen and pattern.search(line):seen[name]=now
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
   order=sorted((v,k) for k,v in seen.items() if v is not None)
   gaps=sorted(((window[i+1][0]-window[i][0],i) for i in range(len(window)-1)),reverse=True)[:8]
   for ms,i in gaps:
    if ms>=20:print(f'[boot.kernel_gap] boot={n} ms={ms} lines={len(window)} after={window[i][1][:100]!r} before={window[i+1][1][:100]!r}',flush=True)
   print(f'[boot.timeline] boot={n} cores={os.environ.get("SEXOS_GATE_CORES","1")} accel={os.environ.get("SEXOS_GATE_ACCEL","tcg")} '+' '.join(f'{k}={v}' for v,k in order),flush=True)
  finally:
   done.set()
   if g:g['stop']()
   disk.unlink(missing_ok=True)
 median={name:statistics.median([r[name] for r in runs if r[name] is not None]) if any(r[name] is not None for r in runs) else None for name,_ in MILESTONES}
 (a.out/'boot-times.json').write_text(json.dumps({'runs':runs,'median_ms':median},indent=2))
 print({'ok':True,'cores':os.environ.get('SEXOS_GATE_CORES','1'),'accel':os.environ.get('SEXOS_GATE_ACCEL','tcg'),'median_ms':median})
if __name__=='__main__':main()
