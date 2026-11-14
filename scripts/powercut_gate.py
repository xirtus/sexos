#!/usr/bin/env python3
"""Power cuts on the running system: QEMU is killed while Files is writing, again and
again (POWERCUT_CYCLES, default 3; the plan's full run is 100). Each cycle first writes a
file and commits it (a folder made after it commits what came before), then starts more
writes and is killed at a random moment among them. After every cut the disk must check
clean with `sexfs-fsck --deep`, and the next boot must read every committed file back.

Simulated cuts at every write, with lost and torn writes, run on the host in
`cargo test -p sexfs --features repair powercut` and `volumes_host_gate.sh power_cuts`."""
import argparse,os,random,shutil,subprocess,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 shutil.rmtree(a.out,ignore_errors=True);a.out.mkdir(parents=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 subprocess.run(['cargo','build','-q','--release','--manifest-path',str(ROOT/'tools/sexfs-fsck/Cargo.toml')],check=True)
 tool=ROOT/'tools/sexfs-fsck/target/release/sexfs-fsck'
 cycles=int(os.environ.get('POWERCUT_CYCLES','3'));rng=random.Random(int(os.environ.get('POWERCUT_SEED','5')))
 from quil_autosave_gate import boot
 committed=[];result={'cycles':cycles,'kills':[]}
 for n in range(cycles+1):
  tag=f'cut{n}';g=boot(a.iso,disk,a.out,tag,a.resolution);t=g['text'];q=g['q']
  def keys(text):
   for k in text:
    code=KEYS.get(k,k.lower())
    if k.isupper():q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':code}],'hold-time':80})
    else:q.key(code)
    time.sleep(.04)
  def line(text):g['focus'](153);time.sleep(.3);keys(text);q.key('ret')
  def expect(text,pattern,timeout=60):s=len(t());line(text);return g['wait'](pattern,s,timeout)
  try:
   g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
   # Spindle can drop the first keys while it opens: the first command is retried.
   for attempt in range(3):
    try:expect('mkdir pc',r'\[spindle.fs.result\] kind=3 status=(0|3)',30);break
    except TimeoutError:
     if attempt==2:raise
   # Everything committed before every earlier cut is there.
   for name,text in committed:expect(f'cat pc/{name}',rf'\[spindle.fs.cat\] {text}',60)
   if n==cycles:break
   name,text=f'c{n}.txt',f'cycle {n} kept'
   expect(f'write pc/{name} {text}',r'\[spindle.fs.result\] kind=8 status=0')
   expect(f'mkdir pc/done{n}',r'\[spindle.fs.result\] kind=3 status=0')
   committed.append((name,text))
   # More writes, cut off at a random moment among them.
   delay=rng.uniform(.2,4.0);result['kills'].append(round(delay,2))
   g['focus'](153)
   for k in range(6):
    keys(f'write pc/w{n}-{k}.txt filler {k} for the cut');q.key('ret')
    if k==2:time.sleep(delay);break
  finally:g['stop']()
  r=subprocess.run([str(tool),'--deep',str(disk)],capture_output=True,text=True)
  print(f'cycle {n}: killed after {result["kills"][-1] if n<cycles else "-"} s; sexfs-fsck: {r.stdout.strip()}')
  assert r.returncode==0,f'cycle {n}: the disk is not clean after the cut:\n{r.stdout}{r.stderr}'
 result['ok']=True;print(result)
if __name__=='__main__':main()
