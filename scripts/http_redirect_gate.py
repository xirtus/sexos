#!/usr/bin/env python3
"""Frozen-image, real HID/DNS/TCP proof of finite same-endpoint navigation.

Loopback sockets are explicit peer fixtures. Every guest request, endpoint,
redirect framing byte, fresh DNS generation and cleanup barrier is checked.
"""
import argparse,hashlib,json,os,re,shutil,socket,subprocess,threading,time
from pathlib import Path
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from network_contention_gate import frozen_binding,owner_joins,cleanup_receipts,MAX_PCAP
from check_tcp_capture import inspect as tcp_capture,fingerprint
from check_dns_capture import inspect as dns_capture,query as dns_query,response as dns_response
from check_nic_command import inspect as nic_check
from http_content_gate import load_pixel_contract,wire_response,STREAM_BODY,STREAM_TAIL,ECHO,overview_preview,wait_http_pixels,wait_reader_pixels,reader_geometry,reader_move,resolution_joins,stream_receipts

MODES=('chain','external','self-loop','two-node-loop','hop-limit','duplicate-location','missing-location','short-body','trailing-body','cancel-retry','close')
REDIRECT_BODY=b'REDIRECT FRAMING DATA / NEVER A READER DOCUMENT.'
HOST=b'probe.sexos.test'

def request(path):
 if not isinstance(path,bytes)or not 1<=len(path)<=48 or not re.fullmatch(rb'/[A-Za-z0-9/._~-]*',path)or b'//'in path or any(part in(b'.',b'..')for part in path.split(b'/')):raise ValueError('noncanonical bounded same-endpoint request path')
 return b'GET '+path+b' HTTP/1.1\r\nHost: '+HOST+b'\r\nConnection: close\r\nAccept: text/plain\r\n\r\n'

def redirect(status,location,body=REDIRECT_BODY,duplicate=False,missing=False,length_delta=0,trailing=b''):
 if status not in(302,307):raise ValueError('explicit redirect fixture status required')
 if len(body)>1024 or len(body)+length_delta<0 or not isinstance(location,bytes)or b'\r'in location or b'\n'in location:raise ValueError('unbounded or injected redirect fixture')
 fields=b''if missing else b'Location: '+location+b'\r\n'
 if duplicate:fields+=b'Location: '+location+b'\r\n'
 return b'HTTP/1.1 '+str(status).encode()+b' Redirect\r\n'+fields+b'Content-Type: text/plain\r\nContent-Length: '+str(len(body)+length_delta).encode()+b'\r\nConnection: close\r\n\r\n'+body+trailing

def plan(mode):
 if mode not in MODES:raise ValueError('explicit supported redirect mode required')
 def hop(path,status=302,to=b'/guide',**kwargs):return dict(path=path,status=status,next=to,response=redirect(status,to,**kwargs),aborted=mode in('external','duplicate-location','missing-location','short-body','trailing-body'))
 if mode=='chain':return [hop(b'/welcome'),hop(b'/guide',307,b'/guide/text'),dict(path=b'/guide/text',status=200,next=None,response=wire_response('stream',STREAM_BODY),aborted=False)]
 if mode=='two-node-loop':return [hop(b'/welcome'),hop(b'/guide',307,b'/welcome')]
 if mode=='hop-limit':return [hop(b'/welcome'),hop(b'/guide',307,b'/guide/text'),hop(b'/guide/text',302,b'/fourth')]
 if mode=='cancel-retry':return [hop(b'/welcome'),dict(path=b'/welcome',status=200,next=None,response=wire_response('stream',STREAM_BODY),aborted=False)]
 return [hop(b'/welcome',to=b'https://outside.example/elsewhere'if mode=='external'else b'/welcome'if mode=='self-loop'else b'/guide',duplicate=mode=='duplicate-location',missing=mode=='missing-location',length_delta=1 if mode=='short-body'else 0,trailing=b'!'if mode=='trailing-body'else b'')]

def expected_admissions(mode):
 if mode=='cancel-retry':return [(2,1),(3,3)]
 return [(index+2,index+1)for index in range(len(plan(mode)))]

def verify_navigation(trace,mode,admissions):
 """Order audit independent of response construction and the app controller."""
 errors=[];hops=[];fixtures=plan(mode)
 try:
  assert [(row['ticket'],row['dns_generation'])for row in admissions]==expected_admissions(mode),'wrong admitted tickets/generations'
  expected_redirects=[]
  if mode in('chain','self-loop','two-node-loop','hop-limit','cancel-retry','close'):
   expected_redirects=[(index+2,row)for index,row in enumerate(fixtures)if row['status']in(302,307)]
  receipts=list(re.finditer(r'\[browser.http.redirect.released\] ticket=(\d+) status=(\d+) path=([^ ]+) positive_release=1 document_bytes=0',trace))
  assert [(int(m[1]),int(m[2]),m[3].encode())for m in receipts]==[(ticket,row['status'],row['next'])for ticket,row in expected_redirects],'wrong released redirect data or missing positive cleanup'
  final_ticket=4 if mode=='chain'else 3 if mode=='cancel-retry'else None
  final_path=b'/guide/text'if mode=='chain'else b'/welcome'
  bodies=[int(n)for n in re.findall(r'\[browser.http.body\] ticket=(\d+) ',trace)]
  documents=list(re.finditer(r'\[browser.http.document\] ticket=(\d+) path=([^ ]+) status=200 private_bytes=(\d+) positive_release=1',trace))
  assert bodies==([]if final_ticket is None else[final_ticket]),'redirect body or failed response published'
  assert [(int(m[1]),m[2].encode(),int(m[3]))for m in documents]==([]if final_ticket is None else[(final_ticket,final_path,len(STREAM_BODY))]),'wrong final released document/path/length'
  if final_ticket:
   released=re.search(rf'\[browser.http.released\] ticket={final_ticket} view=Ready private_job=0',trace);assert released and released.start()<documents[0].start(),'document before positive Ready RELEASE'
   assert not re.search(r'\[browser.view.applied\] bytes=\d+ http=(?:2|258) ',trace[:released.start()]),'redirect became Ready/presented before final release'
  else:assert not re.search(r'\[browser.view.applied\] bytes=\d+ http=(?:2|258) ',trace),'failed navigation produced Ready pixels'
  for index,admission in enumerate(admissions):
   ticket=admission['ticket'];generation=admission['dns_generation']
   if index:
    prior=admissions[index-1]['ticket'];released=re.search(rf'\[browser.http.released\] ticket={prior} view=Pending private_job=0',trace)
    redirect_receipt=next((m for m in receipts if int(m[1])==prior),None)
    dns=re.search(rf'\[net.dns.egress.submit\] ticket={generation} ',trace)
    assert released and redirect_receipt and dns and released.start()<redirect_receipt.start()<dns.start()<admission['admission_log_offset'],'followed before preceding positive HTTP cleanup/fresh DNS'
    hops.append(dict(from_ticket=prior,to_ticket=ticket,generation=generation,release_offset=released.start(),dns_egress_offset=dns.start()))
  follows=re.findall(r'\[browser.http.redirect.follow\] hop=(\d+) path=([^ ]+) same_endpoint=1 fresh_dns=1 positive_release=1',trace)
  expected=[('1','/guide'),('2','/guide/text')]if mode in('chain','hop-limit')else[('1','/guide')]if mode in('two-node-loop','cancel-retry','close')else[]
  assert follows==expected,'unexpected continuation, stale follow, or follow beyond two hops'
  if mode in('self-loop','two-node-loop','hop-limit'):assert trace.count('[browser.http.redirect.denied] loop_or_limit_or_superseded=1 effects=0')==1,'loop/hop cap not positively rejected'
  if mode=='cancel-retry':
   cancelled=re.search(r'\[browser.dns.released\] ticket=2 view=Cancelled private_job=0',trace);assert cancelled,'held second-hop DNS was not cancelled/released'
   assert cancelled.start()<re.search(r'\[net.dns.egress.submit\] ticket=3 ',trace).start(),'fresh retry preceded cancelled DNS cleanup'
 except (AssertionError,AttributeError,StopIteration,KeyError)as exc:errors.append(str(exc))
 return dict(ok=not errors,errors=errors,hops=hops,scope='Actual complete validated redirect cleanup precedes each next fresh DNS; exact generation-bound SYN joins checked separately. Only final positive200 RELEASE may publish a path-bound document. No endpoint authority derives from redirect data.')

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--mode',choices=MODES,default='chain');p.add_argument('--model',choices=['e1000','e1000e'],default='e1000e');p.add_argument('--resolution',choices=['1280x800','1024x768'],default='1280x800');a=p.parse_args()
 root=Path(__file__).resolve().parents[1];iso=a.iso.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 binding=frozen_binding(root,iso,a.binding.resolve(strict=True));contract=load_pixel_contract(root);fixtures=plan(a.mode);resolution=tuple(map(int,a.resolution.split('x')))
 if json.loads(a.binding.read_text())['runner_sha256'].get('scripts/http_redirect_gate.py')!=hashlib.sha256(Path(__file__).read_bytes()).hexdigest():raise RuntimeError('redirect runner absent from exact frozen cohort')
 if request(b'/welcome')!=contract['request']:raise RuntimeError('independent initial request differs from frozen production profile')
 if 'expected_path'not in __import__('inspect').signature(wait_reader_pixels).parameters:raise RuntimeError('frozen pixel helper lacks positive path-bound reader contract')
 rows=[];dns_rows=[];attempts=[];server_errors=[];connections=[];stop=threading.Event();held_dns=threading.Event();pixels=[];readers=[];actions=[];failure=None;cleanup=None
 count_dns=3 if a.mode=='cancel-retry'else 2 if a.mode=='close'else len(fixtures)
 tcp=socket.socket();tcp.bind(('127.0.0.1',46078));tcp.listen(4);tcp.settimeout(.1)
 udp=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);udp.bind(('127.0.0.1',46079));udp.settimeout(.1)
 def serve_dns():
  try:
   while not stop.is_set():
    try:data,remote=udp.recvfrom(513)
    except socket.timeout:continue
    ticket=len(dns_rows)+1
    if ticket>count_dns or data!=dns_query(ticket):raise RuntimeError('unexpected/noncanonical/replayed DNS request')
    row=dict(ticket=ticket,request_hex=data.hex(),response_hex=None,remote=list(remote));dns_rows.append(row)
    if ticket==2 and a.mode in('cancel-retry','close'):held_dns.set();continue
    reply=dns_response(ticket);row['response_hex']=reply.hex();sent=udp.sendto(reply,remote);attempts.append(dict(ticket=ticket,kind='answer',response_hex=reply.hex(),sent_bytes=sent,remote=list(remote)))
  except Exception as exc:
   if not stop.is_set():server_errors.append(repr(exc))
 def serve_tcp():
  try:
   for index,item in enumerate([dict(path=None,response=ECHO,aborted=False)]+fixtures,1):
    while not stop.is_set():
     try:conn,remote=tcp.accept();break
     except socket.timeout:continue
    else:return
    connections.append(conn);conn.settimeout(25);received=bytearray()
    while True:
     part=conn.recv(512)
     if not part:break
     received.extend(part)
     if len(received)>4096:raise RuntimeError('guest request exceeded bounded fixed payload')
    wanted=ECHO if index==1 else request(item['path'])
    if bytes(received)!=wanted:raise RuntimeError('actual request path/Host/framing differs at ticket'+str(index))
    row=dict(ticket=index,path=item['path'].decode()if item['path']else None,payload_hex=bytes(received).hex(),response_hex=item['response'].hex(),aborted=item['aborted'],allow_empty_reply=item['aborted'],remote=list(remote),sent_hex='',send_error=None);rows.append(row);sent=bytearray()
    try:
     for off in range(0,len(item['response']),13):conn.sendall(item['response'][off:off+13]);sent.extend(item['response'][off:off+13]);time.sleep(.001)
     conn.shutdown(socket.SHUT_WR)
    except OSError as exc:
     row['send_error']=repr(exc)
     if not item['aborted']:raise
    finally:row['sent_hex']=sent.hex();conn.close()
   while not stop.wait(.1):
    try:extra,_=tcp.accept();extra.close();raise RuntimeError('unexpected unauthorized/automatic TCP continuation')
    except socket.timeout:pass
  except Exception as exc:
   if not stop.is_set():server_errors.append(repr(exc))
 threads=[threading.Thread(target=serve_tcp,daemon=True),threading.Thread(target=serve_dns,daemon=True)]
 for thread in threads:thread.start()
 disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-redirect-{os.getpid()}.sock'
 command=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-vga','none','-device','VGA,xres='+a.resolution.replace('x',',yres='),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}']
 (out/'manifest.json').write_text(json.dumps(dict(binding=binding,iso_sha256=binding['iso_sha256'],mode=a.mode,model=a.model,resolution=a.resolution,command=command,input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),fixtures=[dict(path=i['path'].decode(),status=i['status'],next=i['next'].decode()if i['next']else None,response_hex=i['response'].hex(),aborted=i['aborted'])for i in fixtures],qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),scope='Fixed same-endpoint origin-form path data under existing BOOT route; exact requests and real captured loopback TCP/UDP bytes. No public HTTP/DNS authority or TLS claim.'),indent=2)+'\n')
 proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic()
 def text():return serial.read_text(errors='replace')if serial.exists()else''
 def health():
  if time.monotonic()-began>300:raise TimeoutError('bounded300-second whole gate')
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if server_errors:raise RuntimeError(str(server_errors))
  if any(p.exists()and p.stat().st_size>MAX_PCAP for p in(serial,pcap)):raise RuntimeError('bounded raw evidence exceeded')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|sexnet.fatal|serial.record.overflow',text()):raise RuntimeError('guest failure')
 def wait(pattern,start=0,timeout=90):
  deadline=time.monotonic()+timeout
  while time.monotonic()<deadline:
   health();match=re.search(pattern,text()[start:])
   if match:return match
   time.sleep(.02)
  raise TimeoutError(pattern)
 def key(code,pattern):
  start=len(text());q.key(code);return wait(pattern,start)
 def focus():focus_surface(300,text,key)
 def http_pixels(view,name):
  result=wait_http_pixels(q,text,out,view,overview_preview(STREAM_BODY),contract,health,name,resolution=resolution);pixels.append(result)
  if not result['ok']:raise RuntimeError(str(result['errors']))
 def zoom():
  key('grave_accent',r'\[shell.palette.item\] idx=0')
  for old,new in[(0,9),(9,8)]:key('k',rf'\[shell.palette.select\] old={old} new={new}')
  key('ret',r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1');wait(r'\[browser.viewport\] target=15 task=15 sequence=\d+ w=\d+ h=\d+ origin=held_shell',zoom_start[0])
 try:
  end=time.monotonic()+15
  while not sock.exists():
   health()
   if time.monotonic()>end:raise TimeoutError('QMP')
   time.sleep(.05)
  q=QMP(sock)
  if a.resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret');actions.append(dict(action='actual_compact_boot_menu'))
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[linen.disk.publish.done\]');wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0');focus();start=len(text());q.key('h')
  if a.mode in('cancel-retry','close'):
   wait(r'\[browser.http.redirect.released\] ticket=2 status=302 path=/guide positive_release=1 document_bytes=0',start)
   deadline=time.monotonic()+30
   while not held_dns.is_set():
    health()
    if time.monotonic()>deadline:raise TimeoutError('actual second-hop DNS query')
    time.sleep(.01)
   if len(rows)!=2:raise RuntimeError('second HTTP started before held DNS cleanup')
   start=len(text());q.key('j'if a.mode=='cancel-retry'else'f11')
   if a.mode=='cancel-retry':
    wait(r'\[browser.dns.released\] ticket=2 view=Cancelled private_job=0',start);wait(r'\[browser.view.applied\] bytes=\d+ http=4 body_bytes=0 ',start);http_pixels('Cancelled','redirect-cancelled');start=len(text());q.key('h');actions.append(dict(action='actual_J_then_fresh_H_initial_path',cancelled_generation=2))
   else:
    wait(r'\[browser.dns.closed\] pending=1 terminal=1',start);wait(r'\[sext.app.pool.ready\] target=15 task=15 .*ok=1',start);cleanup=cleanup_receipts(text(),15,15,start)
    if re.search(r'\[net\.(?:job|dns)\.egress\.submit\]',text()[cleanup['ack_offset']:]):raise RuntimeError('owned egress submission after exact terminal cleanup ACK')
    q.cmd('screendump',{'filename':str(out/'redirect-closed.ppm')});actions.append(dict(action='actual_F11_waiting_second_hop_DNS',cleanup=cleanup,pixel_scope='Terminally closed surface has no live status/reader; five exact-task service roles, zero owner records and Sext pool receipts qualify cleanup.'))
  if a.mode in('chain','cancel-retry'):
   ticket=4 if a.mode=='chain'else 3;path=b'/guide/text'if a.mode=='chain'else b'/welcome'
   wait(rf'\[browser.http.document\] ticket={ticket} path={re.escape(path.decode())} status=200 private_bytes=12288 positive_release=1',start);wait(r'\[browser.view.applied\] bytes=\d+ http=2 body_bytes=12288 ',start);http_pixels('Ready','redirect-ready')
   # Expand through actual shell input so the complete final hostname/path is
   # visible even in compact tiling; restore before overview/cancel checks.
   zoom_start=[len(text())];zoom();key('v',r'\[shell.browser.input\] code=47 down=1 slot=43 accepted=1')
   first=wait_reader_pixels(q,text,out,'redirect-reader-initial',STREAM_BODY,contract,0,health,resolution=resolution,expected_path=path);readers.append(first)
   if not first['ok']or (HOST+b' '+path).decode()not in first['expected_rows'][1]:raise RuntimeError('complete final released path not visible in reader header')
   initial=reader_geometry(text(),STREAM_BODY,contract,0,expected_path=path);anchor=reader_move(initial,'end');key('end',r'\[shell.browser.input\] code=107 down=1 slot=43 accepted=1')
   final=wait_reader_pixels(q,text,out,'redirect-reader-end',STREAM_BODY,contract,anchor,health,resolution=resolution,expected_path=path);readers.append(final)
   if not final['ok']or final['geometry']['end']!=final['geometry']['total_lines']or STREAM_TAIL.decode()not in ''.join(final['expected_rows'][2:-1]):raise RuntimeError('full final document tail not visible/reachable')
   zoom_start[0]=len(text());zoom();key('v',r'\[shell.browser.input\] code=47 down=1 slot=43 accepted=1');start=len(text());q.key('j');wait(r'\[browser.view.applied\] bytes=\d+ http=4 body_bytes=0 ',start);http_pixels('Cancelled','redirect-erased')
   actions.append(dict(action='final_path_full12KiB_reader_End_and_actual_J_erasure',ticket=ticket,path=path.decode(),length=len(STREAM_BODY)))
  elif a.mode!='close':
   wait(r'\[browser.view.applied\] bytes=\d+ http=3 body_bytes=0 ',start);http_pixels('Unavailable','redirect-denied')
 except Exception as exc:failure=repr(exc)
 finally:
  stop.set()
  for conn in connections:
   try:conn.close()
   except OSError:pass
  tcp.close();udp.close()
  for thread in threads:thread.join(timeout=1)
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
 raw=pcap.read_bytes()if pcap.exists()else b'';trace=text();(out/'endpoint-exchanges.json').write_text(json.dumps(dict(tcp=rows,dns=dns_rows,dns_attempts=attempts),indent=2)+'\n')
 transport=tcp_capture(raw,rows,guest_ports=range(48001,65536),initiator='guest')if raw else dict(ok=False,errors=['missing actual PCAP'])
 dns=dns_capture(raw,'echo',attempts,expected=[dict(ticket=row['ticket'],response_hex=row['response_hex'])for row in dns_rows])if dns_rows else dict(ok=False,errors=['missing actual DNS exchanges'])
 joins=resolution_joins(trace,raw,expected_admissions(a.mode),dns);owners=owner_joins(trace,raw,[(15,15)]*(len(fixtures)+1));navigation=verify_navigation(trace,a.mode,joins['joins']);final_ticket=4 if a.mode=='chain'else 3 if a.mode=='cancel-retry'else None
 streaming=stream_receipts(trace,[x[0]for x in expected_admissions(a.mode)],contract,[final_ticket]if final_ticket else[],[final_ticket]if final_ticket else[]);authority=nic_check(trace,expected=1,preparation=True,ethernet=True,network_config=True,routing=True)
 if len(rows)!=len(fixtures)+1 or len(dns_rows)!=count_dns:failure=failure or'incomplete/excess actual HTTP or DNS plan'
 if transport['ok']and a.mode in('short-body','trailing-body'):
  actual=transport['connections'][1];reply=fixtures[0]['response'];header=reply.index(b'\r\n\r\n')+4
  if actual['incoming_bytes']<len(reply)or(a.mode=='short-body'and not actual['fin_in'])or(a.mode=='trailing-body'and actual['incoming_bytes']<=header+len(REDIRECT_BODY)):failure=failure or'actual capture did not carry complete malformed redirect body/EOF evidence'
 if transport['ok']and a.mode in('external','duplicate-location','missing-location')and transport['connections'][1]['incoming_bytes']<fixtures[0]['response'].index(b'\r\n\r\n')+4:failure=failure or'invalid Location header not fully present in actual capture'
 result=dict(ok=failure is None and not server_errors and all(v['ok']for v in(transport,dns,joins,owners,navigation,streaming,authority)),error=failure,server_errors=server_errors,mode=a.mode,binding=binding,transport=transport,dns=dns,resolution_order=joins,owner_packet_joins=owners,navigation=navigation,streaming=streaming,authority=authority,pixels=pixels,reader_pixels=readers,actions=actions,cleanup=cleanup,scope='Actual same-endpoint redirect navigation through fixed-name fresh DNS, positive preceding HTTP RELEASE, generation-bound TCP admission and exact captured origin-form requests. Complete redirect bodies never publish documents. Only positive200 RELEASE yields path-bound full12KiB reader/End; real cancel erases it. No arbitrary URL authority, TLS/public web, DNS authenticity, cache, hard performance, hostile/SMP or physical isolation proof.')
 (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(result))
 print(json.dumps(dict(ok=True,mode=a.mode,http_jobs=len(fixtures),dns_generations=len(dns_rows))))

if __name__=='__main__':main()
