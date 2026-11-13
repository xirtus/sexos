#!/usr/bin/env python3
"""Quil idle autosave survives power loss and reopens from Linen.

Boot 1: type into Quil, wait for the idle draft save (`<title>.draft`), type more,
wait for the second draft save into the same draft object, confirm the user's
document was never saved and stays dirty, then cut power (QEMU kill, no
shutdown). Boot 2 on the same disk: open Linen, select the draft by name and
load it in Quil; the loaded bytes/hash must equal the last drafted buffer.
"""
import argparse,hashlib,json,os,re,shutil,subprocess,time
from pathlib import Path
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from check_desktop_pixels import final_view
from network_contention_gate import frozen_binding
from certificate_time_gate import RTC_NORMAL
from program_role_facts import check_ordinary_launches

FIRST=['d','r','a','f','t','spc','o','n','e']
SECOND=['spc','a','n','d','spc','t','w','o']
SCOPE='Native QEMU power-cut recovery of a Quil idle draft through the held document route and Linen; no claim about physical media flush ordering.'

def disk_args(disk):
 """The gate disk on NVMe (default) or, with SEXOS_GATE_DISK_BUS=ahci, as a SATA disk on q35's built-in AHCI."""
 bus=os.environ.get('SEXOS_GATE_DISK_BUS','nvme')
 if bus=='nvme':return['-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial='+os.environ.get('SEXOS_GATE_DISK_SERIAL','sexos01')+',drive=nvm']
 if bus=='ahci':return['-drive',f'if=none,id=sata,file={disk},format=raw','-device','ide-hd,drive=sata,bus=ide.0']
 raise ValueError('SEXOS_GATE_DISK_BUS must be nvme or ahci')

def check_serial_health(text,tag):
 m=re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',text)
 # Name what failed: the matching line and a few after it.
 if m:raise RuntimeError(tag+' guest failure: '+text[text.rfind('\n',0,m.start())+1:][:600])

def wait_serial(text,health,pattern,start=0,timeout=120,*,tag='serial',raw=False):
 """Wait for complete receipts; raw is reserved for unterminated firmware menus.

 Keep text offsets and match offsets unchanged. Health always checks the raw
 stream, including faults whose terminating newline has not arrived yet.
 """
 d=time.monotonic()+timeout
 while time.monotonic()<d:
  health();now=text()
  if not raw:now=now[:now.rfind('\n')+1]
  match=re.search(pattern,now[start:])
  if match:return match
  time.sleep(.02)
 raise TimeoutError(f'{tag}: {pattern}')

def boot(iso,disk,out,tag,resolution):
 serial=out/f'{tag}-serial.log';sock=Path('/tmp')/f'sex-autosave-{os.getpid()}-{tag}.sock'
 cmd=['qemu-system-x86_64','-M','q35','-cpu',os.environ.get('SEXOS_GATE_CPU','max,+pku,+rdseed'),'-rtc',RTC_NORMAL,'-smp',os.environ.get('SEXOS_GATE_CORES','1'),*(['-accel','tcg,thread=multi']if os.environ.get('SEXOS_GATE_CORES','1')!='1'else[]),'-m',os.environ.get('SEXOS_GATE_MEMORY','512M'),'-vga','none','-device','VGA,xres='+resolution.replace('x',',yres='),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0',*([]if os.environ.get('SEXOS_GATE_NO_CDROM')else['-cdrom',str(iso)]),*(['-drive','if=pflash,format=raw,readonly=on,file='+os.environ['SEXOS_GATE_FIRMWARE']]if os.environ.get('SEXOS_GATE_FIRMWARE')else[]),*disk_args(disk),'-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown',*os.environ.get('SEXOS_GATE_NIC','-nic none').split(),*os.environ.get('SEXOS_GATE_DEVICES','').split(),*os.environ.get('SEXOS_GATE_IOMMU','').split()]
 # QEMU writes the serial file from its start without cutting it: an older run's
 # longer log would show through after the new one's first lines.
 serial.unlink(missing_ok=True)
 with (out/f'{tag}-qemu.log').open('w') as qemu_log:
  proc=subprocess.Popen(cmd,stdout=qemu_log,stderr=subprocess.STDOUT)
 began=time.monotonic()
 # The log only grows: read what is new, so hours-long runs don't reread it on every poll.
 seen={'off':0,'text':'','health':0}
 def text():
  if not serial.exists():return ''
  with serial.open('rb') as f:
   f.seek(seen['off']);new=f.read()
  # Decode only whole lines; a partial last line waits for its end.
  cut=new.rfind(b'\n')+1
  if cut:seen['off']+=cut;seen['text']+=new[:cut].decode('utf-8','replace')
  return seen['text']+new[cut:].decode('utf-8','replace') if cut<len(new) else seen['text']
 def health():
  if time.monotonic()-began>int(os.environ.get('SEXOS_GATE_BOUND','300')):raise TimeoutError(tag+' over its time bound')
  if proc.poll()is not None:raise RuntimeError(tag+' guest exited')
  # Only text not yet checked, backing up a little for a marker split across reads.
  now=text();start=max(0,seen.get('health',0)-64);seen['health']=len(now)
  check_serial_health(now[start:],tag)
 checked_launches={'off':0}
 def wait(pattern,start=0,timeout=120,*,raw=False):
  match=wait_serial(text,health,pattern,start,timeout,tag=tag,raw=raw)
  checked_launches['off']=check_ordinary_launches(Path(__file__).resolve().parents[1],text(),checked_launches['off'])
  return match
 def stop():
  if proc.poll()is None:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
 try:
  d=time.monotonic()+15
  while not sock.exists():
   health()
   if time.monotonic()>d:raise TimeoutError('QMP unavailable')
   time.sleep(.05)
  q=QMP(sock)
  if resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=10,raw=True);q.key('down');q.key('ret')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1',timeout=180);wait(r'\[linen.disk.publish.done\]',timeout=180)
 except BaseException:
  stop();raise
 def key(name,pattern):s=len(text());q.key(name);return wait(pattern,s)
 def chord(mod,k,pattern):
  s=len(text());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,(mod,k)),(False,(k,mod)))for x in ks]});return wait(pattern,s)
 return dict(q=q,proc=proc,text=text,wait=wait,key=key,chord=chord,stop=stop,focus=lambda sid:focus_surface(sid,text,key))

def settle(g,timeout):
 """Wait until Display applied Quil's latest admitted paint."""
 d=time.monotonic()+timeout
 while True:
  txt=g['text']();adm=re.findall(r'\[quil.ui.admitted\] paint=(\d+)',txt)
  if adm and re.search(rf'\[quil.ui.applied\] paint={adm[-1]} ',txt):return
  if time.monotonic()>d:raise TimeoutError('Quil view never applied by Display')
  time.sleep(.1)

def type_keys(g,keys):
 for i in range(0,len(keys),5):
  s=len(g['text']());g['q'].key_sequence(keys[i:i+5]);g['wait'](r'\[quil.document.edit\]',s)

def main():
 p=argparse.ArgumentParser();[p.add_argument(f'--{n}',required=True,type=Path)for n in('iso','source-disk','binding','out')];p.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');a=p.parse_args()
 root=Path(__file__).resolve().parents[1];out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 iso=a.iso.resolve(strict=True);binding=frozen_binding(root,iso,a.binding.resolve(strict=True));held=json.loads(a.binding.read_text())
 if held['runner_sha256'].get('scripts/quil_autosave_gate.py')!=hashlib.sha256(Path(__file__).read_bytes()).hexdigest():raise RuntimeError('autosave checker absent from frozen binding')
 disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk);result=dict(binding=binding,resolution=a.resolution,scope=SCOPE);failure=None;g=None
 try:
  g=boot(iso,disk,out,'boot1',a.resolution);t=g['text']
  g['focus'](201);g['chord']('ctrl','n',r'\[quil.new.ok\] bytes=0');start=len(t())
  type_keys(g,FIRST);typed=time.monotonic()
  b1=g['wait'](r'\[quil.autosave.begin\] draft=0 name=(\S+\.draft) revision=(\d+) bytes=(\d+) hash=(0x[0-9a-f]+) document=(\d+) document_unchanged=1',start,90)
  idle_s=time.monotonic()-typed
  if not 20<=idle_s<=75:raise RuntimeError(f'draft started {idle_s:.1f}s after the last edit, outside the ~30 s idle window')
  r1=g['wait'](rf'\[quil.autosave.result\] outcome=Saved result=Saved {{ draft: (\d+), revision: {b1[2]} }} revision=\d+ dirty=1 document={b1[5]}',b1.end()+start)
  draft,name=int(r1[1]),b1[1]
  g['chord']('ctrl','i',r'\[quil.stats\] lines=1 words=2 chars=9 bytes=9');g['wait'](r'\[quil.text.view\][^\n]*1 LINES / 2 WORDS / 9 CHARS',0,30)
  g['chord']('ctrl','g',r'\[quil.goto.mode\] on=1');s0=len(t());g['q'].key('5');g['wait'](r'\[quil.text.view\][^\n]*Go to line: 5',s0,30)
  g['key']('ret',r'\[quil.goto\] line=5 reached=1 old=9 new=0 len=9');checks_goto=True
  g['chord']('ctrl','right',r'\[quil.cursor.move\] old=0 new=5 len=9 dir=word_right ok=1')
  g['chord']('ctrl','end',r'\[quil.cursor.move\] old=5 new=9 len=9 dir=doc_end ok=1')
  g['chord']('ctrl','home',r'\[quil.cursor.move\] old=9 new=0 len=9 dir=doc_start ok=1')
  g['key']('end',r'\[quil.cursor.move\] old=0 new=9 len=9 dir=end ok=1')
  mid=len(t())
  type_keys(g,SECOND)
  b2=g['wait'](rf'\[quil.autosave.begin\] draft={draft} name={re.escape(name)} revision=(\d+) bytes=(\d+) hash=(0x[0-9a-f]+) document={b1[5]} document_unchanged=1',mid,90)
  g['wait'](rf'\[quil.autosave.result\] outcome=Saved result=Saved {{ draft: {draft}, revision: {b2[1]} }} revision=\d+ dirty=1',mid)
  if re.search(r'\[quil.persist.save.ok\]',t()[start:]):raise RuntimeError('autosave saved the user document')
  g['wait'](r'\[quil.text.view\] target=9 surface=\d+ status=\S* ?[^\n]*DRAFT KEPT',mid,30);settle(g,60);g['q'].cmd('screendump',{'filename':str(out/'draft-kept.ppm')})
  idle=re.findall(r'\[quil.autosave.begin\]',t());result['boot1']=dict(idle_seconds=round(idle_s,1),name=name,draft=draft,first=dict(revision=int(b1[2]),bytes=int(b1[3]),hash=b1[4]),second=dict(revision=int(b2[1]),bytes=int(b2[2]),hash=b2[3]),document=int(b1[5]),document_saved=False,drafts_started=len(idle))
  g['stop']();g=None
  g=boot(iso,disk,out,'boot2',a.resolution);t=g['text']
  g['focus'](201);g['chord']('ctrl','o',r'\[shell.document.open\] target=9 linen=200');g['focus'](200)
  for _ in range(40):
   try:
    view,labels=final_view(t())
    if any(label==name and ident==view['selected']for ident,_,label in labels):break
   except ValueError:pass
   g['key']('j',r'\[shell.action.select_next_linen\] id=\d+')
  else:raise RuntimeError('draft never listed/selected in Linen: '+name)
  m=g['key']('ret',rf'\[quil.persist.load.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)')
  result['boot2']=dict(bytes=int(m[1]),hash=m[2],doc=int(m[3]))
  if(int(m[1]),m[2])!=(int(b2[2]),b2[3]):raise RuntimeError(f'recovered draft differs: {m[0]} vs {b2[0]}')
  g['focus'](201);g['key']('end',r'\[quil.cursor.move\] old=\d+ new=\d+ len=\d+ dir=end ok=1');time.sleep(.5)
  g['chord']('ctrl','backspace',r'\[quil.word.delete\] dir=back bytes=3 changed=1 len=14')
  # Find "n", replacement "N", Ctrl+Enter replaces both as one edit.
  # First Enter finds forward (wrapping to 7), Shift+Enter backward (wrapping to 11).
  g['chord']('ctrl','f',r'\[quil.find.mode\] on=1');g['q'].key('n')
  g['key']('ret',r'\[quil.find\] query_bytes=1 start=7 end=8 wrapped=1 found=1 selected=1')
  g['chord']('shift','ret',r'\[quil.find.prev\] query_bytes=1 start=11 end=12 wrapped=1 found=1 selected=1')
  g['q'].key('down');g['q'].cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'n'}],'hold-time':80})
  g['wait'](r'\[quil.text.view\][^\n]*Find: n > N#',0,30)
  g['chord']('ctrl','ret',r'\[quil.replace.all\] count=2 start=7 end=12 new_bytes=5 applied=1 len=14')
  g['key']('esc',r'\[quil.find.mode\] on=0 reason=escape')
  g['chord']('ctrl','l',r'\[quil.select.line\] start=0 end=14 extended=0 ok=1')
  # A second line "x", then Ctrl+Up moves it above the first.
  g['key']('end',r'\[quil.cursor.move\] old=\d+ new=14 len=14 dir=end ok=1');s0=len(t());g['q'].key('ret');g['q'].key('x')
  g['wait'](r'\[quil.document.edit\] revision=\d+ pending=\d+ bytes=16',s0,30)
  g['chord']('ctrl','up',r'\[quil.line.move\] up=1 start=0 end=16 caret=1 applied=1 len=16')
  # Select all, Ctrl+] indents both lines, Ctrl+[ outdents them again.
  g['chord']('ctrl','a',r'\[quil.selection\] generation=\d+ revision=\d+ bytes=16 anchor=0 caret=16')
  g['chord']('ctrl','bracket_right',r'\[quil.indent\] lines=2 out=0 start=0 end=18 applied=1 len=18')
  g['chord']('ctrl','bracket_left',r'\[quil.indent\] lines=2 out=1 start=0 end=16 applied=1 len=16')
  # Ctrl+Shift+D copies the caret's (second) line below it.
  s0=len(t());g['q'].cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'shift'},{'type':'qcode','data':'d'}],'hold-time':80})
  g['wait'](r'\[quil.line.duplicate\] start=16 bytes=14 caret=31 applied=1 len=31',s0,30)
  # Home, Ctrl+K kills the copied line's text; Ctrl+Home, Ctrl+J joins lines 1 and 2.
  g['key']('home',r'\[quil.cursor.move\] old=31 new=17 len=31 dir=home ok=1')
  g['chord']('ctrl','k',r'\[quil.line.kill\] start=17 removed=14 applied=1 len=17')
  g['chord']('ctrl','home',r'\[quil.cursor.move\] old=17 new=0 len=17 dir=doc_start ok=1')
  g['chord']('ctrl','j',r'\[quil.line.join\] start=1 removed=1 applied=1 len=17')
  settle(g,60)
  g['q'].cmd('screendump',{'filename':str(out/'recovered.ppm')})
 except Exception as exc:failure=repr(exc)
 finally:
  if g:g['stop']()
 result.update(ok=failure is None,error=failure);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if failure:raise RuntimeError(failure)
 print(json.dumps(dict(ok=True,draft=result['boot1']['name'],recovered=result['boot2'])))

if __name__=='__main__':main()
