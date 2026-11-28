#!/usr/bin/env python3
"""Drivers that crash or hang start again by themselves, and the work they were doing goes on.

One boot of a crash-probe image (SEXOS_CRASH_PROBE=1) on an NVMe disk, with a NIC, the
USB tablet and a sound card. Each round stops one driver in the middle of real use:

  disk (Drive)   killed mid-request while a 200 KB download is written, then while a file
                 is read back; then left hung mid-request until the kernel's hang watch
                 ends it. Every file must read back byte for byte (size + FNV-1a 64
                 computed here on the host), Files must say it sent the lost request
                 again, and Files itself must never restart.
  USB            killed while the tablet moves; it must find the tablet again and pointer
                 reports must reach Input after.
  Input          killed mid-request; typed keys must still run commands after.
  sound (Bell)   killed mid-request; a beep must still play after.
  Network        killed mid-download; the next download must arrive whole.
  Files (last)   left stuck mid-request: the desk watch ends it after 60 s, it starts
                 again and files read back and save as before.

Usage: python3 scripts/driver_restart_gate.py --iso sexos-v1.0.0.iso --source-disk blank.img --out DIR
"""
import argparse,http.server,json,os,re,shutil,threading,time
from pathlib import Path

BLOB=bytes((i*7+i//251)%256 for i in range(200_000))
def fnv(data):
 h=0xcbf29ce484222325
 for b in data:h=((h^b)*0x100000001b3)&0xffffffffffffffff
 return f'{h:016x}'
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path!='/blob':self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Type','application/octet-stream');self.send_header('Content-Length',str(len(BLOB)));self.end_headers();self.wfile.write(BLOB)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ.setdefault('SEXOS_GATE_NIC','-netdev user,id=n0 -device e1000e,netdev=n0')
os.environ.setdefault('SEXOS_GATE_DEVICES','-audiodev none,id=snd0 -device virtio-sound-pci,audiodev=snd0')
os.environ.setdefault('SEXOS_GATE_BOUND','2400')
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus',':':'semicolon'}
SPINDLE_SURFACE=153

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path)
 p.add_argument('--resolution',default='1280x800');p.add_argument('--only',default='',help='comma list of rounds to run (disk,hang,usb,input,bell,network,fileshang)');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 want=set(filter(None,a.only.split(',')))or{'disk','hang','usb','input','bell','network','fileshang'}
 g=boot(a.iso,disk,a.out,'drivers',a.resolution);t=g['text'];q=g['q'];result={'rounds':{}}
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def line(text,pattern,timeout=180):
  g['focus'](SPINDLE_SURFACE);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 base=f'http://10.0.2.2:{PORT}/blob';expect_sum=f'{len(BLOB)} bytes, fnv {fnv(BLOB)}'
 def fetch(path,ok=True):
  m=line(f'fetch {base} {path}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',300)
  if ok:assert m[1]==f'Saved {len(BLOB)} bytes',m[1]
  return m[1]
 def check(path):
  m=line(f'sum {path}',r'\[spindle.fs.line\] (\d+ bytes, fnv [0-9a-f]{16}|[^\n]*)',120)
  assert m[1]==expect_sum,f'{path}: {m[1]} (want {expect_sum})';return m[1]
 def crash(name,pd,how=''):
  s=len(t());line(f'crash {name} {how}'.strip(),rf'\[crash.probe.service\] pd={pd}\b',60);return s
 def restarted(pd,s,timeout=180):
  return g['wait'](rf'\[service.restarted\] pd={pd} task=\d+ restarts=(\d+) ',s,timeout)
 def tablet(n=12):
  for i in range(n):
   q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':4000+i*900}},{'type':'abs','data':{'axis':'y','value':9000+i*500}}]});time.sleep(.1)
 began=time.monotonic()
 def save(ok,why=''):
  result.update(ok=ok,why=why,minutes=round((time.monotonic()-began)/60,1));(a.out/'driver_restart.json').write_text(json.dumps(result,indent=1))
 try:
  g['wait'](r'\[sexdrive.boot.serving\]',0,120);g['wait'](r'\[boot.service.restartable\] pd=2 ',0,5);g['wait'](r'\[boot.service.restartable\] pd=5 ',0,5)
  fetch('/blob0');check('/blob0')
  if 'disk' in want:
   # Killed while it holds a request, during a write and during a read.
   s=crash('drive',2,'busy');fetch('/blob1');m=restarted(2,s);g['wait'](r'\[sexfiles.block.resend\]',s,60);check('/blob1')
   s=crash('drive',2,'busy');check('/blob0');restarted(2,s,120);g['wait'](r'\[sexfiles.block.resend\]',s,60);check('/blob1')
   result['rounds']['disk']=dict(restarts=int(m[1]),flr=bool(re.search(r'\[device.reset.flr\] pd=2 ',t())))
  if 'hang' in want:
   # Stuck mid-request: the hang watch (45 s for Drive) ends it, the controller is reset
   # and Files sends the request again. Spindle's own request may give up meanwhile.
   s=crash('drive',2,'hang');line(f'sum /blob0',r'\[spindle.fs.(line|expired)\]',120)
   g['wait'](r'\[crash.probe.hang\] pd=2',s,120);h=g['wait'](r'\[service.hung\] pd=2 held_ticks=(\d+)',s,180)
   restarted(2,s,120);g['wait'](r'\[sexfiles.block.resend\]',s,60);time.sleep(3);check('/blob0');fetch('/blob2');check('/blob2')
   result['rounds']['hang']=dict(held_ticks=int(h[1]))
  if 'usb' in want:
   tablet(4);s=len(t());line('crash usb',r'\[crash.probe.service\] pd=5\b',60);tablet()
   restarted(5,s);g['wait'](r'\[sexusb.enum.summary\][^\n]*pointer_devices=[1-9]',s,120);r=len(t());time.sleep(2);tablet()
   g['wait'](r'\[sexinput.pointer.(recv|raw|forward)',r,60);result['rounds']['usb']='ok'
  if 'input' in want:
   s=crash('input',4,'busy');tablet();keys('x');restarted(4,s,120);g['wait'](r'\[sexinput.restarted\]',s,60);time.sleep(2)
   q.key('backspace');line('echo still-typing',r'still-typing',60);result['rounds']['input']='ok'
  if 'bell' in want:
   s=crash('bell',10,'busy');line('beep 440 300',r'\[spindle.beep\]',60);restarted(10,s,120);g['wait'](r'\[sexbell.restarted\]',s,60);time.sleep(2)
   m=line('beep 880 200',r'\[spindle.beep\] hz=880 ms=200 volume=\d+ status=(\d+)',60);assert m[1]=='0',m[0];result['rounds']['bell']='ok'
  if 'network' in want:
   s=crash('network',13,'busy');fetch('/blob3',ok=False);restarted(13,s,120);time.sleep(5);fetch('/blob4');check('/blob4');result['rounds']['network']='ok'
  files=re.findall(r'\[service.restarted\] pd=11 ',t())
  assert not files,'Files restarted: a driver restart must not take Files down'
  if 'fileshang' in want:
   # Files stuck mid-request (parked, its code making no syscalls): the desk watch ends it
   # after 60 s and Files starts again; earlier rounds, where Files was only busy or
   # waiting on Drive, must never have tripped it (checked just above).
   slow=len(re.findall(r'\[service.slow\] pd=11 ',t()))
   s=crash('files',11,'hang');line('sum /blob0',r'\[spindle.fs.(line|expired)\]',120)
   g['wait'](r'\[crash.probe.hang\] pd=11',s,120);h=g['wait'](r'\[service.hung\] pd=11 held_ticks=(\d+) queued=\d+ desk=1',s,300)
   assert int(h[1])>60_000,h[0]
   restarted(11,s,180);time.sleep(5);check('/blob0');fetch('/blob5');check('/blob5')
   result['rounds']['fileshang']=dict(held_ticks=int(h[1]),slow_before=slow)
  save(True);print('driver restart gate: PASS',json.dumps(result['rounds']))
 except BaseException as e:
  save(False,repr(e));raise
 finally:g['stop']()

if __name__=='__main__':main()
