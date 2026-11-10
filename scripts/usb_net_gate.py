#!/usr/bin/env python3
"""USB network adapters: a PC with no network card of its own gets online through a USB
Ethernet adapter (CDC Ethernet) plugged in after boot. Spindle fetches a file over it,
the adapter is pulled out and plugged back in, and a second fetch works."""
import argparse,hashlib,http.server,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
BODY=bytes(range(256))*512  # 128 KiB
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path!='/blob':self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(BODY)));self.end_headers();self.wfile.write(BODY)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus',':':'semicolon'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 server=Quiet(('127.0.0.1',0),Handler);port=server.server_address[1]
 threading.Thread(target=server.serve_forever,daemon=True).start()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 # The network exists, but no card is on it: only the USB adapter, plugged in later.
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0'
 os.environ.pop('SEXOS_GATE_DEVICES',None)
 os.environ.setdefault('SEXOS_GATE_BOUND','600')
 (a.out/'usbnet-serial.log').unlink(missing_ok=True)
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,a.out,'usbnet',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   shift=k.isupper() or k in ':_'
   code={'_':'minus'}.get(k,KEYS.get(k,k.lower()))
   if shift:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':code}],'hold-time':80})
   else:q.key(code)
   time.sleep(.06)
 def line(text):g['focus'](153);time.sleep(.5);keys(text);q.key('ret')
 def fetch(name):
  # Spindle can drop the first keys while it opens; the command is retried.
  for attempt in range(3):
   s=len(t());line(f'fetch http://10.0.2.2:{port}/blob {name}')
   try:m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,120);break
   except TimeoutError:
    if attempt==2:raise
  assert m[1]==f'Saved {len(BODY)} bytes',m[1]
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  assert '[sexnet.usb.link]' not in t()
  s=len(t());q.cmd('device_add',{'driver':'usb-net','bus':'xhci.0','netdev':'n0','id':'usbnet'})
  g['wait'](r'\[sexusb\.ether\] slot=\d+ [^\n]*ok=1',s,60)
  g['wait'](r'\[sexnet\.usb\.link\] mac=',s,60)
  g['wait'](r'\[sexnet\.transport\.ready\]',s,60)
  fetch('one.bin')
  # Pulled out: SexNet forgets it. Back in: online again.
  s=len(t());q.cmd('device_del',{'id':'usbnet'})
  g['wait'](r'\[sexusb\.ether\.gone\]',s,60);g['wait'](r'\[sexnet\.usb\.link\.gone\]',s,60)
  s=len(t())
  for _ in range(40):
   try:q.cmd('device_add',{'driver':'usb-net','bus':'xhci.0','netdev':'n0','id':'usbnet2'});break
   except RuntimeError:time.sleep(.5)
  else:raise RuntimeError('the adapter could not be plugged back in')
  g['wait'](r'\[sexnet\.usb\.link\] mac=',s,60)
  fetch('two.bin')
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
 finally:g['stop']()
 print({'ok':True,'bytes':len(BODY),'sha256':hashlib.sha256(BODY).hexdigest()[:16]})
if __name__=='__main__':main()
