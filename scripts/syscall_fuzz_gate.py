#!/usr/bin/env python3
"""Random syscalls and pdx calls from a program stop nothing: `run fuzz CALLS SEED`
makes CALLS seeded calls (every syscall number but those that end or reshape the caller,
and pdx calls to any slot with any operation and arguments). The kernel must not stop,
no service may fault, the fuzzer itself must finish and report, and the desktop must
still answer afterwards (Spindle runs a command, a program starts and closes).
Protocol seeds (`run fuzz CALLS SEED proto`) send every operation number sex-pdx names
to the services this program holds, with mutated arguments. A long run for the nightly
soak: --calls 1000000 --seeds 1 --proto-seeds 2."""
import argparse,hashlib,json,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
BUSY_SOURCE_HASHES={
 'apps/spindle/src/main.rs':'57b2800931322a6f34d8867402e47482deec907679d800f35536709c498b5e1b',
 'apps/spindle/src/session_command.rs':'9bce889a22db1f6c80557f0ba0fc0f6a55103f1243e87ea016e3dcc02a0dc7f0',
}
BUSY_TEXT=b'The disk is saving the session for a moment'
SIDE_EFFECT=re.compile(r'\[(?:fuzz\.start|shell\.program\.launch|shell\.launch\.asked|files\.program\.(?:read|bound)|app\.(?:program\.loaded|activation\.committed)|spindle\.session\.started)\]')

def busy_sources(root):
 for name,digest in BUSY_SOURCE_HASHES.items():
  assert hashlib.sha256((root/name).read_bytes()).hexdigest()==digest,'unreviewed Spindle busy admission source: '+name
 return dict(BUSY_SOURCE_HASHES)

def busy_refusal(trace,line):
 """Only the source-pinned pre-dispatch refusal permits retry, never a launch."""
 echoes=list(re.finditer(r'^\[spindle.line.enter\] len=\d+ mode=insert text=(.*)$',trace,re.M))
 if not echoes:return None
 if len(echoes)!=1 or echoes[0][1]!=json.dumps(line):raise ValueError('ambiguous command attempt')
 outputs=list(re.finditer(r'^\[spindle.cmd.output\] name=run bytes=(\d+)$',trace,re.M))
 if len(outputs)!=1 or outputs[0][1]!='84' or outputs[0].start()>echoes[0].start():return None
 # The short tiled viewport wraps/clips the final sentence. Require the full
 # visible fixed busy clause in a complete, exact, current terminal frame.
 for frame in re.finditer(r'^\[spindle.view.text\] target=12 sid=154 sequence=(\d+) paint=(\d+) cols=\d+ rows=\d+ bytes=(\d+)$',trace[echoes[0].end():],re.M):
  at=echoes[0].end()+frame.end();seq,paint,size=map(int,frame.groups())
  if size>65536:raise ValueError('oversized terminal receipt')
  chunks=list(re.finditer(r'^\[spindle.view.bytes\] target=12 sid=154 seq='+str(seq)+r' paint='+str(paint)+r' off=(\d+) hex=([0-9a-f]+)$',trace[at:],re.M))
  data=bytearray();last=at
  for chunk in chunks:
   if int(chunk[1])!=len(data):raise ValueError('duplicate or discontinuous terminal receipt')
   data.extend(bytes.fromhex(chunk[2]));last=at+chunk.end()
  if len(data)!=size:continue
  if BUSY_TEXT in b' '.join(data.split()):
   if SIDE_EFFECT.search(trace):raise ValueError('command has admission side effects; cannot retry')
   return last
 return None

def wait_fuzz_start(g,line,calls,seed,send,*,timeout=120,clock=time.monotonic,pause=time.sleep):
 deadline=clock()+timeout;retries=0;start=len(g['text']());send(line)
 pattern=re.compile(r'\[fuzz.start\] target=(\d+) task=(\d+) calls='+str(calls)+' seed='+str(seed)+r' proto=([01]) ops=\d+')
 while clock()<deadline:
  trace=g['text']()[start:];started=pattern.search(trace)
  if started:
   assert int(started[3])==int(line.endswith(' proto')),'wrong fuzz mode'
   return started,start,retries
  refused=busy_refusal(trace,line)
  if refused is not None:
   saved=re.search(r'\[spindle.session.saved\] bytes=0 status=0(?: programs=\d+)?(?:\n|$)',trace[refused:])
   if saved:
    # Recheck the entire attempt, including the save interval, before resending.
    assert not SIDE_EFFECT.search(trace),'late admission prevents retry'
    assert retries<2,'repeated session-busy refusal'
    retries+=1;start=len(g['text']());send(line);continue
  pause(.05)
 raise TimeoutError('fuzz start admission deadline: '+line)

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800')
 p.add_argument('--calls',type=int,default=20000);p.add_argument('--seeds',default='1,2,3');p.add_argument('--proto-seeds',default='4');a=p.parse_args()
 busy_sources(ROOT)
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 elf=ROOT/'iso_root/programs/fuzz';assert elf.read_bytes()[:4]==b'\x7fELF'
 install(disk,[('fuzz',elf.read_bytes())])
 g=boot(a.iso,disk,a.out,'fuzz',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 runs=[]
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  plan=[(x,False)for x in a.seeds.split(',')if x]+[(x,True)for x in a.proto_seeds.split(',')if x]
  for seed,proto in plan:
   command=f'run fuzz {a.calls} {seed}'+(' proto'if proto else'')
   m,s,retries=wait_fuzz_start(g,command,a.calls,seed,run)
   d=g['wait'](rf'\[fuzz.done\] target={m[1]} task={m[2]} calls=(\d+) ipc=(\d+) ok=(\d+) seed={seed}',s,max(900,a.calls//20000*120))
   run_text=t()[s:];answers=len(re.findall(r'\[fuzz\.answer\] value=',run_text))
   ops=re.search(r'\[fuzz\.ops\] covered=(\d+) of=(\d+) other_answers=(\d+)',run_text)
   runs.append(dict(seed=int(seed),proto=proto,admission_retries=retries,calls=int(d[1]),ipc=int(d[2]),ok=int(d[3]),answer_kinds=answers,ops_covered=f'{ops[1]}/{ops[2]}'if ops else None))
   if proto:assert ops and int(ops[1])*10>=int(ops[2])*9,f'protocol run covered too few operations: {ops and ops.group(0)}'
   # Close it the normal way: Shell and the services still answer.
   time.sleep(1);surface=int(re.findall(rf'\[shell.program.launch\] target={m[1]} task={m[2]} surface=(\d+)',t())[-1])
   s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11');g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s,90)
  text=t()
  services=[l for l in re.findall(r'fault.kill \[[^\]]+\] pd=(\d+)',text) if int(l)<=15]
  assert not services,f'services faulted: {services}'
  assert '[fuzz.fatal]' not in text and 'KERNEL PANIC' not in text,'the fuzzer or the kernel stopped'
  # The desktop still works: Spindle answers a command.
  s=len(t());run('crashes');g['wait'](r'\[spindle.crashes\] count=\d+',s,60)
  print({'ok':True,'runs':runs,'busy_admission_source_hashes':dict(BUSY_SOURCE_HASHES)})
 finally:g['stop']()
if __name__=='__main__':main()
