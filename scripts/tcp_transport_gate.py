#!/usr/bin/env python3
"""Normal-image real host TCP client → QEMU NIC/DMA → private guest engine."""
import argparse,hashlib,json,re,shutil,socket,struct,subprocess,time
from pathlib import Path
from check_nic_command import inspect as nic
from check_tcp_capture import inspect as capture_check,fingerprint

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--model',required=True,choices=['e1000','e1000e']);a=p.parse_args()
 iso=a.iso.resolve(strict=True);src=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh output required')
 disk=out/'nvme.img';shutil.copyfile(src,disk);serial=out/'serial.log';pcap=out/'wire.pcap';rows=[]
 with socket.socket()as reserve:reserve.bind(('127.0.0.1',0));port=reserve.getsockname()[1]
 cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-cdrom',str(iso),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev',f'user,id=peer,hostfwd=tcp:127.0.0.1:{port}-10.0.2.15:46076','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}']
 (out/'manifest.json').write_text(json.dumps(dict(iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),input_disk_sha256=hashlib.sha256(src.read_bytes()).hexdigest(),command=cmd,qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),scope='Real host kernel TCP client, explicit private loopback forward and selected QEMU NIC/DMA. No host mock TCP producer or change to host routing.'),indent=2)+'\n')
 proc=subprocess.Popen(cmd,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT)
 def text():return serial.read_text(errors='replace')if serial.exists()else''
 def wait(pattern,timeout=45):
  end=time.monotonic()+timeout
  while time.monotonic()<end:
   if proc.poll()is not None:raise RuntimeError('guest exited')
   t=text()
   if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|to=Quarantined|sexnet.transport.failed',t):raise RuntimeError('guest/device/protocol failure')
   match=re.search(pattern,t)
   if match:return match
   time.sleep(.02)
  raise TimeoutError(pattern)
 try:
  wait(r'\[sexnet.transport.ready\] implementation=smoltcp0.14.0 flows=1 rx=4096 tx=4096 turn=256 port=46076 no_std=1 heap=0 diagnostic_listener=1 predictable_seed=1 app_network=0 dns=0 http=0 tls=0')
  for flow,(length,seed,aborted)in enumerate([(16641,7,False),(32771,11,False),(4097,29,False),(1025,71,True),(8193,251,False)],1):
   payload=bytes(((n*17+seed)&255)for n in range(length));started=time.monotonic();client=socket.create_connection(('127.0.0.1',port),timeout=15);client.settimeout(15);client.setsockopt(socket.SOL_SOCKET,socket.SO_RCVBUF,4096)
   wait(r'\[sexnet.transport.state\] from=\w+ to=Established flow='+str(flow)+r' ')
   try:
    client.sendall(payload)
    if aborted:
     response=client.recv(256)
     if not response or not payload.startswith(response):raise RuntimeError('aborted flow did not echo its actual prefix')
     client.setsockopt(socket.SOL_SOCKET,socket.SO_LINGER,struct.pack('ii',1,0))
    else:
     client.shutdown(socket.SHUT_WR);response=b''
     while True:
      part=client.recv(1024)
      if not part:break
      response+=part
      if len(response)>len(payload):raise RuntimeError('extra echoed data')
     if response!=payload:raise RuntimeError('real TCP echo differs')
   finally:client.close()
   if aborted:wait(r'\[sexnet.transport.state\] from=\w+ to=Listen flow='+str(flow)+r' ')
   else:
    value=fingerprint(payload);wait(r'\[sexnet.transport.state\] from=\w+ to=Listen flow='+str(flow)+r' received='+str(length)+r' queued='+str(length)+r' receive_hash='+hex(value)+r' queue_hash='+hex(value))
   rows.append(dict(flow=flow,length=length,aborted=aborted,payload_hex=payload.hex(),payload_sha256=hashlib.sha256(payload).hexdigest(),response_sha256=hashlib.sha256(response).hexdigest(),seconds=time.monotonic()-started))
  wait(r'\[app.surface.presented\] target=15 surface=300')
 finally:
  (out/'endpoint-exchanges.json').write_text(json.dumps(rows,indent=2)+'\n')
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=10)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
 if len(rows)!=5:raise RuntimeError('incomplete endpoint exchanges')
 validation=capture_check(pcap.read_bytes(),rows);authority=nic(text(),1,True,True)
 result=dict(ok=validation['ok']and authority['ok'],capture=validation,authority=authority,completed_stream_bytes=sum(r['length']for r in rows if not r['aborted']),aborted_flow=4,scope='Actual normal-image IPv4/TCP endpoint bytes and close/reuse after abrupt peer exit, including more than4KiB buffering. Host-generated payloads are explicit inputs. Diagnostic service only; app network/DNS/HTTP/TLS and physical/hostile/epoch gates remain unqualified.')
 (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(str(result))
if __name__=='__main__':main()
