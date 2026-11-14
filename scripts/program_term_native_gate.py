#!/usr/bin/env python3
"""Programs of their own run inside Term, with Term as their console
(`sex_pdx::console_relay`: bytes through Files, no window, no kernel API).

* Spindle's `spindle` command opens Term (Spindle's terminal window) with the `+launch` grant. `stdhello alpha beta` there is the
  ordinary Rust std program: every check passes, its output is on Term's screen, it opens
  no window of its own, and Term sees it end with status 0.
* `stdhello echo`: lines typed into Term reach its standard input; Ctrl+D ends input and
  it exits 3, which Term reports.
* `stdhello count` prints until Ctrl+C in Term stops it (status 130).
* `chello one two`: a C program on sex-libc, the same way; it is refused the network.
* Lua 5.4 (`lua -e`, then its interactive prompt until Ctrl+D) and CPython (`python -c`).
* A Term started with plain `run term` holds no grant: Files refuses its run (DENIED)."""
import argparse,os,re,shutil,time
from pathlib import Path
from install_program import install
# A network to refuse: a run Term starts never gets it (chello checks connect is EACCES).
os.environ.setdefault('SEXOS_GATE_NIC','-netdev user,id=n0 -device e1000e,netdev=n0')
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}
KEYS={' ':'spc','.':'dot','/':'slash','-':'minus','=':'equal',',':'comma',';':'semicolon'}
SHIFTED={'(':'9',')':'0','*':'8','+':'equal'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 progs=[(n,(ROOT/'iso_root/programs'/n).read_bytes()) for n in ('term','stdhello','chello','lua','python')]
 for n,b in progs:assert b[:4]==b'\x7fELF',n
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,progs)
 g=boot(a.iso,disk,a.out,'termnative',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k in SHIFTED or k.isupper():q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':SHIFTED.get(k,k.lower())}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.05)
 def chord(*ks):q.cmd('send-key',{'keys':[{'type':'qcode','data':k} for k in ks],'hold-time':80});time.sleep(.2)
 def spindle(line):g['focus'](153);time.sleep(.5);keys(line);q.key('ret')
 def term_line(line):keys(line);q.key('ret')
 def snapshot():
  s=len(t());term_line('snapshot');g['wait'](r'\[term\.snapshot\] rows=\d+',s,60)
  return '\n'.join(row for _,row in re.findall(r'\[term\.row\] r=(\d+) ([^\n]*)\n',t()[s:]))
 def start(line):
  s=len(t());spindle(line)
  m=g['wait'](r'\[term\.ready\] target=(\d+) task=(\d+) cols=(\d+) rows=(\d+)',s,240)
  target,task=int(m[1]),int(m[2])
  g['wait'](rf'\[display\.app\.title\] target={target} ',s,60);time.sleep(1)
  g['focus'](SURFACE[target]);time.sleep(.5)
  return target,task
 def ran(name,code,s,timeout=300):
  m=g['wait'](rf'\[term\.job\.exit\] name={name} code=(-?\d+)\n',s,timeout);assert m[1]==str(code),m[0];time.sleep(1)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  target,task=start('spindle')
  # An ordinary std program, its console in Term.
  s=len(t());term_line('stdhello alpha beta')
  g['wait'](r'\[term\.native\.start\] name=stdhello',s,60)
  g['wait'](r'\[files\.console\.attached\] ',s,240)
  m=g['wait'](r'\[stdhello\.(ok|fail)\] passed=(\d+)/(\d+)\n',s,600);assert m[1]=='ok',m[0]
  ran('stdhello',0,s)
  screen=snapshot();assert 'Hello from Rust std on SexOS.' in screen and 'alpha' in screen,screen
  q.cmd('screendump',{'filename':str(a.out/'term-native-std.ppm')})
  # Lines in, lines out; Ctrl+D ends input.
  term_line('clear');s=len(t());term_line('stdhello echo')
  g['wait'](r'\[files\.console\.attached\] ',s,240);time.sleep(3)
  term_line('hello');g['wait'](r'\[std\.out\] got: HELLO\n',s,60)
  term_line('two words');g['wait'](r'\[std\.out\] got: TWO WORDS\n',s,60)
  chord('ctrl','d');g['wait'](r'\[std\.out\] \[stdhello\.echo\] lines=2\n',s,60)
  ran('stdhello',3,s)
  screen=snapshot();assert 'got: TWO WORDS' in screen and '[stdhello exited with 3]' in screen,screen
  q.cmd('screendump',{'filename':str(a.out/'term-native-echo.ppm')})
  # Ctrl+C stops a program that never reads.
  term_line('clear');s=len(t());term_line('stdhello count')
  g['wait'](r'\[std\.out\] count 20\n',s,240)
  chord('ctrl','c');g['wait'](r'\[std\.interrupted\]',s,60)
  ran('stdhello',130,s)
  # C on sex-libc.
  term_line('clear');s=len(t());term_line('chello one two')
  m=g['wait'](r'\[chello\.(ok|fail)\] passed=(\d+)/(\d+)\n',s,600);assert m[1]=='ok',m[0]
  ran('chello',0,s)
  screen=snapshot();assert 'one' in screen,screen
  q.cmd('screendump',{'filename':str(a.out/'term-native-c.ppm')})
  # Lua and Python, one line each, then Lua's prompt read line by line until Ctrl+D.
  term_line('clear');s=len(t());term_line('lua -e print(6*7)')
  g['wait'](r'\[std\.out\] 42\n',s,300);ran('lua',0,s)
  s=len(t());term_line('lua');g['wait'](r'\[files\.console\.attached\] ',s,240);time.sleep(3)
  term_line('print(2+3)');g['wait'](r'\[std\.out\] (> )?5\n',s,60)
  chord('ctrl','d');ran('lua',0,s)
  s=len(t());term_line('python -c print(6*7)')
  g['wait'](r'\[std\.out\] 42\n',s,600);ran('python',0,s,600)
  screen=snapshot();assert screen.count('42')>=2 and '5' in screen,screen
  q.cmd('screendump',{'filename':str(a.out/'term-native-lua-python.ppm')})
  s=len(t());q.key('f11');g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s,120)
  # Without the grant, Files refuses.
  target,task=start('run term')
  s=len(t());term_line('stdhello');g['wait'](r'\[term\.native\.refused\] name=stdhello status=11\n',s,60)
  assert '[files.console.attached]' not in t()[s:]
  s=len(t());q.key('f11');g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s,120)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[term.fatal]' not in t()
 finally:g['stop']()
 print({'ok':True})
if __name__=='__main__':main()
