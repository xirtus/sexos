#!/usr/bin/env python3
"""Programs of their own run inside Term, with Term as their console
(`sex_pdx::console_relay`: bytes through Files, no window, no kernel API).

* Spindle's `spindle` command opens Term (Spindle's terminal window) with the `+launch` grant. `stdhello alpha beta` there is the
  ordinary Rust std program: every check passes, its output is on Term's screen, it opens
  no window of its own, and Term sees it end with status 0.
* `stdhello echo`: lines typed into Term reach its standard input; Ctrl+D ends input and
  it exits 3, which Term reports.
* `stdhello count` prints until Ctrl+C in Term stops it (status 130).
* `stdhello size` prints the size Term told Files (`[term.native.size]`), and the new size
  after Shell's Meta+M (monocle) resizes Term's window.
* `stdhello wait` blocked reading: Ctrl+C ends it within 50 ms (Term's and std's ticks).
* `stdhello spin` never reads, writes or sleeps: the first Ctrl+C cannot reach it, the
  second within a second has Shell stop it (status 137).
* `stdhello raw`: raw mode reaches Term (mode 7); keys arrive one at a time, Ctrl+C as 3.
* `chello one two`: a C program on sex-libc, the same way; it is refused the network, and
  its `ioctl(TIOCGWINSZ)` is Term's size.
* Lua 5.4 (`lua -e`, then its interactive prompt until Ctrl+D) and CPython (`python -c`).
* Two panes each run a program of their own at once.
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
  # The window's size reaches the program, and again after a resize.
  term_line('clear');s=len(t());term_line('stdhello size')
  told=g['wait'](r'\[term\.native\.size\] cols=(\d+) rows=(\d+) ok=1\n',s,60)
  m=g['wait'](r'\[std\.out\] \[stdhello\.size\] cols=(\d+) rows=(\d+) changes=(\d+)\n',s,240)
  assert (m[1],m[2])==(told[1],told[2]),(m[0],told[0])
  r=len(t());chord('meta_l','m');g['wait'](r'\[shell\.wm\.action\] action=ToggleMonocle done=1',r,30)
  told=g['wait'](r'\[term\.native\.size\] cols=(\d+) rows=(\d+) ok=1\n',r,60)
  m2=g['wait'](r'\[std\.out\] \[stdhello\.resized\] cols=(\d+) rows=(\d+) changes=(\d+)\n',r,90)
  assert (m2[1],m2[2])==(told[1],told[2]) and (m2[1],m2[2])!=(m[1],m[2]) and int(m2[3])>int(m[3]),(m[0],m2[0])
  ran('stdhello',0,s)
  r=len(t());chord('meta_l','m');g['wait'](r'\[shell\.wm\.action\] action=ToggleMonocle done=1',r,30);time.sleep(1)
  # A run blocked reading sees Ctrl+C within 50 ms.
  term_line('clear');s=len(t());term_line('stdhello wait')
  g['wait'](r'\[std\.out\] \[stdhello\.wait\] reading\n',s,240);time.sleep(2)
  chord('ctrl','c')
  pressed=g['wait'](r'\[term\.job\.interrupt\] name=stdhello ms=(\d+)\n',s,30)
  ended=g['wait'](r'\[std\.interrupted\] ms=(\d+)\n',s,30)
  late=int(ended[1])-int(pressed[1]);assert 0<=late<=50,f'interrupt took {late} ms'
  ran('stdhello',130,s)
  print({'interrupt_ms':late})
  # A run that computes without asking: the second Ctrl+C has Shell stop it.
  term_line('clear');s=len(t());term_line('stdhello spin')
  g['wait'](r'\[std\.out\] \[stdhello\.spin\] spinning\n',s,240);time.sleep(1)
  chord('ctrl','c');g['wait'](r'\[term\.job\.interrupt\] name=stdhello ',s,30);time.sleep(.3)
  chord('ctrl','c');g['wait'](r'\[term\.job\.stop\] name=stdhello target=\d+ ok=1 ',s,30)
  g['wait'](r'\[shell\.attached\.stop\] terminal=\d+ target=\d+ task=\d+ ok=1\n',s,30)
  ran('stdhello',137,s,120)
  # Raw mode: keys one at a time, unechoed, Ctrl+C a byte.
  term_line('clear');s=len(t());term_line('stdhello raw')
  g['wait'](r'\[std\.out\] \[stdhello\.raw\] on=1\n',s,240)
  g['wait'](r'\[term\.native\.mode\] name=stdhello mode=7\n',s,30)
  keys('a');g['wait'](r'\[std\.out\] \[stdhello\.raw\] byte=97\n',s,30)
  chord('ctrl','c');g['wait'](r'\[std\.out\] \[stdhello\.raw\] byte=3\n',s,30)
  keys('q');g['wait'](r'\[std\.out\] \[stdhello\.raw\] off\n',s,30)
  ran('stdhello',0,s)
  assert '[term.job.interrupt] name=stdhello' not in t()[s:]
  # Ctrl+C stops a program that never reads.
  term_line('clear');s=len(t());term_line('stdhello count')
  g['wait'](r'\[std\.out\] count 20\n',s,240)
  chord('ctrl','c');g['wait'](r'\[std\.interrupted\]',s,60)
  ran('stdhello',130,s)
  # C on sex-libc.
  term_line('clear');s=len(t());term_line('chello one two')
  m=g['wait'](r'\[chello\.(ok|fail)\] passed=(\d+)/(\d+)\n',s,600);assert m[1]=='ok',m[0]
  told=re.findall(r'\[term\.native\.size\] cols=(\d+) rows=(\d+) ok=1\n',t()[s:])[-1]
  ws=re.search(r'\[std\.out\] winsize cols=(\d+) rows=(\d+)\n',t()[s:]);assert ws and ws.groups()==told,(ws and ws[0],told)
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
  # Two panes, a program of its own in each at once (one relay slot per pane).
  term_line('clear');s=len(t());chord('ctrl','shift','e');g['wait'](r'\[term\.pane\.split\] ',s,30);time.sleep(1)
  term_line('stdhello wait');g['wait'](r'\[std\.out\] \[stdhello\.wait\] reading\n',s,240)
  chord('ctrl','shift','left');time.sleep(.5);r=len(t());term_line('stdhello alpha beta')
  m=g['wait'](r'\[stdhello\.(ok|fail)\] passed=(\d+)/(\d+)\n',r,600);assert m[1]=='ok',m[0]
  ran('stdhello',0,r)
  assert '[std.interrupted]' not in t()[s:],'the waiting pane was disturbed'
  chord('ctrl','shift','right');time.sleep(.5);r=len(t());chord('ctrl','c');ran('stdhello',130,r)
  q.cmd('screendump',{'filename':str(a.out/'term-native-two-panes.ppm')})
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
