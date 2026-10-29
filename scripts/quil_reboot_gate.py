#!/usr/bin/env python3
"""Normal-image Quil document workflow through actual QMP input; never builds."""
import argparse,pathlib,hashlib,json,subprocess,time,re,shutil,os
from desktop_close_gate import QMP

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=pathlib.Path);p.add_argument('--source-disk',required=True,type=pathlib.Path);p.add_argument('--out',required=True,type=pathlib.Path);p.add_argument('--round',type=int,default=16);p.add_argument('--boot-a',required=True,type=pathlib.Path);a=p.parse_args()
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
  source_result=json.loads(a.boot_a.joinpath('result.json').read_text())
  if not source_result.get('ok') or source_result['iso_sha256']!=hashlib.sha256(iso.read_bytes()).hexdigest():raise RuntimeError('Boot A must pass with the same image')
  first,second=source_result['documents']
  opened=[]
  for expected in [first,second]:
   focus(200)
   for _ in range(20):
    key('j',r'\[shell.action.select_next_linen\] id=\d+')
    from quil_third_probe import open_selected
    observed=open_selected(dict(q=q,wait=wait,text=text))
    if observed and observed['object']==expected['object']:
     if (observed['bytes'],observed['hash'])!=(expected['bytes'],expected['hash']):raise RuntimeError('reopened bytes changed')
     opened.append(expected);break
    focus(200)
   else:raise RuntimeError('Linen selection never opened '+expected['name'])
  q.cmd('screendump',{'filename':str(out/'reopened.ppm')})
  from check_document_disk import recover
  checked=[]
  for expected in opened:
   r=recover(disk.read_bytes(),expected['object'])
   if r.pop('payload')!=expected['content'].encode():raise RuntimeError('reboot disk bytes mismatch')
   checked.append(r)
  result=dict(ok=True,round=a.round,iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),documents=opened,reopened_in=9,boot_a=str(a.boot_a.resolve()),disk_documents=checked)
  (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
 except Exception as e:
  (out/'result.json').write_text(json.dumps(dict(ok=False,error=str(e)))+'\n');raise
 finally:
  if proc.poll()is None:
   try:proc.terminate();proc.wait(timeout=10)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
if __name__=='__main__':main()
