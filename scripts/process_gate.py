#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Programs run programs through the standard libraries (crates/sex-std/src/process.rs,
crates/sex-libc/src/posix_process.rs), over sex_pdx::launch.

Each run is typed into Spindle as a person would:

* `run stdspawn`: Rust `std::process::Command` (the test program apps/stdspawn starts
  copies of itself): a child that exits with 7 gives status 7; `/bin/NAME`; arguments
  with blanks and quotes arrive whole; `try_wait` is None before a child ends and
  `wait` then gives success; a long child is killed (no code, not success); a missing
  program is NotFound; asking for the network from a run without it is refused by
  Shell (PermissionDenied); piped streams and `output()` are Unsupported, `output()`
  with null streams works; a windowed child runs too.
* `run +net stdspawn net`: a run that has the network may hand it on.
* `run chello spawn`: C `system()` (wait status 7, quoted words, 127 for a missing
  program), `posix_spawnp` + `waitpid(WNOHANG)` + `kill(SIGKILL)` + `waitpid`
  (WIFSIGNALED), ECHILD after reaping, `posix_spawn` + `waitpid` (status 5), ENOENT,
  ENOSYS for file actions.

Hidden children never open a window and never wait to be dismissed; every check must
pass, Shell's log must show the refused network and the kill, and each parent must
end with status 0."""
import argparse,os,re,shutil,time
from pathlib import Path
from install_program import install
os.environ.setdefault('SEXOS_GATE_NIC','-netdev user,id=n0 -device e1000e,netdev=n0')
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 elfs=[(n,(ROOT/'iso_root/programs'/n).read_bytes()) for n in ['stdspawn','chello']]
 assert all(e[:4]==b'\x7fELF' for _,e in elfs)
 if b'This build has no C compiler output (musl headers were missing).' in dict(elfs)['chello']:
  raise SystemExit('chello is the no-compiler fallback: install musl headers or set MUSL_INCLUDE, then rebuild before running this gate')
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,elfs)
 g=boot(a.iso,disk,a.out,'process',a.resolution);t=g['text'];q=g['q']
 def run(line):
  g['focus'](153);time.sleep(.5)
  for k in line:
   if k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key({' ':'spc','.':'dot'}.get(k,k))
   time.sleep(.05)
  q.key('ret')
 def scored(line,score,least):
  s=len(t());run(line)
  m=g['wait'](score,s,900)
  if m[1]!='ok':
   time.sleep(1);raise AssertionError(f'{line}: checks failed:\n'+''.join(re.findall(r'\[(?:stdspawn\.check\] FAIL|std\.out\] FAIL)[^\n]*\n',t()[s:])))
  assert int(m[3])>=least and m[2]==m[3],f'not every check ran: {m[0]}'
  e=g['wait'](r'\[std\.exit\] target=(\d+) task=(\d+) status=0\n',s+m.end(),120)
  return s,dict(line=line,target=int(e[1]),task=int(e[2]),passed=int(m[2]))
 def close(r):
  s=len(t());g['focus'](SURFACE[r['target']]);time.sleep(.5);q.key('f11')
  g['wait'](rf'\[app\.private\.retired\] target={r["target"]} task={r["task"]} ',s,180)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  runs=[]
  s,r=scored('run stdspawn',r'\[stdspawn\.(ok|fail)\] passed=(\d+)/(\d+)\n',13)
  log=t()[s:]
  assert re.search(r'\[shell\.launch\.denied\] parent=\d+ cookie=\d+ reason=network',log),'Shell did not refuse the network'
  assert re.search(r'\[shell\.launch\.kill\] parent=\d+ child=\d+ task=\d+ ok=1',log),'Shell did not end the killed child'
  assert re.search(r'\[shell\.launch\.done\] parent=\d+ child=\d+ task=\d+ status=Some\(7\)',log),'Shell did not report status 7'
  assert re.search(r'\[shell\.launch\.done\] parent=\d+ child=\d+ task=\d+ status=None',log),'Shell did not report the killed child'
  assert '[stdspawn.child] args=["exit", "7"] exit=7' in log
  # Hidden helpers' output stays in the serial record; they never wait to be closed.
  assert '[std.out] sleeping' in log
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'stdspawn.ppm')})
  close(r);runs.append(r)
  s=len(t());run('run +net stdspawn net')
  m=g['wait'](r'\[stdspawn\.net\] (ok|fail) ([^\n]*)\n',s,600)
  assert m[1]=='ok',f'a run with the network could not hand it on: {m[2]}'
  e=g['wait'](r'\[std\.exit\] target=(\d+) task=(\d+) status=0\n',s+m.end(),120)
  r=dict(line='run +net stdspawn net',target=int(e[1]),task=int(e[2]));close(r);runs.append(r)
  s,r=scored('run chello spawn',r'\[chello\.spawn\.(ok|fail)\] passed=(\d+)/(\d+)\n',13)
  log=t()[s:]
  assert re.search(r'\[shell\.launch\.kill\] parent=\d+ child=\d+ task=\d+ ok=1',log),'Shell did not end the C program\'s child'
  assert '[stdspawn.child] args=["exit", "5"] exit=5' in log
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'chello-spawn.ppm')})
  close(r);runs.append(r)
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'runs':runs})
 finally:g['stop']()
if __name__=='__main__':main()
