#!/usr/bin/env python3
"""Real QEMU Ethernet backend capture; host frames are explicit peer fixtures.

No TAP, root, host network changes, mock guest producer or online-stack claim.
"""
import argparse,hashlib,json,os,re,shutil,socket,struct,subprocess,time
from pathlib import Path
from check_nic_command import inspect

LOCAL=bytes([10,0,2,15]);GATEWAY=bytes([10,0,2,2]);PEER=bytes([2,0,0,0,0,1])
def fingerprint(b):
 h=0xcbf29ce484222325
 for byte in b:h=((h^byte)*0x100000001b3)&0xffffffffffffffff
 return h

def packet(dst,src,opcode,sender_ip,target_mac,target_ip,pad=b'\0'*18):
 return dst+src+b'\x08\x06'+struct.pack('!HHBBH',1,0x800,6,4,opcode)+src+sender_ip+target_mac+target_ip+pad

def query(mac):return packet(b'\xff'*6,mac,1,LOCAL,b'\0'*6,GATEWAY)
def reply(mac,peer,ip):return packet(peer,mac,2,LOCAL,peer,ip)
def validate(text,exchanges,mac):
 errors=list(inspect(text,1,True,True)['errors'])
 rx=[tuple(int(x,16 if i==3 else 10)for i,x in enumerate(m.groups()))for m in re.finditer(r'\[sexnet.ether.rx\] sequence=(\d+) index=(\d+) length=(\d+) hash=(0x[0-9a-f]+)',text)]
 for kind in ['submit','done']:
  actual=[tuple(int(x,16 if i==3 else 10)for i,x in enumerate(m.groups()))for m in re.finditer(r'\[sexnet.ether.tx\.'+kind+r'\] sequence=(\d+) index=(\d+) length=(\d+) hash=(0x[0-9a-f]+)',text)]
  wanted=[(i+1,i%8,len(bytes.fromhex(e['guest_tx'])),fingerprint(bytes.fromhex(e['guest_tx'])))for i,e in enumerate(exchanges)]
  if actual!=wanted:errors.append('actual TX '+kind+' differs from independently captured full frames/indices')
 wanted=[(i+1,i%8,len(bytes.fromhex(e['peer_fixture'])),fingerprint(bytes.fromhex(e['peer_fixture'])))for i,e in enumerate(exchanges)]
 if rx!=wanted:errors.append('actual RX bytes/length/index differ from explicit peer fixture')
 if len(exchanges)!=25:errors.append('requires startup gateway exchange plus24 request/reply exchanges')
 for i,e in enumerate(exchanges):
  output=bytes.fromhex(e['guest_tx']);incoming=bytes.fromhex(e['peer_fixture'])
  if i==0:
   if output!=query(mac)or incoming!=packet(mac,PEER,2,GATEWAY,mac,LOCAL):errors.append('wrong real startup query/gateway fixture')
  else:
   ip=bytes([10,0,2,32+i]);padding=bytes(((i*17+n)&255)for n in range(18))
   if incoming!=packet(b'\xff'*6,PEER,1,ip,b'\0'*6,LOCAL,padding)or output!=reply(mac,PEER,ip):errors.append('wrong full peer request/guest response at exchange'+str(i))
 if '[app.surface.presented] target=15 surface=300'not in text:errors.append('cached desktop app did not present while driver polls')
 if '[sexnet.ether.rx.drop]'in text or '[sexnet.arp.drop]'in text:errors.append('unexpected loss in stop-and-wait native exchange')
 return dict(ok=not errors,errors=errors,exchanges=len(exchanges),tx_descriptor_wraps=len(exchanges)//8,rx_descriptor_wraps=len(exchanges)//8,
             scope='Actual selected QEMU NIC and held BOOT DMA queues exchange exact real Ethernet/ARP bytes. Host-generated peer packets are explicit fixtures. No IP/DNS/TCP/HTTP/TLS, IRQ idle-wait, physical-hardware or hostile-DMA containment proof.')

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--model',choices=['e1000','e1000e'],required=True);a=p.parse_args()
 iso=a.iso.resolve(strict=True);src=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh output required')
 disk=out/'nvme.img';shutil.copyfile(src,disk);serial=out/'serial.log';captures=[];exchanges=[]
 server=socket.socket();server.bind(('127.0.0.1',0));server.listen(1);server.settimeout(30);port=server.getsockname()[1]
 cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-cdrom',str(iso),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev',f'socket,id=peer,connect=127.0.0.1:{port}','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56']
 (out/'manifest.json').write_text(json.dumps(dict(iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),input_disk_sha256=hashlib.sha256(src.read_bytes()).hexdigest(),command=cmd,qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),fixture_scope='Host-created ARP frames through loopback socket backend; captured guest TX frames are independently received from real QEMU device.'),indent=2)+'\n')
 proc=subprocess.Popen(cmd,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);connection=None
 def text():return serial.read_text(errors='replace')if serial.exists()else''
 def wait(pattern,timeout=30):
  end=time.monotonic()+timeout
  while time.monotonic()<end:
   if proc.poll()is not None:raise RuntimeError('guest exited')
   t=text()
   if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|to=Quarantined',t):raise RuntimeError('guest/device failure')
   match=re.search(pattern,t)
   if match:return match
   time.sleep(.02)
  raise TimeoutError(pattern)
 def capture(direction,data):captures.append(dict(direction=direction,host_time=time.time(),hex=data.hex(),sha256=hashlib.sha256(data).hexdigest()))
 def exact(n):
  result=b''
  while len(result)<n:
   part=connection.recv(n-len(result))
   if not part:raise RuntimeError('peer closed')
   result+=part
  return result
 def receive():
  n=struct.unpack('!I',exact(4))[0]
  if not 14<=n<=65536:raise RuntimeError('invalid backend frame length')
  data=exact(n);capture('guest_tx',data);return data
 def send(data):connection.sendall(struct.pack('!I',len(data))+data);capture('peer_fixture',data)
 try:
  connection,_=server.accept();connection.settimeout(15)
  wait(r'\[sexnet.ether.ready\]');mac=bytes.fromhex(wait(r'\[sexnet.nic.prepare\] from=Verify to=Quiescent failure=None status=\d+ mac=([0-9a-f]{12})')[1])
  # Preserve unrelated pre-guest firmware frames, but never count them as driver proof.
  for _ in range(64):
   observed=receive()
   if observed==query(mac):break
  else:raise RuntimeError('no actual gateway query')
  incoming=packet(mac,PEER,2,GATEWAY,mac,LOCAL);send(incoming);exchanges.append(dict(guest_tx=observed.hex(),peer_fixture=incoming.hex()))
  wait(r'\[sexnet.ether.rx\] sequence=1 ')
  for i in range(1,25):
   ip=bytes([10,0,2,32+i]);padding=bytes(((i*17+n)&255)for n in range(18));incoming=packet(b'\xff'*6,PEER,1,ip,b'\0'*6,LOCAL,padding)
   send(incoming);observed=receive();exchanges.append(dict(guest_tx=observed.hex(),peer_fixture=incoming.hex()))
   if observed!=reply(mac,PEER,ip):raise RuntimeError('actual guest reply differs at exchange'+str(i))
   wait(r'\[sexnet.ether.tx.done\] sequence='+str(i+1)+r' ')
  wait(r'\[app.surface.presented\] target=15 surface=300')
  result=validate(text(),exchanges,mac);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
  if not result['ok']:raise RuntimeError(str(result['errors']))
 finally:
  (out/'frame-capture.json').write_text(json.dumps(dict(captures=captures,exchanges=exchanges),indent=2)+'\n')
  for direction in ['guest_tx','peer_fixture']:
   with(out/(direction+'.pcap')).open('wb')as f:
    f.write(struct.pack('<IHHIIII',0xa1b2c3d4,2,4,0,0,65536,1))
    for item in captures:
     if item['direction']!=direction:continue
     data=bytes.fromhex(item['hex']);seconds=int(item['host_time']);micros=int((item['host_time']-seconds)*1e6)
     f.write(struct.pack('<IIII',seconds,micros,len(data),len(data))+data)
  if connection:connection.close()
  server.close()
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=10)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
if __name__=='__main__':main()
