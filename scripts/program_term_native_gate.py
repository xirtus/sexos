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
* A Term started with plain `run term` holds no relay grant: Files rejects its
  legacy relay ticket downstream. Term's missing refusal feedback is not qualified."""
import argparse,hashlib,json,os,re,shutil,time
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
def denial_sources(root):
 """Bind this legacy relay denial to the reviewed exact-run ticket route.

 These are bounded source-shape checks, not a general Rust authorization proof.
 Native denial receipts and their causal window remain checked separately.
 """
 paths=('servers/sexfiles/src/fs_service.rs','crates/sex-app/src/lib.rs',
        'servers/silk-shell/src/main.rs','servers/sexfiles/src/trampoline.rs')
 source={name:(root/name).read_text()for name in paths}
 def compact(text):
  # Preserve normal string literals; comment-only copies cannot satisfy a guard.
  text=re.sub(r'"(?:\\.|[^"\\])*"|//[^\n]*|/\*[\s\S]*?\*/',
              lambda m:'' if m[0].startswith(('//','/*')) else m[0],text)
  return re.sub(r'\s+','',text)
 def section(text,start,end):
  if text.count(start)!=1:raise ValueError('legacy relay source contract changed: ambiguous '+start)
  at=text.index(start);stop=text.find(end,at+len(start))
  if stop<0:raise ValueError('legacy relay source contract changed: missing '+end)
  return compact(text[at:stop])
 files,sdk,shell,trampoline=(source[name]for name in paths)
 body=compact(files);front=section(files,'pub fn handle(','fn bound_sender(')
 live=section(files,'fn program_live(','pub fn handle(')
 legacy=section(shell,'    if msg.type_id==store::OP_RUN {','        PROGRAM_READING=status==0;')
 stamped='ifby_program&&!program_run(msg.caller_pd).is_some_and(|(_,task)|launch_caller_task(task,msg.caller_task)){returntrue}'
 forwarded='pdx_call(store::SHELL_ROUTE_SLOT,store::OP,msg.arg0,msg.arg1,ifby_program{msg.caller_pdasu64}else{0})'
 sender=section(files,'fn bound_sender(','fn current_ticket_owner(')
 owner=section(files,'fn current_ticket_owner(','pub enum Program')
 ticket=section(files,'pub fn read_ticket(','fn run(')
 run=section(files,'fn run(','pub fn run_on')
 prepare=section(files,'fn prepare(','pub fn program_is(')
 facts=[
  ('exact ticket owner', 'structTicketOwner{target:u32,task:u32}' in body and
   'staticmutPREPARED_OWNER:Option<TicketOwner>=None;' in body),
  ('stamped live prepare', 'bound_task!=0&&program_live(target,bound_task)&&(!preparing||(sender_task!=0&&sender_task==bound_task))' in sender and
   'bound_sender(caller,caller_task,b.task,r.kind==w::PREPARE)' in front and
   'crate::fs_service::handle(caller,msg.caller_task,msg.arg0,msg.arg1,msg.arg2)' in compact(trampoline)),
  ('live exact catalog task', 'c.is_subject(target,row.surface,3)&&c.task_id==task' in live and
   'sex_pdx::app_lifecycle::info(row.lifecycle_slot)==Err(sex_pdx::app_lifecycle::LIVE)' in live),
  ('current live task owner', '(binding.task!=0&&program_live(target,binding.task)).then_some(TicketOwner{target,task:binding.task})' in owner),
  ('grant-specific relay prepare',
   'letrelay_terminal=program.is_some_and(|k|unsafe{(*core::ptr::addr_of!(BINDINGS))[k]}.is_some_and(|b|holds_launch(&b)));' in front and
   'iflet(Some(k),w::PREPARE,true)=(program,r.kind,relay_terminal)' in front and
   'if!crate::console_relay::may_prepare(k){returnw::reply(r.seq,w::PENDING,0);}' in front and
   'sex_pdx::console_relay::plain_args(' in front and 'PREPARING_FOR' not in body),
  ('explicit owner and relay forwarding',
   'letowner=ifr.kind==w::PREPARE&&program.is_some(){letSome(owner)=current_ticket_owner(caller)else{returnw::reply(r.seq,w::DENIED,0);};Some(owner)}else{None};letrelay=ifrelay_terminal{program}else{None};letresult=run(fs,r,path,data,owner,relay);' in front and
   'returnprepare(fs,r,path,data,owner,relay);' in run),
  ('captured exact owner and separate relay',
   'letby=relay;unsafe{PREPARED_OWNER=owner;PREPARED_LAUNCHER=by;}ifletSome(k)=by{crate::console_relay::prepared(k);}' in prepare),
  ('requester exact owner and relay check',
   'letSome(owner)=current_ticket_owner(requester)else{return(Program::Absent,None);};Some(owner)' in ticket and
   'ifunsafe{PREPARED_OWNER}!=owner||(by.is_some()&&by!=asker){sex_pdx::serial_record!("[files.ticket.denied]requester={}",requester);return(Program::Absent,None);}' in ticket),
  ('owner check before single-use consumption',
   ticket.find('ifunsafe{PREPARED_OWNER}!=owner')>=0 and
   ticket.find('ifunsafe{PREPARED_OWNER}!=owner')<ticket.find('core::mem::take(&mutprepared.1)')<ticket.find('PREPARED_OWNER=None;')),
  ('legacy relay request', 'sex_pdx::program_store::OP_RUN,sex_pdx::program_store::TICKET,ticketasu64' in compact(sdk)),
  ('Shell forwards stamped requester',
   'fnlaunch_caller_task(current_task:u32,caller_task:u32)->bool{current_task!=0&&current_task==caller_task}' in compact(shell) and
   stamped in legacy and forwarded in legacy and legacy.index(stamped)<legacy.index(forwarded)),
 ]
 if not all(ok for _,ok in facts):raise ValueError('legacy relay source contract changed: '+str(facts))
 return {name:hashlib.sha256((root/name).read_bytes()).hexdigest()for name in paths}

def downstream_denial(trace,launch_start,action_start,end,target,task,source_hashes):
 """Exact negative action window; an enqueue receipt is never launch success."""
 if not 0<=launch_start<action_start<end<=len(trace):raise ValueError('invalid denial boundaries')
 setup=trace[launch_start:action_start];window=trace[action_start:end]
 bound=rf'\[files.program.bound\] target={target} task={task} root=/data/term\n'
 ready=rf'\[term.ready\] target={target} task={task} cols=\d+ rows=\d+\n'
 b=list(re.finditer('^'+bound,setup,re.M));r=list(re.finditer('^'+ready,setup,re.M))
 if len(b)!=1 or len(r)!=1 or b[0].end()>r[0].start():raise ValueError('missing exact current Term run')
 # A replacement ready/binding cannot be silently accepted as the same run.
 forbidden=('shell.program.launch','files.program.bound','files.console.attached',
            'shell.launch.asked','shell.launch.event','app.activation.committed',
            'term.ready','term.job.exit','std.out','stdhello.ok','stdhello.fail')
 if any('['+tag+']'in window for tag in forbidden):raise ValueError('child, relay or run replacement in denial window')
 expected=[r'\[term.command\] stdhello\n',r'\[term.native.start\] name=stdhello\n',
           rf'\[files.ticket.denied\] requester={target}\n',
           r'\[shell.program.denied\] reason=store error=1\n',r'\[spindle.program.refused\] code=1\n']
 matches=[]
 for pattern in expected:
  found=list(re.finditer('^'+pattern,window,re.M))
  if len(found)!=1:raise ValueError('missing/duplicate downstream denial receipt: '+pattern)
  matches.append(found[0])
 if any(a.end()>b.start()for a,b in zip(matches,matches[1:])):raise ValueError('denial receipt order differs')
 if window.count('[files.ticket.denied]')!=1 or window.count('[shell.program.denied]')!=1:raise ValueError('unrelated requester/store denial')
 if window.count('[term.command]')!=1 or window.count('[term.native.start]')!=1 or window.count('[spindle.program.refused]')!=1:raise ValueError('nonexclusive Term action')
 ticks=[int(m[1])for m in re.finditer(r'^\[sched.active\] tick=(\d+) [^\n]*\n',window[matches[-1].end():],re.M)]
 if len(ticks)<2 or any(b<=a for a,b in zip(ticks,ticks[1:])):raise ValueError('missing progressing post-denial observation window')
 return dict(ok=True,target=target,task=task,launch_start=launch_start,action_start=action_start,end=end,
             serial_sha256=hashlib.sha256(trace[:end].encode()).hexdigest(),source_sha256=source_hashes,
             downstream_denied=True,child_started=False,relay_attached=False,feedback_qualified=False,
             observation_ticks=[ticks[0],ticks[-1]],scope='Exact legacy relay ticket denial; Term refusal feedback remains unqualified.')
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
  return target,task,s
 def ran(name,code,s,timeout=300):
  m=g['wait'](rf'\[term\.job\.exit\] name={name} code=(-?\d+)\n',s,timeout);assert m[1]==str(code),m[0];time.sleep(1)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  target,task,_=start('spindle')
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
  source_hashes=denial_sources(ROOT)
  target,task,launch_start=start('run term')
  s=len(t());term_line('stdhello')
  g['wait'](rf'\[files\.ticket\.denied\] requester={target}\n',s,60)
  refusal=g['wait'](r'\[spindle\.program\.refused\] code=1\n',s,60)
  cursor=s+refusal.end()
  first=g['wait'](r'\[sched.active\] tick=(\d+) [^\n]*\n',cursor,60)
  cursor+=first.end()
  g['wait'](r'\[sched.active\] tick=(\d+) [^\n]*\n',cursor,60)
  assert denial_sources(ROOT)==source_hashes,'source changed during denial capture'
  proof=downstream_denial(t(),launch_start,s,len(t()),target,task,source_hashes)
  (a.out/'ungranted-denial.json').write_text(json.dumps(proof,indent=2)+'\n')
  s=len(t());q.key('f11');g['wait'](rf'\[app\.private\.retired\] target={target} task={task} ',s,120)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[term.fatal]' not in t()
 finally:g['stop']()
 print({'ok':True})
if __name__=='__main__':main()
