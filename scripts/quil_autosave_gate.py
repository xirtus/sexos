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

FIRST=['d','r','a','f','t','spc','o','n','e']
SECOND=['spc','a','n','d','spc','t','w','o']
SCOPE='Native QEMU power-cut recovery of a Quil idle draft through the held document route and Linen; no claim about physical media flush ordering.'

def boot(iso,disk,out,tag,resolution):
 serial=out/f'{tag}-serial.log';sock=Path('/tmp')/f'sex-autosave-{os.getpid()}-{tag}.sock'
 cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku,+rdseed','-rtc',RTC_NORMAL,'-m','512M','-vga','none','-device','VGA,xres='+resolution.replace('x',',yres='),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none']
 proc=subprocess.Popen(cmd,stdout=(out/f'{tag}-qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic()
 def text():return serial.read_text(errors='replace')if serial.exists()else''
 def health():
  if time.monotonic()-began>300:raise TimeoutError(tag+' bounded 300 s')
  if proc.poll()is not None:raise RuntimeError(tag+' guest exited')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',text()):raise RuntimeError(tag+' guest failure')
 def wait(pattern,start=0,timeout=120):
  d=time.monotonic()+timeout
  while time.monotonic()<d:
   health();m=re.search(pattern,text()[start:])
   if m:return m
   time.sleep(.02)
  raise TimeoutError(f'{tag}: {pattern}')
 d=time.monotonic()+15
 while not sock.exists():
  health()
  if time.monotonic()>d:raise TimeoutError('QMP unavailable')
  time.sleep(.05)
 q=QMP(sock)
 if resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret')
 wait(r'\[shell.app.registered.reply\] target=15 .*committed=1',timeout=180);wait(r'\[linen.disk.publish.done\]',timeout=180)
 def key(name,pattern):s=len(text());q.key(name);return wait(pattern,s)
 def chord(mod,k,pattern):
  s=len(text());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,(mod,k)),(False,(k,mod)))for x in ks]});return wait(pattern,s)
 def stop():
  if proc.poll()is None:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
 return dict(q=q,text=text,wait=wait,key=key,chord=chord,stop=stop,focus=lambda sid:focus_surface(sid,text,key))

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
  settle(g,60)
  g['q'].cmd('screendump',{'filename':str(out/'recovered.ppm')})
 except Exception as exc:failure=repr(exc)
 finally:
  if g:g['stop']()
 result.update(ok=failure is None,error=failure);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if failure:raise RuntimeError(failure)
 print(json.dumps(dict(ok=True,draft=result['boot1']['name'],recovered=result['boot2'])))

if __name__=='__main__':main()
