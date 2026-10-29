#!/usr/bin/env python3
"""Real QMP input and glyph proof for correlated Spindle storage jobs."""
import argparse,pathlib,hashlib,json,subprocess,time,re,shutil,os
from desktop_close_gate import QMP
from check_spindle_viewport_pixels import verify
from check_desktop_pixels import font_from_source

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=pathlib.Path);p.add_argument('--source-disk',required=True,type=pathlib.Path);p.add_argument('--out',required=True,type=pathlib.Path);p.add_argument('--round',type=int,default=9);a=p.parse_args()
 iso=a.iso.resolve(strict=True);src=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 disk=out/'nvme.img';shutil.copyfile(src,disk);sock=pathlib.Path('/tmp')/f'sex-probe-{os.getpid()}.sock';log=out/'serial.log'
 command=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{log}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown']
 (out/'manifest.json').write_text(json.dumps(dict(round=a.round,iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),input_disk_sha256=hashlib.sha256(src.read_bytes()).hexdigest(),command=command),indent=2)+'\n')
 proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT)
 def text():return log.read_text(errors='replace')if log.exists()else''
 def wait(pattern,start=0,timeout=180):
  end=time.monotonic()+timeout
  while time.monotonic()<end:
   if proc.poll()is not None:raise RuntimeError('guest exited')
   value=text();m=re.search(pattern,value[start:])
   if m:return m
   if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at',value):raise RuntimeError('guest fault')
   time.sleep(.1)
  raise RuntimeError('timeout: '+pattern)
 try:
  end=time.monotonic()+15
  while not sock.exists():
   if time.monotonic()>end:raise RuntimeError('no QMP')
   time.sleep(.1)
  q=QMP(sock);font=font_from_source((pathlib.Path(__file__).resolve().parents[1]/'servers/sexdisplay/src/main.rs').read_text())
  def key(code,pattern):
   start=len(text());q.key(code);return wait(pattern,start)
  def typing(command):
   for first in range(0,len(command),6):
    end=min(first+6,len(command));start=len(text());q.key_sequence([{' ':'spc','-':'minus'}.get(ch,ch)for ch in command[first:end]])
    wait(rf'\[spindle.cursor.view\] target=12 sequence=\d+ pos={end} len={end}(?:\s|$)',start)
  def cmd(command,pattern):
   typing(command);start=len(text());q.key('ret');result=wait(pattern,start)
   if a.round>=11 and not command.startswith('probe-'):wait(r'\[spindle.history.receipt\] target=12 task=12 kind=saved .*error=0',start)
   return result
  wait(r'\[spindle.history.receipt\] target=12 task=12 kind=loaded .*error=0')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[linen.disk.publish.done\]')
  key('scroll_lock',r'\[shell.focus.set\] id=153');wait(r'\[spindle.view.accepted\] target=12 ')
  stat=cmd('disk-stat 0',r'\[spindle.probe.result\] target=12 kind=done job=Stat\(0\) value=(\d+) error=0')[1]
  if a.round>=10:
   wait(r'\[spindle.history.receipt\] target=12 task=12 kind=saved .*error=0')
   cmd('disk',r'\[spindle.probe.result\] target=12 kind=done job=List value=[1-9]\d* error=0')
   wait(r'\[spindle.probe.entry\] object=0 name=sexfiles-proof-v1')
  if a.round>=11:
   created=cmd('mkdoc alphaprobe',r'\[spindle.probe.result\] target=12 kind=done job=Create\([^\r\n]*\) value=(\d+) error=0')
   object_id=int(created[1])
   length=33 if a.round==12 else 513
   for command,job in [(f'filldocx {object_id} {length}',rf'Fill\({object_id}, {length}, true\)'),(f'catdocx {object_id}',rf'Verify\({object_id}, true\)'),(f'truncdoc {object_id} 17',rf'Truncate\({object_id}, 17\)'),(f'catdocx {object_id}',rf'Verify\({object_id}, true\)'),(f'mvdoc {object_id} alpharenamed',rf'Rename\({object_id}, [^\r\n]*\)'),(f'rmdoc {object_id}',rf'Delete\({object_id}\)')]:
    cmd(command,rf'\[spindle.probe.result\] target=12 kind=done job={job} value=\d+ error=0')
  if a.round>=12:
   cmd('probe-retry',r'\[spindle.probe.control\] target=12 action=retry accepted=0')
   cmd('disk-stat 0',r'\[spindle.probe.result\] target=12 kind=done job=Stat\(0\) value=4096 error=0')
   cmd('probe-cancel',r'\[spindle.probe.control\] target=12 action=cancel accepted=1')
   wait(r'\[spindle.probe.result\] target=12 kind=cancelled job=Cancel value=0 error=0')
   if a.round>=13:
    wait(r'\[shell.spindle.status.accepted\] target=12 sequence=\d+ state=12 live=1')
    key('grave_accent',r'\[shell.palette.item\] idx=0')
    from check_launcher_pixels import verify as launcher
    end=time.monotonic()+90;errors=['not painted']
    while time.monotonic()<end:
     q.cmd('screendump',{'filename':str(out/'probe-launcher.ppm')});value=text()
     errors=launcher(value,(out/'probe-launcher.ppm').read_bytes(),font)
     rows=re.findall(r'\[launcher.text.row\] index=0 row=0 name=Open Spindle status=([^\r\n\[]*)',value)
     if not rows or rows[-1]!='DISK QUIESCENT / INSPECT':errors.append('missing quiescent probe label')
     if not errors:break
     time.sleep(.2)
    if errors:raise RuntimeError('probe launcher pixels: '+str(errors))
    (out/'launcher-status.json').write_text(json.dumps(dict(ok=True,errors=errors,label=rows[-1]))+'\n')
    key('grave_accent',r'\[command_palette.close\]')
   start=len(text());cmd('probe-retry',r'\[spindle.probe.control\] target=12 action=retry accepted=1')
   wait(r'\[spindle.probe.result\] target=12 kind=done job=Stat\(0\) value=4096 error=0',start)
  typing('echo responsive');errors=['not painted'];end=time.monotonic()+90
  while time.monotonic()<end:
   q.cmd('screendump',{'filename':str(out/'responsive.ppm')})
   errors=verify(text(),(out/'responsive.ppm').read_bytes(),font,12,'> echo responsive',False,False)
   if not errors:break
   time.sleep(.2)
  if errors:raise RuntimeError('input pixels: '+str(errors))
  close_evidence=None
  if a.round>=13:
   remaining=15
   while remaining:
    count=min(6,remaining);remaining-=count;start=len(text());q.key_sequence(['backspace']*count)
    wait(rf'\[spindle.cursor.view\] target=12 sequence=\d+ pos={remaining} len={remaining}(?:\s|$)',start)
   object_id=int(cmd('mkdoc closeprobe',r'\[spindle.probe.result\] target=12 kind=done job=Create\([^\r\n]*\) value=(\d+) error=0')[1])
   typing(f'filldocx {object_id} 65536');start=len(text());q.key('ret')
   wait(r'\[spindle.instance.command\] target=12 task=12 name=filldocx ok=1',start)
   wait(rf'\[files.probe.bound\] target=12 object={object_id} generation=\d+',start)
   typing('xyz');wait(r'\[spindle.probe.input\] target=12 pending=1 .*down=1',start)
   end=time.monotonic()+60;pending_errors=['not painted']
   while time.monotonic()<end:
    q.cmd('screendump',{'filename':str(out/'pending-input.ppm')})
    pending_errors=verify(text(),(out/'pending-input.ppm').read_bytes(),font,12,'> xyz',False,False)
    if not pending_errors:break
    time.sleep(.2)
   if pending_errors:raise RuntimeError('pending-input glyphs: '+str(pending_errors))
   q.key('f11');wait(r'\[spindle.probe.closed\] target=12 pending=1 terminal=1',start)
   wait(r'\[files.probe.retire\] target=12 lease=0 pending=0 terminal=1',start)
   wait(r'\[sext.app.pool.ready\] target=12 task=12 .*ok=1',start)
   if re.search(rf'\[spindle.probe.result\] target=12 kind=done job=Fill\({object_id}, 65536, true\)',text()[start:]):raise RuntimeError('long job completed before Close overlap proof')
   q.cmd('screendump',{'filename':str(out/'after-close.ppm')})
   close_evidence=dict(ok=True,object=object_id,pending_input_pixels=True,service_binding_retired=True,sext_pool_ready=True,scope='actual HID and Close during an admitted unfinished job; cancellation is not rollback')
  (out/'result.json').write_text(json.dumps(dict(ok=True,round=a.round,stat=int(stat),pixel_errors=errors,close_during_probe=close_evidence,input_during_probe=bool(re.search(r'\[spindle.probe.input\] target=12 pending=1 .*down=1',text())),scope='real task12 tagged stat/list and named-object job workflow, final glyphs, and versioned cancel/retry controls where enabled; host tests inject Full/stalls; no hard real-time claim'),indent=2)+'\n')
  print('[spindle.probe.gate] PASS')
 except Exception as e:
  (out/'result.json').write_text(json.dumps(dict(ok=False,errors=[str(e)]),indent=2)+'\n');raise
 finally:
  proc.terminate()
  try:proc.wait(timeout=5)
  except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
if __name__=='__main__':main()
