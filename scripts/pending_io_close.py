"""Normal-image, real-input Quil Close while a journal WRITE is outstanding."""
import gzip,hashlib,json,re,socket,struct,threading,time
from pathlib import Path
from desktop_close_gate import QMP
from check_document_disk import fnv,recover,object_bytes
from network_contention_gate import cleanup_receipts
from check_pending_io_trace import check_trace,outstanding_target_writes,MAX_TRACE_BYTES

BASELINE=b'x'*12286
SNAPSHOT=BASELINE+b'q'
EVENTS=('serial_write','pci_nvme_io_cmd','pci_nvme_write','pci_nvme_enqueue_req_completion')

def journal_ranges(disk,document):
 raw,identity=object_bytes(disk,document)
 if len(raw)!=65536:raise ValueError('prepared journal is not fully grown')
 actual=recover(disk,document)
 if actual['payload']!=BASELINE:raise ValueError('prepared journal lacks exact ASCII baseline')
 descriptor=disk[(1925-document)*512:(1926-document)*512]
 count=struct.unpack_from('<H',descriptor,6)[0]
 ranges=[]
 for i in range(count):
  start,length=struct.unpack_from('<HH',descriptor,12+i*4)
  if not length:raise ValueError('empty journal extent')
  ranges.append((start,start+length))
 if sum(b-a for a,b in ranges)!=128:raise ValueError('journal extents do not cover exact prepared container')
 return ranges,identity

class CriticalProfile:
 """A separate QMP connection restores limits even if main input waits fail."""
 def __init__(self,path,stop_guest,disk=None,kill_guest=None):
  self.path=path;self.stop_guest=stop_guest;self.ready=threading.Event();self.done=threading.Event();self.rows=[];self.errors=[];self.armed=None;self.disarmed=None;self.q=None
  self.thread=threading.Thread(target=self.worker,daemon=True)
  self.disk=None if disk is None else Path(disk).resolve()
  self.kill_guest=stop_guest if kill_guest is None else kill_guest;self.deadline_timer=None;self.expired=False
 def expire(self):
  self.expired=True;self.errors.append('absolute eight-second deadline expired');self.done.set();self.kill_guest()
 def call(self,op,args):
  began=time.monotonic();result=self.q.cmd(op,args);self.rows.append(dict(command=op,arguments=args,result=result,started=began,completed=time.monotonic()));return result
 def throttle(self,rate):
  self.call('block_set_io_throttle',dict(device='nvm',bps=0,bps_rd=0,bps_wr=0,iops=0,iops_rd=0,iops_wr=rate,iops_size=0))
 def worker(self):
  try:
   self.q=QMP(self.path)
   blocks=self.call('query-block',{})
   backend=[b for b in blocks if b.get('device')=='nvm']
   if len(backend)!=1:raise ValueError('exact NVMe backend absent')
   if self.disk is not None and Path(backend[0].get('inserted',{}).get('file','')).resolve()!=self.disk:raise ValueError('NVMe backend is not the owned fixture disk')
   for name in EVENTS:self.call('trace-event-set-state',dict(name=name,enable=True))
   self.armed=time.monotonic();self.deadline_timer=threading.Timer(8,self.expire);self.deadline_timer.daemon=True;self.deadline_timer.start()
   self.throttle(2);self.ready.set();self.done.wait(max(0,self.armed+7-time.monotonic()))
  except BaseException as error:self.errors.append(type(error).__name__+': '+str(error))
  finally:
   self.ready.set()
   if self.q:
    try:
     self.throttle(0);self.disarmed=time.monotonic()
     if self.deadline_timer:self.deadline_timer.cancel()
    except BaseException as error:self.errors.append('restore: '+str(error))
    for name in EVENTS:
     try:self.call('trace-event-set-state',dict(name=name,enable=False))
     except BaseException as error:self.errors.append('trace-disable: '+str(error))
    self.q.file.close();self.q.sock.close()
 def start(self):
  self.thread.start()
  if not self.ready.wait(10)or self.errors or self.armed is None:raise RuntimeError('critical profile could not arm: '+str(self.errors))
 def finish(self):
  self.done.set();self.thread.join(35)
  if self.thread.is_alive():
   self.stop_guest()
   if self.q:self.q.sock.shutdown(socket.SHUT_RDWR)
   self.thread.join(5)
  if self.thread.is_alive()or self.errors or self.disarmed is None:raise RuntimeError('critical profile restore failed: '+str(self.errors))
  duration=self.disarmed-self.armed if self.armed is not None else None
  if duration is None or not 0<duration<=8:raise RuntimeError('critical write throttle exceeded eight-second budget')
  return dict(armed_seconds=duration,commands=self.rows,restored=True,trace_disabled=True,absolute_deadline_seconds=8,deadline_fired=self.expired)

def reopen(c,document,allowed):
 from quil_third_probe import open_selected
 c['key']('grave_accent',r'\[shell.palette.item\] idx=0')
 c['key']('j',r'\[shell.palette.select\] old=0 new=1')
 c['key']('j',r'\[shell.palette.select\] old=1 new=2')
 c['key']('ret',r'\[shell.focus.set\] id=200');c['focus'](200)
 for _ in range(20):
  c['key']('j',r'\[shell.action.select_next_linen\] id=\d+')
  got=open_selected(c)
  if got and got['object']==document:
   if (got['bytes'],got['hash'])not in [(len(b),hex(fnv(b)))for b in allowed]:raise ValueError('reopened document is neither complete baseline nor immutable snapshot')
   return got
  c['focus'](200)
 raise RuntimeError('prepared document not selectable through Linen')

def run(c):
 q,wait,text,key,ctrl,focus=(c[k]for k in ('q','wait','text','key','ctrl','focus'))
 out=c['out'];focus(201);ctrl('n',r'\[quil.new.ok\] bytes=0');c['typing']('xxxxxx');length=6
 for _ in range(11):
  ctrl('a',rf'\[quil.selection\] generation=\d+ revision=\d+ bytes={length} anchor=0 caret={length}')
  ctrl('c',rf'\[quil.clipboard.published\] ticket=\d+ bytes={length} generation=\d+ hash={hex(fnv(b"x"*length))} cut=0 applied=0')
  key('end',r'\[quil.cursor.move\] .*dir=end ok=1')
  ctrl('v',rf'\[quil.clipboard.paste\] ticket=\d+ bytes={length} generation=\d+ hash={hex(fnv(b"x"*length))} applied=1 atomic=1')
  length*=2
 for length in (12287,12286):key('backspace',rf'\[quil.document.edit\] revision=\d+ pending=0 bytes={length}(?:\s|$)')
 ctrl('s',r'\[quil.name.mode\] on=1');c['typing']('pendclose',True);start=len(text());q.key_sequence(['ret'])
 saved=wait(rf'\[quil.persist.save.ok\] bytes=12286 hash={hex(fnv(BASELINE))} doc=(\d+)',start);document=int(saved[1]);wait(r'\[quil.document.saved\].*dirty=0',start)
 before=c['disk'].read_bytes();ranges,identity=journal_ranges(before,document);(out/'before-pending.img.gz').write_bytes(gzip.compress(before,mtime=0))
 reopen(c,document,[BASELINE]);c['capture']('pending-baseline','pendclose');focus(201)
 key('q',r'\[quil.document.edit\] revision=\d+ pending=0 bytes=12287(?:\s|$)')
 profile=CriticalProfile(c['control_sock'],c['stop_guest'],c['disk'],c['kill_guest']);failure=None;record=None;candidate=c['captures'].candidate('pending-close-guard')
 try:
  profile.start();start=len(text())
  q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}for k,down in [('ctrl',True),('s',True),('s',False),('ctrl',False),('z',True),('z',False),('f11',True),('f11',False)]]})
  wait(r'\[quil.close.declined\] dirty=1 pending=1 revision=\d+',start,timeout=6)
  wait(r'\[shell.app.close.declined\] target=9 task=9 reason=unsaved next_close=confirmed',start,timeout=6)
  wait(r'\[quil.text.view\] target=9 .*status=.*UNSAVED',start,timeout=6)
  deadline=profile.armed+6
  while time.monotonic()<deadline:
   c['health']();raw=(out/'device-trace.log').read_bytes()
   if len(raw)>MAX_TRACE_BYTES:raise ValueError('bounded critical trace exceeded')
   end=raw.rfind(b'\n')+1;pending=outstanding_target_writes(raw[:end],ranges)
   if len(pending)==1:
    observed=pending[0];observed_bytes=end;break
   time.sleep(.01)
  else:raise RuntimeError('no unique outstanding document WRITE before confirmation')
  q.cmd('screendump',{'filename':str(candidate)});prefix=text();pixels=candidate.read_bytes();confirm=len(text());q.key_sequence(['f11'])
  wait(r'\[quil.close.confirmed\] dirty=1 pending=1',confirm,timeout=6);wait(r'\[quil.document.close\] pending=1',confirm,timeout=6)
  wait(r'\[task.finish\] pd=9 task=9 reason=0 terminal=1',confirm,timeout=6)
  wait(r'\[sext.app.pool.ready\] target=9 task=9 .*ok=1',confirm,timeout=6)
  while time.monotonic()<deadline:
   raw=(out/'device-trace.log').read_bytes();end=raw.rfind(b'\n')+1
   try:proof=check_trace(raw[:end],ranges);break
   except ValueError:time.sleep(.01)
  else:raise RuntimeError('same-trace WRITE/Close/finish/completion order not proved')
 except BaseException as error:failure=error;raise
 finally:
  try:record=profile.finish()
  finally:(out/'pending-profile.json').write_text(json.dumps(dict(profile=record,error=None if failure is None else str(failure),commands=profile.rows),indent=2)+'\n')
 # Verification is deliberately after confirmed Close and throttle removal.
 from alpha_evidence import verify_frame
 guard=verify_frame(prefix,pixels,c['pixel_contract'],c['resolution'],'quil','UNSAVED',9)
 if not guard['ok']:raise ValueError('retained pre-confirm guard pixels: '+str(guard['errors']))
 frame=c['captures'].commit('pending-close-guard',candidate,prefix,guard,'quil','UNSAVED',9,None,pixels)
 proof=check_trace((out/'device-trace.log').read_bytes(),ranges)
 if proof['cid']!=observed['cid']or proof['command']!=observed['command']or proof['write']!=observed['write']:raise ValueError('observed outstanding WRITE differs from final device proof')
 wait(r'\[sext.app.pool.ready\] target=9 task=9 .*ok=1',confirm);cleanup=cleanup_receipts(text(),9,9,confirm)
 key('grave_accent',r'\[shell.palette.item\] idx=0');key('j',r'\[shell.palette.select\] old=0 new=1');key('ret',r'\[shell.app.relaunch\] predecessor=9 successor=16 ')
 wait(r'\[app.surface.presented\] target=16 surface=301 .*ok=1');loaded=reopen(c,document,[BASELINE,SNAPSHOT]);successor_frame=c['capture']('pending-successor-recovered','pendclose',target=16)
 for target,sid in [(16,301),(15,300),(12,153)]:
  focus(sid);offset=len(text());key('f11',rf'\[shell.app.close.request\] target={target} task={target}(?:\s|$)');wait(rf'\[sext.app.pool.ready\] target={target} task={target} .*ok=1',offset)
 # Preserve survivor input during a real rescan, then selection after commit.
 key('grave_accent',r'\[shell.palette.item\] idx=0');key('j',r'\[shell.palette.select\] old=0 new=1');key('j',r'\[shell.palette.select\] old=1 new=2')
 offset=len(text());q.key_sequence(['ret','j']);wait(r'\[linen.remote.snapshot.ok\].*complete=1',offset);key('j',r'\[shell.action.select_next_linen\] id=[1-9]\d*')
 return dict(document=document,identity=identity,journal_ranges=ranges,baseline_bytes=len(BASELINE),snapshot_bytes=len(SNAPSHOT),profile=record,trace_proof=proof,observation=observed,observed_trace_bytes=observed_bytes,guard_frame=frame,cleanup=cleanup,successor=loaded,successor_frame=successor_frame,terminal_pending=True,cold_recovery_verified=False,scope='Actual device WRITE crosses confirmed Quil9 Close/task.finish; cancellation permits complete old or new snapshot, not rollback. Separate cold boot still required.')

def verify(out,evidence,frames,text,command):
 out=Path(out);document=evidence['document'];before=gzip.decompress((out/'before-pending.img.gz').read_bytes());ranges,identity=journal_ranges(before,document)
 if evidence['identity']!=identity or evidence['journal_ranges']!=[list(r)for r in ranges]:raise ValueError('pending fixture journal identity/extents differ')
 if command.count('-D')!=1 or command[command.index('-D')+1]!=str(out/'device-trace.log')or command.count('-qmp')!=2 or command.count('-msg')!=1 or command[command.index('-msg')+1]!='timestamp=on':raise ValueError('pending fixture trace/control command differs')
 if command.count('if=none,id=nvm,file='+str(out/'nvme.img')+',format=raw')!=1 or len([a for a in command if a.startswith('nvme,')])!=1 or command.count('nvme,serial=sexos01,drive=nvm')!=1:raise ValueError('pending fixture must trace the one owned NVMe disk')
 raw=(out/'device-trace.log').read_bytes();proof=check_trace(raw,ranges)
 if proof!=evidence['trace_proof']:raise ValueError('pending device trace proof differs')
 observed=evidence['observation'];prefix=raw[:evidence['observed_trace_bytes']];candidates=outstanding_target_writes(prefix,ranges)
 if len(candidates)!=1 or candidates[0]!=observed or proof['cid']!=observed['cid']or proof['command']!=observed['command']or proof['write']!=observed['write']:raise ValueError('pending WRITE observation is not bound to final trace')
 profile=evidence['profile'];rows=profile['commands'];expected=[('query-block',None)]+[('trace-event-set-state',dict(name=n,enable=True))for n in EVENTS]+[('block_set_io_throttle',dict(device='nvm',bps=0,bps_rd=0,bps_wr=0,iops=0,iops_rd=0,iops_wr=i,iops_size=0))for i in (2,0)]+[('trace-event-set-state',dict(name=n,enable=False))for n in EVENTS]
 if len(rows)!=len(expected)or not profile['restored']or not profile['trace_disabled']or profile.get('absolute_deadline_seconds')!=8 or profile.get('deadline_fired')is not False:raise ValueError('missing bounded throttle/trace restoration')
 backend=[b for b in rows[0]['result']if b.get('device')=='nvm']
 if len(backend)!=1 or Path(backend[0].get('inserted',{}).get('file','')).resolve()!=(out/'nvme.img').resolve():raise ValueError('pending QMP selected a different disk')
 previous=0
 for row,(name,args)in zip(rows,expected):
  if row['command']!=name or args is not None and row['arguments']!=args or not previous<=row['started']<=row['completed']:raise ValueError('pending QMP profile command/order differs')
  previous=row['completed']
 if not 0<profile['armed_seconds']<=8 or abs(rows[6]['completed']-rows[5]['started']-profile['armed_seconds'])>.001:raise ValueError('pending write throttle not bounded to eight seconds')
 guard=[f for f in frames if f['name']=='pending-close-guard']
 if len(guard)!=1 or guard[0]!=evidence['guard_frame']or(guard[0]['kind'],guard[0]['target'],guard[0]['expected'])!=('quil',9,'UNSAVED'):raise ValueError('pending guard frame is not immutable/subject exact')
 if not re.search(r'\[quil.close.declined\] dirty=1 pending=1 revision=\d+',text[:guard[0]['serial_end']]):raise ValueError('guard frame predates pending decline')
 uart=proof['uart']['text'].replace('\r\n','\n').replace('\r','\n');spans=list(re.finditer(re.escape(uart),text))
 if len(spans)!=1:raise ValueError('device UART Close/finish/pool span is not exact unique serial evidence')
 close_offset=spans[0].start();start=text.rfind('[quil.close.confirmed] dirty=1 pending=1',0,close_offset)
 if start<0 or guard[0]['serial_end']>start or cleanup_receipts(text,9,9,start)!=evidence['cleanup']:raise ValueError('pending pre-confirm frame/exact-incarnation cleanup differs')
 from check_desktop_close_log import check
 errors=check(text,include_successor=True)
 if errors:raise ValueError('pending fixture terminal cleanup: '+str(errors))
 disk=gzip.decompress((out/'nvme.img.gz').read_bytes());found=recover(disk,document);payload=found.pop('payload')
 if payload not in (BASELINE,SNAPSHOT)or found!=evidence['terminal_document']:raise ValueError('pending terminal document partial/unsaved or changed')
 if(evidence['successor']['bytes'],evidence['successor']['hash'])!=(len(payload),hex(fnv(payload))):raise ValueError('successor load differs from terminal journal')
 successor=[f for f in frames if f['name']=='pending-successor-recovered'];relaunched=text.find('[shell.app.relaunch] predecessor=9 successor=16 ',evidence['cleanup']['pool_offset']);retired=text.find('[task.finish] pd=16 task=16 reason=0 terminal=1',relaunched)
 if len(successor)!=1 or successor[0]!=evidence.get('successor_frame')or(successor[0]['kind'],successor[0]['target'],successor[0]['expected'])!=('quil',16,'pendclose')or not 0<=relaunched<successor[0]['serial_end']<retired:raise ValueError('pending successor lacks exact BOOT relaunch/pixels/retirement')
 if not re.search(rf'\[quil.persist.load.ok\] bytes={len(payload)} hash={hex(fnv(payload))} doc={document}(?:\s|$)',text[relaunched:successor[0]['serial_end']]):raise ValueError('pending successor lacks exact positive durable load')
 return dict(ok=True,device_overlap=proof,document=found,cold_recovery_verified=False,scope='Native pending-device WRITE/Close/task.finish, exact cleanup and successor recovery. Separate cold boot remains required.')
