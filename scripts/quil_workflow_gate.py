#!/usr/bin/env python3
"""Normal-image Quil document workflow through actual QMP input; never builds."""
import argparse,pathlib,hashlib,json,subprocess,time,re,shutil,os,math
from alpha_evidence import Captures,contract,require_inputs,verify_frame,finish,replay,RTC,serial_text
from network_contention_gate import frozen_binding
from desktop_close_gate import QMP

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=pathlib.Path);p.add_argument('--source-disk',type=pathlib.Path);p.add_argument('--binding',required=True,type=pathlib.Path);p.add_argument('--check',action='store_true');p.add_argument('--model',choices=('e1000','e1000e'),default='e1000e');p.add_argument('--out',required=True,type=pathlib.Path);p.add_argument('--round',type=int,default=0);p.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');p.add_argument('--soak-seconds',type=float,default=0);p.add_argument('--rehearsal',action='store_true');p.add_argument('--typing-bursts',type=int,default=0);p.add_argument('--workspace-refresh',action='store_true');p.add_argument('--third-quil',action='store_true');p.add_argument('--capture-profile',action='store_true');p.add_argument('--find',action='store_true');p.add_argument('--toggle-stress',type=int,default=0);p.add_argument('--pending-io-close',action='store_true');a=p.parse_args()
 if a.pending_io_close and a.soak_seconds:p.error('pending device-I/O fixture is separate from sustained alpha')
 if not 0<=a.typing_bursts<=100:p.error('typing bursts must be 0..100')
 root=pathlib.Path(__file__).resolve().parents[1];iso=a.iso.resolve(strict=True);out=a.out.resolve();binding_path=a.binding.resolve(strict=True)
 if a.check:print(json.dumps(replay(root,iso,binding_path,out),indent=2));return
 if a.source_disk is None:p.error('native workflow needs --source-disk')
 if not math.isfinite(a.soak_seconds)or not 0<=a.soak_seconds<=7200:p.error('soak seconds must be finite and0..7200')
 src=a.source_disk.resolve(strict=True);out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh output required')
 bound=frozen_binding(root,iso,binding_path);pixel_contract=contract(root);require_inputs(root,binding_path,pixel_contract);shutil.copyfile(binding_path,out/'source-binding.json')
 disk=out/'nvme.img';shutil.copyfile(src,disk);sock=pathlib.Path('/tmp')/f'sex-quil-{os.getpid()}.sock';log=out/'serial.log';control_sock=pathlib.Path('/tmp')/f'sex-quil-control-{os.getpid()}.sock'
 command=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-rtc',RTC,'-m','512M','-vga','none','-device','VGA,xres='+a.resolution.split('x')[0]+',yres='+a.resolution.split('x')[1],'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{log}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',a.model+',netdev=peer,mac=52:54:00:12:34:56']
 if a.pending_io_close:command+=['-qmp',f'unix:{control_sock},server=on,wait=off','-D',str(out/'device-trace.log'),'-msg','timestamp=on']
 manifest=dict(evidence_version=2,round=a.round,binding=bound,iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),input_disk_sha256=hashlib.sha256(src.read_bytes()).hexdigest(),command=command,resolution=a.resolution,model=a.model,host_utc_start=time.time(),pixel_source_sha256=pixel_contract['source_sha256'],options=dict(soak_seconds=a.soak_seconds,rehearsal=a.rehearsal,find=a.find,typing_bursts=a.typing_bursts,workspace_refresh=a.workspace_refresh,third_quil=a.third_quil,toggle_stress=a.toggle_stress,pending_io_close=a.pending_io_close))
 (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
 proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT)
 captures=Captures(out,pixel_contract,tuple(map(int,a.resolution.split('x'))));failure=None;started=time.monotonic()
 def text():return serial_text(log)if log.exists()else''
 def stop_guest():
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=10)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
 def kill_guest():
  if proc.poll()is None:proc.kill()
 def health():
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if time.monotonic()-started>a.soak_seconds+1200:raise TimeoutError('bounded whole alpha workflow exceeded')
  if log.exists()and log.stat().st_size>64*1024*1024:raise RuntimeError('bounded alpha serial evidence exceeded')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',text()):raise RuntimeError('guest fault or diagnostic overflow')
 def wait(pattern,start=0,timeout=180):
  end=time.monotonic()+timeout
  while time.monotonic()<end:
   health();v=text();m=re.search(pattern,v[start:])
   if m:return m
   if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at',v):raise RuntimeError('guest fault')
   time.sleep(.05)
  raise RuntimeError('timeout: '+pattern)
 try:
  end=time.monotonic()+15
  while not sock.exists():
   if time.monotonic()>end:raise RuntimeError('QMP unavailable')
   time.sleep(.1)
  q=QMP(sock)
  def key(k,pattern):
   start=len(text());q.key_sequence([k]);return wait(pattern,start)
  from desktop_focus import focus_surface
  def focus(sid):return focus_surface(sid,text,key)
  if a.resolution=='1024x768':
   wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[linen.disk.publish.done\]')
  focus(201)
  from check_desktop_pixels import font_from_source
  from check_quil_ui_pixels import verify,view,wait_view
  from check_document_disk import recover
  font=font_from_source((pathlib.Path(__file__).resolve().parents[1]/'servers/sexdisplay/src/main.rs').read_text());resolution=tuple(map(int,a.resolution.split('x')));stages=[];capture_traces=[]
  stages=captures.frames
  def capture(name,expected='',target=9,kind='quil',region=None):
   began=time.monotonic();end=began+90;proof=dict(ok=False,errors=['not painted']);attempts=[];candidate=captures.candidate(name)
   while time.monotonic()<end:
    health();before=time.monotonic();q.cmd('screendump',{'filename':str(candidate)});dumped=time.monotonic();v=text();raw=candidate.read_bytes();read=time.monotonic()
    try:proof=verify_frame(v,raw,pixel_contract,resolution,kind,expected,target,region)
    except ValueError as error:proof=dict(ok=False,errors=[str(error)])
    verified=time.monotonic()
    if a.capture_profile:attempts.append(dict(start_seconds=before-began,screendump_seconds=dumped-before,read_seconds=read-dumped,verify_seconds=verified-read,errors=proof['errors'][:8]))
    if proof['ok']:break
    time.sleep(.15)
   if a.capture_profile:
    capture_traces.append(dict(name=name,passed=proof['ok'],elapsed_seconds=time.monotonic()-began,attempts=attempts));(out/'capture-profile.json').write_text(json.dumps(capture_traces,indent=2)+'\n')
   if not proof['ok']:raise RuntimeError(name+': '+str(proof['errors']))
   return captures.commit(name,candidate,v,proof,kind,expected,target,region,raw)
  def ctrl(k,pattern):
   # The chord's effect can precede its release under emulation. Wait for the
   # actual Ctrl break edge so later keys are never sent as Ctrl chords.
   start=len(text());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':k}],'hold-time':80});found=wait(pattern,start)
   wait(r'\[sexinput.ps2.scancode\] raw=0x9d(?=\n|$)',start);return found
  def park_pointer():
   start=len(text())
   q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':30000}},{'type':'abs','data':{'axis':'y','value':30000}}]})
   wait(r'\[shell.abs.normalize\] raw_x=30000 raw_y=30000 .*reason=(?:ok|duplicate_sample)',start)
  def toolbar(action,pattern):
   v=wait_view(text);per=4 if v['cols']>=32 else 2 if v['cols']>=16 else 1 if v['cols']>=8 else 0;scale=2 if v['w']>=256 else 1;i=action-1
   if not per or v['rows']<4//per+2:raise ValueError('toolbar is hidden in the actual short viewport')
   x=v['x']+8+((i%per)*8+1)*6*scale;y=v['y']+v['top']+(1+i//per)*9*scale+3
   start=len(text());q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(resolution[0]-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(resolution[1]-1))}}]})
   # Wait for the real USB tablet movement before submitting its button.
   wait(r'\[shell.pointer.abs\]|\[silk-shell.pointer.recv\].*class=',start)
   q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':True,'button':'left'}}]})
   wait(rf'\[quil.ui.pointer\] action={action} sequence=\d+',start)
   q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':False,'button':'left'}}]});result=wait(pattern,start);park_pointer();return result
  def typing(value,name=False):
   for i in range(0,len(value),5):
    start=len(text());part=value[i:i+5];q.key_sequence([{' ':'spc'}.get(c,c)for c in part])
    if name:wait(r'\[quil.text.view\].*status=Name: '+re.escape(value[:i+len(part)])+r'#',start)
    else:wait(r'\[quil.document.edit\]',start)
  wait(r'\[quil.ui.admitted\]')
  # Pointer Open selects the real Linen app, then keyboard returns to Quil.
  capture('startup','New')
  toolbar(2,r'\[shell.document.open\] target=9 linen=200');focus(201)
  toolbar(1,r'\[quil.new.ok\] bytes=0');typing('first note')
  capture('edited','first note#')
  find=None
  if a.find:
   # Bounded find: actual Ctrl+F prompt pixels, then exact selection receipts
   # for a forward match and a wrapped repeat. Selection pixels are not checked.
   key('home',r'\[quil.cursor.move\] .*dir=home ok=1')
   ctrl('f',r'\[quil.find.mode\] on=1 ');start=len(text());q.key_sequence(['n','o','t','e'])
   wait(r'\[quil.text.view\] target=9 .*status=.*Find: note#',start);capture('find-prompt','Find: note#')
   start=len(text());key('ret',r'\[quil.find\] query_bytes=4 start=6 end=10 wrapped=0 found=1 selected=1')
   wait(r'\[quil.selection\] generation=\d+ revision=\d+ bytes=10 anchor=6 caret=10',start)
   key('ret',r'\[quil.find\] query_bytes=4 start=6 end=10 wrapped=1 found=1 selected=1')
   start=len(text());q.key_sequence(['z']);key('ret',r'\[quil.find\] query_bytes=5 found=0')
   # Replace: the selection 6..10 is exactly one match of "note".
   start=len(text());q.key_sequence(['backspace','down','m','e','m','o'])
   wait(r'\[quil.text.view\] target=9 .*status=.*Find: note > memo#',start);capture('replace-prompt','Find: note > memo#')
   start=len(text());key('ret',r'\[quil.replace\] start=6 old_bytes=4 new_bytes=4 applied=1 ')
   wait(r'\[quil.find\] query_bytes=4 found=0',start)
   key('esc',r'\[quil.find.mode\] on=0 reason=escape');capture('replaced','first memo#')
   # One undo restores the document for the rest of the workflow.
   start=len(text());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'z'}],'hold-time':80})
   wait(r'\[sexinput.ps2.scancode\] raw=0x9d(?=\n|$)',start);capture('find-closed','first note#')
   # Typing over a selection would replace it; collapse to the end.
   key('end',r'\[quil.cursor.move\] .* dir=end ok=1')
   find=dict(prompt=True,forward=dict(start=6,end=10,wrapped=False),repeat=dict(start=6,end=10,wrapped=True),missing='notez',replace=dict(query='note',replacement='memo',start=6,undone=True),pixels_checked=['find-prompt','replace-prompt','replaced','find-closed'])
  ctrl('s',r'\[quil.name.mode\] on=1');typing('uialpha',True)
  automatic_lists=[]
  initial_count=int(re.findall(r'\[linen.remote.snapshot.ok\] count=(\d+)',text())[-1]) if a.workspace_refresh else 0
  cold_start=time.monotonic();start=len(text());q.key('ret');m=wait(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)',start);cold_first=time.monotonic()-cold_start
  first=dict(bytes=int(m[1]),hash=m[2],object=int(m[3]),name='uialpha',content='first note')
  wait(r'\[quil.document.saved\].*dirty=0',start);capture('saved','uialpha')
  if a.workspace_refresh:
   from workspace_refresh_probe import after_save
   automatic_lists.append(after_save(locals(),start,initial_count+1,'uialpha'))
  typing('b');toolbar(4,r'\[quil.name.mode\] on=1 save_as=1');typing('uibeta',True)
  cold_start=time.monotonic();start=len(text());q.key('ret');m=wait(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)',start);cold_second=time.monotonic()-cold_start;wait(r'\[quil.document.saved\].*dirty=0',start)
  second=dict(bytes=int(m[1]),hash=m[2],object=int(m[3]),name='uibeta',content='first noteb')
  if a.workspace_refresh:automatic_lists.append(after_save(locals(),start,initial_count+2,'uibeta'))
  # Save and a following edit in one actual hardware event stream.
  start=len(text());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}for k,down in [('ctrl',True),('s',True),('s',False),('ctrl',False),('z',True),('z',False)]]})
  wait(r'\[quil.document.edit\].*pending=1',start);wait(r'\[quil.document.saved\].*dirty=1',start);capture('newer-edit','first notebz#')
  # Pointer Save commits the newer editor revision.
  toolbar(3,r'\[quil.document.saved\].*dirty=0');capture('pointer-saved','uibeta')
  m=re.findall(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)',text())[-1];second.update(bytes=int(m[0]),hash=m[1],content='first notebz')
  # Real geometry changes, then minimize and restore through ordinary controls.
  key('grave_accent',r'\[shell.palette.item\] idx=0');key('k',r'\[shell.palette.select\] old=0 new=9');key('k',r'\[shell.palette.select\] old=9 new=8');key('ret',r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=201 ok=1');capture('zoomed','first notebz#')
  key('grave_accent',r'\[shell.palette.item\] idx=0');key('k',r'\[shell.palette.select\] old=0 new=9');key('k',r'\[shell.palette.select\] old=9 new=8');key('ret',r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=201 ok=1');capture('retiled','first notebz#')
  focus(201)
  before_min=wait_view(text)
  key('grave_accent',r'\[shell.palette.item\] idx=0');key('k',r'\[shell.palette.select\] old=0 new=9');key('ret',r'\[silk.lifecycle.minimize.hidden\] sid=201 frame=\d+')
  capture('minimized',kind='empty',region={k:before_min[k]for k in('x','y','w','h')})
  # Restore the existing live frame through the visible launcher action.
  key('grave_accent',r'\[shell.palette.item\] idx=0');key('k',r'\[shell.palette.select\] old=0 new=9');key('k',r'\[shell.palette.select\] old=9 new=8');key('k',r'\[shell.palette.select\] old=8 new=7');key('ret',r'\[silk.lifecycle.restore.ok\] frame=\d+ sid=201');focus(201);capture('restored','first notebz#')
  input_latency=None
  if a.typing_bursts:
   from quil_input_latency import run as measure_input
   input_latency=measure_input(locals(),a.typing_bursts)
  toggles=None
  if a.toggle_stress:
   # Alternate real focus between Quil and Spindle through the Spindle toggle;
   # every round trip must end focused, with no cancelled restore focus.
   for i in range(a.toggle_stress):
    focus(153);focus(201)
    if i%25==24:print(f'[toggle.stress.progress] round_trips={i+1}',flush=True)
   cancels=re.findall(r'\[launcher.focus.cancel\] [^\n]*',text())
   toggles=dict(round_trips=a.toggle_stress,cancels=cancels)
   if cancels:raise RuntimeError('cancelled launcher focus: '+str(cancels[:4]))
  pending_io=None
  if a.pending_io_close:
   from pending_io_close import run as pending_run
   pending_io=pending_run(locals())
  soak=None
  if a.soak_seconds:
   from alpha_session import run
   soak=run(locals(),a.soak_seconds,acceptance=not a.rehearsal,third_quil=a.third_quil)
  stop_guest()
  if pending_io:
   from pending_io_close import BASELINE,SNAPSHOT
   durable=recover(disk.read_bytes(),pending_io['document']);payload=durable.pop('payload')
   if payload not in (BASELINE,SNAPSHOT):raise ValueError('terminal pending fixture recovered partial/unsaved payload')
   pending_io['terminal_document']=durable
  checked=[]
  for expected in [first,second]:
   r=recover(disk.read_bytes(),expected['object'])
   if r.pop('payload')!=expected['content'].encode():raise RuntimeError('native saved bytes differ')
   checked.append(r)
  result=dict(ok=True,acceptance=bool(a.soak_seconds>=1800 and not a.rehearsal),evidence_version=2,round=a.round,soak=soak,pending_io_close=pending_io,find=find,toggles=toggles,automatic_lists=automatic_lists,input_latency=input_latency,cold_save_seconds=[cold_first,cold_second],iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),resolution=resolution,documents=[first,second],disk_documents=checked,stages=stages,keyboard=True,pointer=True,edit_during_save_dirty=True)
  (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 except BaseException as e:
  failure=type(e).__name__+': '+str(e)
  (out/'result.json').write_text(json.dumps(dict(ok=False,error=str(e)))+'\n');raise
 finally:
  stop_guest();sock.unlink(missing_ok=True);control_sock.unlink(missing_ok=True);finish(out,manifest,src,captures,failure)
 verified=replay(root,iso,binding_path,out);(out/'verification.json').write_text(json.dumps(verified,indent=2)+'\n');print(json.dumps(dict(ok=True,acceptance=verified['acceptance'],verified_captures=verified['verified_captures'],cycles=verified['cycles'])))
if __name__=='__main__':main()
