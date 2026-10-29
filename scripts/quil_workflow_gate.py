#!/usr/bin/env python3
"""Normal-image Quil document workflow through actual QMP input; never builds."""
import argparse,pathlib,hashlib,json,subprocess,time,re,shutil,os
from desktop_close_gate import QMP

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=pathlib.Path);p.add_argument('--source-disk',required=True,type=pathlib.Path);p.add_argument('--out',required=True,type=pathlib.Path);p.add_argument('--round',type=int,default=17);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 iso=a.iso.resolve(strict=True);src=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh output required')
 disk=out/'nvme.img';shutil.copyfile(src,disk);sock=pathlib.Path('/tmp')/f'sex-quil-{os.getpid()}.sock';log=out/'serial.log'
 command=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-vga','none','-device','VGA,xres='+a.resolution.split('x')[0]+',yres='+a.resolution.split('x')[1],'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{log}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown']
 (out/'manifest.json').write_text(json.dumps(dict(round=a.round,iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),input_disk_sha256=hashlib.sha256(src.read_bytes()).hexdigest(),command=command),indent=2)+'\n')
 proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT)
 def text():return log.read_text(errors='replace')if log.exists()else''
 def wait(pattern,start=0,timeout=180):
  end=time.monotonic()+timeout
  while time.monotonic()<end:
   if proc.poll()is not None:raise RuntimeError('guest exited')
   v=text();m=re.search(pattern,v[start:])
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
   start=len(text());q.key(k);return wait(pattern,start)
  def focus(sid):
   for _ in range(16):
    ids=re.findall(r'\[shell.focus.set\] id=(\d+)',text())
    if ids and int(ids[-1])==sid:return
    key('tab',r'\[shell.focus.set\] id=\d+')
   raise RuntimeError('focus failed')
  if a.resolution=='1024x768':
   wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[linen.disk.publish.done\]')
  focus(201)
  from check_desktop_pixels import font_from_source
  from check_quil_ui_pixels import verify,view
  from check_document_disk import recover
  font=font_from_source((pathlib.Path(__file__).resolve().parents[1]/'servers/sexdisplay/src/main.rs').read_text());resolution=tuple(map(int,a.resolution.split('x')));stages=[]
  def capture(name,expected):
   end=time.monotonic()+90;errors=['not painted']
   while time.monotonic()<end:
    q.cmd('screendump',{'filename':str(out/(name+'.ppm'))});v=text();errors=verify(v,(out/(name+'.ppm')).read_bytes(),font,expected,resolution=resolution)
    if not errors:break
    time.sleep(.15)
   if errors:raise RuntimeError(name+': '+str(errors))
   stages.append(dict(name=name,view=view(v),pixel_errors=errors))
  def ctrl(k,pattern):
   start=len(text());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':k}],'hold-time':80});return wait(pattern,start)
  def toolbar(action,pattern):
   v=view(text());per=4 if v['cols']>=20 else 2 if v['cols']>=10 else 1;scale=2 if v['w']>=256 else 1;i=action-1
   x=v['x']+8+((i%per)*5+1)*6*scale;y=v['y']+v['top']+(1+i//per)*9*scale+3
   start=len(text());q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(resolution[0]-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(resolution[1]-1))}}]})
   # Wait for the real USB tablet movement before submitting its button.
   wait(r'\[shell.pointer.abs\]|\[silk-shell.pointer.recv\].*class=',start)
   q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':True,'button':'left'}}]})
   wait(rf'\[quil.ui.pointer\] action={action} sequence=\d+',start)
   q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':False,'button':'left'}}]});return wait(pattern,start)
  def typing(value,name=False):
   for i in range(0,len(value),5):
    start=len(text());part=value[i:i+5];q.key_sequence([{' ':'spc'}.get(c,c)for c in part])
    if name:wait(r'\[quil.text.view\].*status=NAME\? '+re.escape(value[:i+len(part)])+r'#',start)
    else:wait(r'\[quil.document.edit\]',start)
  wait(r'\[quil.ui.admitted\]')
  # Pointer Open selects the real Linen app, then keyboard returns to Quil.
  capture('startup','NEW')
  toolbar(2,r'\[shell.document.open\] target=9 linen=200');focus(201)
  toolbar(1,r'\[quil.new.ok\] bytes=0');typing('first note')
  capture('edited','first note#')
  ctrl('s',r'\[quil.name.mode\] on=1');typing('uialpha',True)
  start=len(text());q.key('ret');m=wait(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)',start)
  first=dict(bytes=int(m[1]),hash=m[2],object=int(m[3]),name='uialpha',content='first note')
  wait(r'\[quil.document.saved\].*dirty=0',start);capture('saved','uialpha')
  typing('b');toolbar(4,r'\[quil.name.mode\] on=1 save_as=1');typing('uibeta',True)
  start=len(text());q.key('ret');m=wait(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)',start);wait(r'\[quil.document.saved\].*dirty=0',start)
  second=dict(bytes=int(m[1]),hash=m[2],object=int(m[3]),name='uibeta',content='first noteb')
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
  before_min=view(text())
  key('grave_accent',r'\[shell.palette.item\] idx=0');key('k',r'\[shell.palette.select\] old=0 new=9');key('ret',r'\[silk.lifecycle.minimize.hidden\] sid=201 frame=\d+')
  from check_disk_persistence import ppm_rgb
  end=time.monotonic()+90
  while True:
   q.cmd('screendump',{'filename':str(out/'minimized.ppm')});width,height,pixels=ppm_rgb((out/'minimized.ppm').read_bytes(),min_width=1,min_height=1)
   ink=sum(pixels[(y*width+x)*3:(y*width+x)*3+3]==b'\xe0\xf0\xff'for y in range(before_min['y'],before_min['y']+before_min['h'])for x in range(before_min['x'],before_min['x']+before_min['w']))
   if ink==0:break
   if time.monotonic()>end:raise RuntimeError('minimized Quil ink remains')
   time.sleep(.15)
  # Restore the existing live frame through the visible launcher action.
  key('grave_accent',r'\[shell.palette.item\] idx=0');key('k',r'\[shell.palette.select\] old=0 new=9');key('k',r'\[shell.palette.select\] old=9 new=8');key('k',r'\[shell.palette.select\] old=8 new=7');key('ret',r'\[silk.lifecycle.restore.ok\] frame=\d+ sid=201');focus(201);capture('restored','first notebz#')
  checked=[]
  for expected in [first,second]:
   r=recover(disk.read_bytes(),expected['object'])
   if r.pop('payload')!=expected['content'].encode():raise RuntimeError('native saved bytes differ')
   checked.append(r)
  result=dict(ok=True,round=17,iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),resolution=resolution,documents=[first,second],disk_documents=checked,stages=stages,keyboard=True,pointer=True,edit_during_save_dirty=True)
  (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
 except Exception as e:
  (out/'result.json').write_text(json.dumps(dict(ok=False,error=str(e)))+'\n');raise
 finally:
  if proc.poll()is None:
   try:proc.terminate();proc.wait(timeout=10)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
if __name__=='__main__':main()
