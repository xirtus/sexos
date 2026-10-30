#!/usr/bin/env python3
"""Actual fixed-peer Browser/Spindle contention and terminal network cleanup.

Never builds. Requires one immutable source/image/runner binding. Host TCP/UDP
sockets are explicit loopback peer fixtures, never guest descriptors/authority.
"""
import argparse,hashlib,json,os,re,shutil,socket,struct,subprocess,threading,time
from pathlib import Path
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from check_tcp_capture import inspect as tcp_capture,frames,checksum,fingerprint
from check_dns_capture import inspect as dns_capture,query as dns_query,response as dns_response
from check_nic_command import inspect as nic_check
from check_spindle_viewport_pixels import verify as spindle_pixels
from http_content_gate import load_pixel_contract,wait_http_pixels,wire_response,BODY,STREAM_BODY,ECHO

LOCAL=bytes([10,0,2,15]);REMOTE=bytes([10,0,2,2]);MAX_PCAP=16*1024*1024

def frozen_binding(root,iso,path):
 binding=json.loads(path.read_text())
 if binding.get('iso_sha256')!=hashlib.sha256(iso.read_bytes()).hexdigest():raise ValueError('frozen ISO binding differs')
 provenance=binding.get('build_provenance',{})
 if provenance.get('build_gate')!='PASS'or provenance.get('source_image_binding')!='sealed_build_this_run'or provenance.get('sources_unchanged_during_build')is not True or provenance.get('iso_sha256')!=binding['iso_sha256']:raise ValueError('missing successful unchanged sealed source/image build')
 for group in ('sources','runner_sha256'):
  values=binding.get(group,{})
  if not values:raise ValueError('missing frozen '+group)
  for name,wanted in values.items():
   file=(root/name).resolve()
   if not file.is_relative_to(root.resolve())or not file.is_file()or hashlib.sha256(file.read_bytes()).hexdigest()!=wanted:raise ValueError('frozen source/runner mismatch: '+name)
 if 'scripts/network_contention_gate.py'not in binding['runner_sha256']:raise ValueError('contention runner absent from frozen cohort')
 return dict(path=str(path),sha256=hashlib.sha256(path.read_bytes()).hexdigest(),iso_sha256=binding['iso_sha256'],production_files=len(binding['sources']),runner_files=len(binding['runner_sha256']))

def syn_records(raw):
 """Independently bind each first actual guest SYN to immutable endpoint bytes."""
 found={}
 for index,frame in enumerate(frames(raw),1):
  if len(frame)<34 or frame[12:14]!=b'\x08\x00':continue
  ip=frame[14:];ihl=(ip[0]&15)*4;length=int.from_bytes(ip[2:4],'big')
  if ip[9]!=6 or ip[12:16]!=LOCAL:continue
  if ip[0]>>4!=4 or ihl<20 or length<ihl+20 or len(ip)<length or checksum(ip[:ihl]):raise ValueError('bad actual outgoing IPv4')
  segment=ip[ihl:length];sport,dport=struct.unpack_from('!HH',segment)
  if ip[16:20]!=REMOTE or dport!=46078 or not 48001<=sport<=65535:raise ValueError('guest TCP escaped fixed endpoint/source selector')
  if checksum(ip[12:20]+b'\0\6'+len(segment).to_bytes(2,'big')+segment):raise ValueError('bad actual outgoing TCP checksum')
  if segment[13]&2 and not segment[13]&16:
   item=dict(ticket=sport-48000,packet=index,length=len(frame),hash=hex(fingerprint(frame)),sequence=int.from_bytes(segment[4:8],'big'))
   if sport in found and found[sport]['sequence']!=item['sequence']:raise ValueError('source tuple reused')
   found.setdefault(sport,item)
 return list(found.values())

def owner_joins(trace,raw,owners):
 errors=[];joined=[]
 try:
  syns=syn_records(raw)
  if [s['ticket']for s in syns]!=list(range(1,len(owners)+1)):raise ValueError('extra/missing SYN, denied caller allocated a flow or tickets skipped')
  for syn,(owner,task)in zip(syns,owners):
   ticket=syn['ticket']
   state=re.search(rf'\[net.job.state\] owner=Some\(Identity \{{ pd: {owner}, task: {task} \}}\) ticket={ticket} ',trace)
   submit=re.search(rf'\[net.job.egress.submit\] ticket={ticket} sequence=(\d+) length=(\d+) hash=(0x[0-9a-f]+) scope=dma_admission_not_delivery',trace)
   if not state or not submit:raise ValueError('missing exact authenticated owner/task or first SYN admission')
   if (int(submit[2]),submit[3])!=(syn['length'],syn['hash']):raise ValueError('first SYN does not join actual DMA admission')
   if owner in (12,17):
    intents=re.findall(rf'\[spindle.net.intent\] target={owner} task={task} command=net-(?:probe|retry) accepted=1 seq=(\d+) fixed_peer=1',trace[:state.start()])
    if not intents or not re.search(rf'\[spindle.net.reply\] target={owner} task={task} tag=120 seq={int(intents[-1])} value={ticket} observed=1',trace):raise ValueError('Spindle BEGIN lacks exact intent/sequence/ticket receipt')
   joined.append(dict(owner=owner,task=task,**syn,submit_sequence=int(submit[1])))
 except (ValueError,IndexError,struct.error)as exc:errors.append(str(exc))
 return dict(ok=not errors,errors=errors,joins=joined)

def visible_spindle_text(trace,target):
 sids={12:[154,160,161,162],17:[302,303,304,305]}[target];panes=[]
 for sid in sids:
  records=re.findall(rf'\[spindle.view.text\] target={target} sid={sid} sequence=(\d+) paint=(\d+) cols=(\d+) rows=(\d+) bytes=(\d+)',trace)
  if not records:raise ValueError('missing actual Spindle pane')
  seq,paint,cols,rows,length=map(int,records[-1]);data=bytearray()
  for offset,part in re.findall(rf'\[spindle.view.bytes\] target={target} sid={sid} seq={seq} paint={paint} off=(\d+) hex=([0-9a-f]*)',trace):
   if int(offset)!=len(data):raise ValueError('noncontiguous current Spindle pane')
   data.extend(bytes.fromhex(part))
  if len(data)!=length:raise ValueError('incomplete actual Spindle pane')
  lines=bytes(data).split(b'\n')
  if rows and (len(lines)!=rows+1 or any(len(line)!=cols for line in lines[:-1])):raise ValueError('malformed actual Spindle rows')
  panes.append(lines[:-1]if rows else[])
 return b'\n'.join(left+right for a,b in ((0,1),(2,3))for left,right in zip(panes[a],panes[b]))

def cleanup_receipts(trace,target,task,start):
 suffix=trace[start:];roles={int(n)for n in re.findall(rf'\[app.service.ack\] target={target} task={task} holder=\d+ role=(\d+) ',suffix)}
 retired=re.search(rf'\[net.dns.records.retired\] target={target} task={task} remaining=0 before_cleanup_ack=1',suffix)
 ack=re.search(rf'\[net.app.cleanup.ack\] target={target} task={task} client_leases=0 shared_cache=retained ok=1',suffix)
 pool=re.search(rf'\[sext.app.pool.ready\] target={target} task={task} .*ok=1',suffix)
 if roles!={1,2,4,8,16}or not retired or not ack or not pool or not retired.start()<ack.start()<pool.start():raise ValueError('missing/out-of-order exact task five-role network retirement/ACK/pool cleanup')
 return dict(target=target,task=task,roles=sorted(roles),retire_offset=start+retired.start(),ack_offset=start+ack.start(),pool_offset=start+pool.start())

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--mode',choices=['contention','close'],default='contention');p.add_argument('--model',choices=['e1000','e1000e'],default='e1000e');p.add_argument('--resolution',choices=['1280x800','1024x768'],default='1280x800');a=p.parse_args()
 root=Path(__file__).resolve().parents[1];iso=a.iso.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 binding=frozen_binding(root,iso,a.binding.resolve(strict=True));contract=load_pixel_contract(root);request=contract['request'];response=wire_response('echo',BODY);stream=wire_response('stream',STREAM_BODY)
 plan=[('echo',False),('http',True),('echo',False),('echo',True),('http',False)]if a.mode=='contention'else[('echo',False),('echo',True),('echo',False)]
 rows=[];dns_rows=[];dns_attempts=[];server_errors=[];connections=[];stopping=threading.Event();stalled=threading.Event();stages=[];pixels=[];failure=None;cleanup=None;ack_wall=None
 listener=socket.socket();listener.bind(('127.0.0.1',46078));listener.listen(4);listener.settimeout(.1)
 udp=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);udp.bind(('127.0.0.1',46079));udp.settimeout(.1)
 def dns_peer():
  try:
   while not stopping.is_set():
    try:data,remote=udp.recvfrom(513)
    except socket.timeout:continue
    if len(dns_rows)>=2 or len(data)<2:raise RuntimeError('unexpected DNS datagram count/length')
    ticket=int.from_bytes(data[:2],'big')
    if data!=dns_query(ticket)or ticket!=len(dns_rows)+1:raise RuntimeError('wrong fixed DNS question/ID or automatic replay')
    reply=dns_response(ticket);row=dict(ticket=ticket,request_hex=data.hex(),response_hex=reply.hex());dns_rows.append(row);sent=udp.sendto(reply,remote);dns_attempts.append(dict(ticket=ticket,kind='answer',response_hex=reply.hex(),sent_bytes=sent,remote=list(remote)))
  except Exception as exc:
   if not stopping.is_set():server_errors.append(repr(exc))
 def tcp_peer():
  try:
   for index,(kind,stall)in enumerate(plan,1):
    while not stopping.is_set():
     try:conn,remote=listener.accept();break
     except socket.timeout:continue
    else:return
    connections.append(conn);conn.settimeout(20);data=bytearray()
    while True:
     part=conn.recv(512)
     if not part:break
     data.extend(part)
     if len(data)>4096:raise RuntimeError('guest request exceeded fixed bound')
    wanted=ECHO if kind=='echo'else request
    if bytes(data)!=wanted:raise RuntimeError('unexpected actual fixed payload/order at TCP '+str(index))
    reply=ECHO if kind=='echo'else stream if stall else response
    prefix=reply[:reply.index(b'\r\n\r\n')+4+32]if kind=='http'and stall else reply[:32]if stall else reply
    row=dict(ticket=index,kind=kind,payload_hex=bytes(data).hex(),response_hex=prefix.hex(),aborted=stall,allow_empty_reply=stall,peer_remote=list(remote));rows.append(row)
    for off in range(0,len(prefix),13):conn.sendall(prefix[off:off+13])
    if stall:
     # Deliberately withhold remaining bytes/FIN. No background drip or retry.
     stalled.set();continue
    conn.shutdown(socket.SHUT_WR);conn.close()
   while not stopping.wait(.1):
    try:extra,_=listener.accept();extra.close();raise RuntimeError('unexpected automatic extra TCP connection')
    except socket.timeout:pass
  except Exception as exc:
   if not stopping.is_set():server_errors.append(repr(exc))
 threads=[threading.Thread(target=dns_peer,daemon=True),threading.Thread(target=tcp_peer,daemon=True)]
 for thread in threads:thread.start()
 disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-contention-{os.getpid()}.sock'
 command=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-vga','none','-device','VGA,xres='+a.resolution.replace('x',',yres='),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}']
 (out/'manifest.json').write_text(json.dumps(dict(binding=binding,command=command,mode=a.mode,model=a.model,resolution=a.resolution,input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),plan=plan,echo_hex=ECHO.hex(),http_request_hex=request.hex(),qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True)),indent=2)+'\n')
 proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic()
 def text():return serial.read_text(errors='replace')if serial.exists()else''
 def health():
  if time.monotonic()-began>300:raise TimeoutError('bounded 300-second whole native gate')
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if server_errors:raise RuntimeError(str(server_errors))
  if(serial.exists()and serial.stat().st_size>MAX_PCAP)or(pcap.exists()and pcap.stat().st_size>MAX_PCAP):raise RuntimeError('bounded serial/PCAP evidence exceeded')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|sexnet.fatal|serial.record.overflow',text()):raise RuntimeError('guest failure')
 def wait(pattern,start=0,timeout=60):
  end=time.monotonic()+timeout
  while time.monotonic()<end:
   health();match=re.search(pattern,text()[start:])
   if match:return match
   time.sleep(.02)
  raise TimeoutError(pattern)
 def key(code,pattern):
  start=len(text());q.key(code);return wait(pattern,start)
 def focus(sid):focus_surface(sid,text,key)
 def typing(value,target=12):
  for off in range(0,len(value),6):
   part=value[off:off+6];start=len(text());q.key_sequence([{' ':'spc','-':'minus'}.get(ch,ch)for ch in part]);length=off+len(part)
   wait(rf'\[spindle.cursor.view\] target={target} sequence=\d+ pos={length} len={length}(?:\s|$)',start)
 def command(value,target=12):
  typing(value,target);start=len(text());q.key('ret');wait(rf'\[spindle.instance.command\] target={target} task={target} name={re.escape(value.split()[0])} ok=1',start);return start
 def picture(name,target=12,needle=None,prompt=None):
  end=time.monotonic()+20;errors=['no screenshot'];path=out/(name+'.ppm')
  while time.monotonic()<end:
   health();q.cmd('screendump',{'filename':str(path)});trace=text();raw=path.read_bytes();errors=spindle_pixels(trace,raw,contract['font'],target,prompt,False,False,resolution=tuple(map(int,a.resolution.split('x'))))
   try:
    if needle and needle not in visible_spindle_text(trace,target):errors.append('required actual status text absent')
   except ValueError as exc:errors.append(str(exc))
   if not errors:break
   time.sleep(.15)
  result=dict(ok=not errors,name=name,target=target,errors=errors,screenshot=path.name,sha256=hashlib.sha256(path.read_bytes()).hexdigest());pixels.append(result)
  (out/(name+'-pixels.json')).write_text(json.dumps(result,indent=2)+'\n')
  if errors:raise RuntimeError(name+': '+str(errors))
 try:
  end=time.monotonic()+15
  while not sock.exists():
   health()
   if time.monotonic()>end:raise TimeoutError('QMP socket')
   time.sleep(.05)
  q=QMP(sock)
  if a.resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[linen.disk.publish.done\]');wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0')
  for target in (12,17):wait(rf'\[boot.spindle.net.route\] target={target} task={target} slot=18 service=13 fixed_peer=1 nic_metadata=0 runtime_mint=0')
  if a.mode=='contention':
   focus(300);start=len(text());q.key('h');wait(r'\[net.job.stream.drain\] owner=15 task=15 ticket=2 .*state_before=Receiving',start)
   focus(153);start=command('net-probe');wait(r'\[spindle.net.reply\] target=12 task=12 tag=232 seq=1 value=4 observed=1',start);wait(r'\[spindle.net.state\] target=12 task=12 state=unavailable busy=0 ',start)
   if len(rows)!=2:raise RuntimeError('BUSY Spindle emitted extra TCP flow')
   typing('echo responsive');picture('busy-responsive',prompt='> echo responsive');q.key('ret');wait(r'\[spindle.instance.command\] target=12 task=12 name=echo ok=1',start)
   focus(300);start=len(text());q.key('j');wait(r'\[browser.http.released\] ticket=2 view=Cancelled private_job=0',start)
   focus(153);start=command('net-retry');wait(r'\[spindle.net.state\] target=12 task=12 state=verified busy=0 ',start);picture('spindle-verified',needle=b'NET / verified')
   stalled.clear();start=command('net-probe');wait(r'\[net.job.state\] owner=Some\(Identity \{ pd: 12, task: 12 \}\) ticket=4 .*to=Receiving',start)
   focus(300);start=len(text());q.key('h');wait(r'\[browser.view.applied\] bytes=\d+ http=3 body_bytes=0 ',start)
   if len(rows)!=4 or len(dns_rows)!=1:raise RuntimeError('held Spindle allowed competing Browser wire work')
   focus(153);start=command('net-cancel');wait(r'\[spindle.net.state\] target=12 task=12 state=cancelled busy=0 ',start)
   focus(300);start=len(text());q.key('h');wait(r'\[browser.http.released\] ticket=5 view=Ready private_job=0',start)
   result=wait_http_pixels(q,text,out,'Ready',BODY,contract,health,'browser-final',resolution=tuple(map(int,a.resolution.split('x'))));pixels.append(result)
   if not result['ok']:raise RuntimeError(str(result['errors']))
   stages.append(dict(bidirectional_busy=True,explicit_retry=True,responsive_input_pixels=True,positive_release=True))
  else:
   focus(153);start=command('net-probe');wait(r'\[net.job.state\] owner=Some\(Identity \{ pd: 12, task: 12 \}\) ticket=2 .*to=Receiving',start)
   typing('xyz');picture('pending-close-input',prompt='> xyz');start=len(text());q.key('f11');wait(r'\[spindle.net.closed\] target=12 task=12 pending=1 terminal=1 positive_release=0',start);wait(r'\[sext.app.pool.ready\] target=12 task=12 .*ok=1',start);ack_wall=time.time();cleanup=cleanup_receipts(text(),12,12,start)
   key('grave_accent',r'\[shell.palette.item\] idx=0');key('ret',r'\[shell.app.relaunch\] predecessor=12 successor=17 task=17 surface=302 receipt_replayed=1');wait(r'\[app.surface.presented\] target=17 surface=302 sequence=1 ok=1');wait(r'\[shell.focus.set\] id=306')
   start=command('net-probe',17);wait(r'\[spindle.net.state\] target=17 task=17 state=verified busy=0 ',start);picture('successor-verified',17,b'NET / verified')
   if re.search(r'\[net.job.egress.submit\] ticket=2 ',text()[cleanup['ack_offset']:]):raise RuntimeError('old owner egress submission after cleanup ACK')
   stages.append(dict(actual_f11_pending=True,reserved17_actual_launcher=True,old_owner_submission_after_ack=False,already_submitted_dma_scope='Driver custody is distinct; PCAP may include its completion.'))
 except Exception as exc:failure=repr(exc)
 finally:
  stopping.set()
  for conn in connections:
   try:conn.close()
   except OSError:pass
  listener.close();udp.close()
  for thread in threads:thread.join(timeout=1)
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
 raw=pcap.read_bytes()if pcap.exists()else b'';trace=text()
 (out/'endpoint-exchanges.json').write_text(json.dumps(dict(tcp=rows,dns=dns_rows,dns_attempts=dns_attempts),indent=2)+'\n')
 transport=tcp_capture(raw,rows,guest_ports=range(48001,65536),initiator='guest')if raw else dict(ok=False,errors=['missing PCAP'])
 owners=[(15,15),(15,15),(12,12),(12,12),(15,15)]if a.mode=='contention'else[(15,15),(12,12),(17,17)]
 joins=owner_joins(trace,raw,owners);authority=nic_check(trace,expected=1,preparation=True,ethernet=True,network_config=True,routing=True)
 dns=dns_capture(raw,'echo',dns_attempts,expected=[dict(ticket=row['ticket'],response_hex=row['response_hex'])for row in dns_rows])if a.mode=='contention'else dict(ok=not dns_rows,errors=[]if not dns_rows else['unexpected DNS in echo-only Close'])
 if len(rows)!=len(plan):failure=failure or'incomplete actual peer plan'
 if a.mode=='contention'and len(dns_rows)!=2:failure=failure or'missing exact two admitted H DNS exchanges'
 result=dict(ok=failure is None and not server_errors and all(v['ok']for v in(transport,joins,authority,dns)),error=failure,server_errors=server_errors,mode=a.mode,binding=binding,transport=transport,owner_packet_joins=joins,authority=authority,dns=dns,pixels=pixels,stages=stages,cleanup=cleanup,cleanup_observed_wall=ack_wall,scope='Actual HID commands, fixed controlled peer Ethernet PCAP, exact owner/task/ticket joins, bidirectional finite one-wire admission and visible glyphs or exact pending network Close/BOOT successor. Historical shared browser.net.job labels are used only for initial exclusive startup, never to attribute Spindle effects to Browser. Receipt Full/six-owner exhaustive fairness, hard latency, physical/SMP isolation, arbitrary endpoints, public web and DNS authenticity are not qualified.')
 (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(result))
 print(json.dumps(dict(ok=True,mode=a.mode,connections=len(rows))))

if __name__=='__main__':main()
