#!/usr/bin/env python3
"""Normal-image Quil document workflow through actual QMP input; never builds."""
import argparse,pathlib,hashlib,json,subprocess,time,re,shutil,os
from desktop_close_gate import QMP

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=pathlib.Path);p.add_argument('--source-disk',required=True,type=pathlib.Path);p.add_argument('--out',required=True,type=pathlib.Path);p.add_argument('--round',type=int,default=15);a=p.parse_args()
 iso=a.iso.resolve(strict=True);src=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh output required')
 disk=out/'nvme.img';shutil.copyfile(src,disk);sock=pathlib.Path('/tmp')/f'sex-quil-{os.getpid()}.sock';log=out/'serial.log'
 command=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{log}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown']
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
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[linen.disk.publish.done\]')
  focus(201)
  # Save snapshot while editing; the admitted revision must not clear newer edits.
  key('down',r'\[quil.palette.selected\] row=1')
  start=len(text());q.key_sequence(['ret','esc','z'])
  wait(r'\[quil.document.edit\] revision=\d+ pending=1 bytes=\d+',start)
  wait(r'\[quil.document.saved\] saved_revision=0 revision=1 dirty=1',start)
  # Create two useful named notes through the same real palette/name input.
  def palette(row,currently=False):
   if not currently:key('esc',r'kind=esc toggle_on=1')
   for i in range(row):key('down',rf'\[quil.palette.selected\] row={i+1}')
  def typing(value,name=False):
   for i in range(0,len(value),5):
    start=len(text());part=value[i:i+5];q.key_sequence([{' ':'spc'}.get(c,c)for c in part])
    if name:wait(r'\[quil.text.view\].*status=NAME\? '+re.escape(value[:i+len(part)])+r'#',start)
    else:wait(r'\[quil.document.edit\].*bytes='+str(i+len(part))+r'(?:\s|$)',start)
  def new_doc(content,name,currently=False):
   palette(3,currently);key('ret',r'\[quil.new.ok\] bytes=0');key('esc',r'kind=esc clear=1');typing(content)
   palette(1);key('ret',r'\[quil.name.mode\] on=1');typing(name,True)
   start=len(text());q.key('ret');m=wait(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)',start);wait(r'\[quil.document.saved\].*dirty=0',start);return dict(bytes=int(m[1]),hash=m[2],object=int(m[3]),name=name,content=content)
  first=new_doc('alpha note','alpha')
  # Save As retains the original alpha object and adopts a second object.
  # Existing text mode after name-entry save: add one character, then Save As.
  key('b',r'\[quil.document.edit\]')
  palette(4);key('ret',r'\[quil.name.mode\] on=1 save_as=1');typing('beta',True)
  start=len(text());q.key('ret');m=wait(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)',start);wait(r'\[quil.document.saved\].*dirty=0',start)
  second=dict(bytes=int(m[1]),hash=m[2],object=int(m[3]),name='beta',content='alpha noteb')
  if first['object']==second['object'] or first['hash']==second['hash']:raise RuntimeError('documents collapsed')
  q.cmd('screendump',{'filename':str(out/'saved.ppm')})
  # Close Quil9 and activate the BOOT-reserved fresh incarnation16.
  key('f11',r'\[shell.app.close.request\] target=9 ');wait(r'\[sext.app.pool.ready\] target=9 .*ok=1')
  key('grave_accent',r'\[shell.palette.item\] idx=0');key('j',r'\[shell.palette.select\] old=0 new=1');key('ret',r'\[shell.app.relaunch\] predecessor=9 successor=16 ');wait(r'\[shell.app.registered.reply\] target=16 .*committed=1')
  # Refresh Linen through its ordinary launcher action, then select actual rows.
  key('grave_accent',r'\[shell.palette.item\] idx=0');key('j',r'\[shell.palette.select\] old=0 new=1');key('j',r'\[shell.palette.select\] old=1 new=2');key('ret',r'\[linen.remote.snapshot.ok\] count=\d+ .*complete=1')
  opened=[]
  for expected in [first,second]:
   focus(200)
   for _ in range(20):
    key('j',r'\[shell.action.select_next_linen\] id=\d+')
    start=len(text());q.key('ret');end=time.monotonic()+1
    while time.monotonic()<end and '[linen.quil.disk_doc.intent]'not in text()[start:]:time.sleep(.05)
    m=re.search(r'\[linen.quil.disk_doc.intent\] object_id=\d+ path_id=(\d+) ok=1',text()[start:])
    if m and int(m[1])==expected['object']:
     loaded=wait(r'\[quil.persist.load.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc='+str(expected['object']),start)
     if (int(loaded[1]),loaded[2])!=(expected['bytes'],expected['hash']):raise RuntimeError('reopened bytes changed')
     opened.append(expected);break
    focus(200)
   else:raise RuntimeError('Linen selection never opened '+expected['name'])
  q.cmd('screendump',{'filename':str(out/'reopened.ppm')})
  result=dict(ok=True,round=a.round,iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),edit_during_save_dirty=True,documents=opened,reopened_in=16)
  (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
 except Exception as e:
  (out/'result.json').write_text(json.dumps(dict(ok=False,error=str(e)))+'\n');raise
 finally:
  if proc.poll()is None:
   try:proc.terminate();proc.wait(timeout=10)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
if __name__=='__main__':main()
