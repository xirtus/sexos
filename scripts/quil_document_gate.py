#!/usr/bin/env python3
"""Normal-image Quil document workflow through actual QMP input; never builds."""
import argparse,pathlib,hashlib,json,subprocess,time,re,shutil,os
from desktop_close_gate import QMP

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=pathlib.Path);p.add_argument('--source-disk',required=True,type=pathlib.Path);p.add_argument('--out',required=True,type=pathlib.Path);p.add_argument('--round',type=int,default=14);a=p.parse_args()
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
  # Initial palette row0. Save a known normal document, then load while editing.
  key('down',r'\[quil.palette.selected\] row=1')
  key('ret',r'\[quil.persist.save.ok\]')
  key('down',r'\[quil.palette.selected\] row=2')
  start=len(text());q.key_sequence(['ret','esc','x'])
  wait(r'\[quil.document.edit\] revision=\d+ pending=1 bytes=\d+',start)
  wait(r'\[quil.document.result\] outcome=Loaded .*buffer_retained=1',start)
  # Retry explicitly; a complete load now replaces the edited buffer.
  key('esc',r'kind=esc toggle_on=1')
  key('down',r'\[quil.palette.selected\] row=1')
  key('down',r'\[quil.palette.selected\] row=2')
  key('ret',r'\[quil.persist.load.ok\]')
  q.cmd('screendump',{'filename':str(out/'loaded.ppm')})
  # Real Linen selection routes the exact disk object through the shell.
  focus(200)
  for _ in range(20):
   key('j',r'\[shell.action.select_next_linen\] id=\d+')
   current=re.findall(r'\[linen.object_select.current\] id=(\d+)',text())[-1]
   start=len(text());q.key('ret')
   end=time.monotonic()+1
   while time.monotonic()<end:
    v=text()[start:]
    if '[linen.quil.disk_doc.intent]'in v:break
    time.sleep(.05)
   m=re.search(r'\[linen.quil.disk_doc.intent\] object_id=\d+ path_id=(\d+) ok=1',text()[start:])
   if m and int(m[1])==2:
    wait(r'\[quil.persist.load.ok\] bytes=\d+ hash=0x[0-9a-f]+ doc=2',start);break
   focus(200)
  else:raise RuntimeError('Linen did not open document2')
  focus(201)
  # Palette remains on row2. Close is a real shell F11 action during load.
  start=len(text());q.key_sequence(['ret','f11'])
  wait(r'\[quil.document.close\] pending=1',start)
  wait(r'\[files.document.retire\] target=9 staged=0 pending=0 terminal=1',start)
  wait(r'\[sext.app.pool.ready\] target=9 .*ok=1',start)
  q.cmd('screendump',{'filename':str(out/'closed.ppm')})
  result=dict(ok=True,round=a.round,iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),staged_revision_rejection=True,linen_exact_object=2,close_pending=True)
  (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
 except Exception as e:
  (out/'result.json').write_text(json.dumps(dict(ok=False,error=str(e)))+'\n');raise
 finally:
  if proc.poll()is None:
   try:proc.terminate();proc.wait(timeout=10)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
if __name__=='__main__':main()
