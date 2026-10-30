#!/usr/bin/env python3
"""Real app HTTP intent, controlled loopback bytes, TCP capture and body pixels."""
import argparse,ast,hashlib,json,os,re,shutil,socket,struct,subprocess,threading,time
from pathlib import Path
from desktop_close_gate import QMP
from check_tcp_capture import inspect as capture,fingerprint,frames
from check_dns_capture import inspect as dns_capture,query as dns_query,response as dns_response
from check_nic_command import inspect as nic
from check_desktop_pixels import font_from_source,glyph,verify_kaleido_layout
from check_disk_persistence import ppm_rgb

DNS_MODES=('dns-disallowed','dns-zero-ttl','dns-malformed','dns-silence','dns-cancel','dns-close','dns-cname-disallowed','dns-cname-zero-ttl','dns-cname-cycle','dns-cname-conflict','dns-bad-pointer','dns-cname-unrelated')
DNS_SUCCESS=('dns-compressed','dns-cname','dns-cname-reordered','dns-cname-sections')
MODES=('echo','malformed','short-body','oversized','redirect','reset','silence','cancel-retry','close','switch')+DNS_MODES+DNS_SUCCESS
BODY=b'LIVE SEXOS HTTP BODY 80 / FROM CONTROLLED PEER.'
ECHO=bytes((n*17+78)&255 for n in range(1025))

def load_pixel_contract(root):
 paths={'renderer':'servers/sexdisplay/src/main.rs','preview':'apps/kaleidoscope/src/body_text.rs','compose':'apps/kaleidoscope/src/browser_view.rs','app':'apps/kaleidoscope/src/main.rs','style':'crates/sex-pdx/src/desktop_style.rs','surface':'servers/sexdisplay/src/surface_lifecycle.rs','wire':'apps/kaleidoscope/src/http_wire.rs','client':'apps/kaleidoscope/src/http_probe.rs'}
 sources={key:(root/path).read_text()for key,path in paths.items()}
 def number(key,pattern):
  match=re.search(pattern,sources[key])
  if not match:raise ValueError('production HTTP pixel contract changed: '+pattern)
  return int(match[1],0)
 def literal(key,pattern):
  match=re.search(pattern,sources[key])
  if not match:raise ValueError('missing production HTTP literal: '+pattern)
  return ast.literal_eval(match[1])
 compose=sources['compose']
 echo={view:ast.literal_eval(value)for view,value in re.findall(r'network_probe::View::(\w+)=>\s*(b"[^"\n]*")',compose)}
 dns={view:ast.literal_eval(value)for view,value in re.findall(r'dns_probe::View::(\w+)=>append\([^\n]*,(b"[^"\n]*")\)',compose)}
 http={view:ast.literal_eval(value)for view,value in re.findall(r'http_probe::View::(\w+)=>\s*(b"[^"\n]*")',compose)}
 if 'self.ui.queue(&text.bytes[..text.len]'not in sources['app']or 'browser_view::compose('not in sources['app']:raise ValueError('Browser no longer publishes composed atomic snapshots')
 if 'FRAME_TOP_BAR_HEIGHT_PX + 8'not in sources['renderer']:raise ValueError('renderer text origin changed')
 if 'let dns_start=preview.len()+64'not in compose or 'let http_start=dns_start+64'not in compose or 'let body_start=http_start+64'not in compose:raise ValueError('production status extent layout changed')
 return dict(font=font_from_source(sources['renderer']),prefix=literal('preview',r'preview\.append\((b"[^"\n]*")\)'),echo=echo,dns=dns,http=http,
  dns_prefix=literal('compose',r'append\([^;\n]*,(b"\\n\\nControlled DNS: ")\)'),dns_footer=literal('compose',r'append\([^;\n]*,(b"\\nD query  X cancel")\)'),
  http_prefix=literal('compose',r'append\([^;\n]*,(b"\\n\\nPlain HTTP / no TLS: ")\)'),http_footer=literal('compose',r'append\([^;\n]*,(b"\\nH fetch  J cancel")\)'),
  inset=number('style',r'INSET: u32 = (\d+)'),advance=number('style',r'ADVANCE: u32 = (\d+)'),line=number('style',r'LINE: u32 = (\d+)'),scale_threshold=number('style',r'width >= (\d+)'),top=number('renderer',r'FRAME_TOP_BAR_HEIGHT_PX: usize = (\d+)')+8,max_rows=number('surface',r'TEXT_MAX_ROWS: usize = (\d+)'),capacity=number('surface',r'TEXT_BUF_CAP: usize = (\d+)'),
  request=literal('wire',r'const REQUEST: &\[u8\] = (b"[^"\n]*")'),color=number('app',r'self\.ui\.queue\(&text\.bytes\[\.\.text\.len\],(0x[0-9A-Fa-f]+)'),
  source_sha256={paths[key]:hashlib.sha256(value.encode()).hexdigest()for key,value in sources.items()})

def http_status_geometry(text,expected,body,contract):
 tiles=re.findall(r'\[shell\.tile\.apply\] sid=300 x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text)
 previews=re.findall(r'\[browser\.preview\.layout\] bytes=(\d+) newline=1 cached_or_sample=1',text)
 echoes=re.findall(r'\[browser\.net\.job\.view\] state=(\w+) web=0',text)
 dns_views=re.findall(r'\[browser\.dns\.view\] state=([^\r\n]+?) web=0',text)
 if not tiles or not previews:raise ValueError('missing actual Browser geometry or preview length')
 echo=echoes[-1]if echoes else'Verified'if '[browser.net.job.released] ticket=1 view=Verified private_job=0'in text else None
 if echo not in contract['echo']:raise ValueError('missing supported preceding Echo status')
 dns=dns_views[-1]if dns_views else'Idle'
 if dns in contract['dns']:dns_label=contract['dns'][dns]
 else:
  address=re.fullmatch(r'Answer\(\[(\d+), (\d+), (\d+), (\d+)\]\)',dns)
  if not address or any(int(b)>255 for b in address.groups()):raise ValueError('unsupported preceding DNS status')
  dns_label='.'.join(address.groups()).encode()
 if expected not in contract['http']:raise ValueError('unsupported HTTP status')
 if len(body)>64 or any(b!=10 and not 32<=b<=126 for b in body):raise ValueError('invalid expected normalized HTTP body')
 x,y,w,h=map(int,tiles[-1]);length=int(previews[-1]);offset=length+128
 if not len(contract['prefix'])<length or length+258>contract['capacity']:raise ValueError('composed text exceeds production extent')
 scale=2 if w>=contract['scale_threshold']else 1;columns=max(1,min(80,(w-2*contract['inset'])//(contract['advance']*scale)))
 prior=contract['echo'][echo].ljust(64,b' ')+(contract['dns_prefix']+dns_label+contract['dns_footer']).ljust(64,b' ')
 status=(contract['http_prefix']+contract['http'][expected]+contract['http_footer']).ljust(64,b' ')
 if len(prior)!=128 or len(status)!=64:raise ValueError('invalid production fixed status extent')
 rendered_body=body.ljust(64,b' ')if expected=='Ready'else b' '*64
 data=contract['prefix']+b'?'*(length-len(contract['prefix']))+prior+status+b'\n\n'+rendered_body
 starts=[0];column=0
 for index,byte in enumerate(data):
  if byte==10:starts.append(index+1);column=0
  else:
   column+=1
   if column>=columns and index+1<len(data):starts.append(index+1);column=0
 if len(starts)>contract['max_rows']:raise ValueError('HTTP snapshot exceeds renderer row table')
 rows=[]
 for row,start in enumerate(starts):
  if start<=offset:continue
  end=starts[row+1]if row+1<len(starts)else len(data)
  rows.append(dict(row=row,text=data[start:end].rstrip(b'\n').decode('ascii'),x=x+contract['inset'],y=y+contract['top']+row*contract['line']*scale,start=start,end=end,body=end>length+194))
 if not rows or not any('Plain HTTP'in r['text']for r in rows):raise ValueError('missing complete HTTP label region')
 if any(r['y']+contract['line']*scale>y+h for r in rows):raise ValueError('complete HTTP/body rows are clipped by actual tile')
 return dict(tile=dict(x=x,y=y,w=w,h=h),scale=scale,columns=columns,preview_bytes=length,http_offset=offset,body_offset=length+194,body_bytes=64,echo_view=echo,dns_view=dns,rows=rows)

def verify_http_pixels(text,ppm,expected,body,contract):
 try:
  geometry=http_status_geometry(text,expected,body,contract);width,height,pixels=ppm_rgb(ppm);tile=geometry['tile'];scale=geometry['scale'];errors=[];checked=0;ink=0;body_checked=0;color=contract['color'].to_bytes(3,'big')
  if tile['x']<0 or tile['y']<50 or tile['x']+tile['w']>width or tile['y']+tile['h']>height:raise ValueError('actual Browser tile outside framebuffer')
  for row in geometry['rows']:
   mismatch=0
   for column in range(geometry['columns']):
    bitmap=glyph(contract['font'],row['text'][column])if column<len(row['text'])else(0,)*7
    for gy in range(contract['line']):
     for gx in range(contract['advance']):
      on=gy<7 and gx<5 and bool(bitmap[gy]&(1<<(4-gx)))
      for dy in range(scale):
       for dx in range(scale):
        px=row['x']+(column*contract['advance']+gx)*scale+dx;py=row['y']+gy*scale+dy
        actual=pixels[(py*width+px)*3:(py*width+px)*3+3]==color;mismatch+=actual!=on;checked+=1;ink+=on;body_checked+=row['body']
   if mismatch:errors.append(f'Browser HTTP/body row={row["row"]} glyph_mismatches={mismatch}')
  if not ink or not body_checked:errors.append('missing complete HTTP label or body region')
  return dict(ok=not errors,errors=errors,expected=expected,expected_body_hex=body.hex()if expected=='Ready'else'',geometry=geometry,checked_pixels=checked,body_region_pixels=body_checked,expected_ink_pixels=ink)
 except(ValueError,KeyError,OverflowError,UnicodeError)as exc:return dict(ok=False,errors=[str(exc)],expected=expected)

def wait_http_pixels(q,text,out,expected,body,contract,health,name='browser-http',timeout=30):
 began=time.monotonic();deadline=began+timeout;attempts=0;result=dict(ok=False,errors=['no screenshot']);screenshot=out/(name+'.ppm')
 while time.monotonic()<deadline:
  try:
   health();attempts+=1;q.cmd('screendump',{'filename':str(screenshot)});trace=text();raw=screenshot.read_bytes();result=verify_http_pixels(trace,raw,expected,body,contract)
   result['errors']+=verify_kaleido_layout(trace,raw,contract['font']);result['ok']=not result['errors']
   if result['ok']:break
  except Exception as exc:result=dict(ok=False,errors=['HTTP pixel poll failed: '+repr(exc)],expected=expected);break
  time.sleep(.2)
 result.update(attempts=attempts,seconds=time.monotonic()-began,screenshot=screenshot.name,screenshot_sha256=hashlib.sha256(screenshot.read_bytes()).hexdigest()if screenshot.exists()else None,source_sha256=contract['source_sha256'],scope='Complete actual tiled Plain HTTP/no TLS label, keyboard footer, padding, blank rows and all64body cells including trailing erasure, source-bound production font/style/composer and actual preceding status geometry. Serial completion alone cannot pass.')
 (out/(name+'-pixels.json')).write_text(json.dumps(result,indent=2)+'\n');return result

def wire_response(mode):
 body=BODY;status=b'HTTP/1.1 200 OK\r\n';extra=b'';length=len(body)
 if mode=='short-body':length+=9
 elif mode=='oversized':body=b'X'*2049;length=len(body)
 elif mode=='redirect':status=b'HTTP/1.1 302 Found\r\n';extra=b'Location: /elsewhere\r\n'
 elif mode=='malformed':extra=b'Content-Length: '+str(length).encode()+b'\r\n'
 return status+b'Content-Type: text/plain; charset=us-ascii\r\nContent-Length: '+str(length).encode()+b'\r\n'+extra+b'Connection: close\r\n\r\n'+body

def resolution_joins(trace,pcap,expected,dns_validation):
 """Join positive app cleanup, private admission and the actual first SYN.

 The serial egress hash/length names the exact captured frame, rather than
 treating a serial state transition or host accept call as TCP wire proof.
 """
 errors=[];joins=[]
 admissions=list(re.finditer(r'\[net\.job\.resolved\.admitted\] owner=15 task=15 ticket=(\d+) dns_generation=(\d+) observed=(\d+) expires=(\d+) now=(\d+) fixed=1 at_begin=1',trace))
 if [(int(m[1]),int(m[2]))for m in admissions]!=expected:errors.append('resolved admission tickets/generations differ from exact intended HTTP jobs')
 syns={}
 try:
  for index,frame in enumerate(frames(pcap)):
   if len(frame)<54 or frame[12:14]!=b'\x08\0':continue
   ip=frame[14:];ihl=(ip[0]&15)*4
   if ip[0]>>4!=4 or ip[9]!=6 or ip[12:16]!=bytes([10,0,2,15])or ihl<20 or len(ip)<ihl+20:continue
   tcp=ip[ihl:];port=int.from_bytes(tcp[:2],'big')
   if 48001<=port<=65535 and tcp[13]&0x12==2:
    syns.setdefault(port-48000,dict(packet=index,length=len(frame),hash=hex(fingerprint(frame))))
  for admission in admissions:
   ticket,generation,observed,expires,now=map(int,admission.groups())
   release=re.search(rf'\[browser.dns.released\] ticket={generation} view=Answer\(\[10, 0, 2, 2\]\) private_job=0',trace)
   egress=re.search(rf'\[net.job.egress.submit\] ticket={ticket} sequence=(\d+) length=(\d+) hash=(0x[0-9a-f]+) scope=dma_admission_not_delivery',trace)
   answer=next((r for r in dns_validation.get('responses',[])if r['ticket']==generation),None);syn=syns.get(ticket)
   if not release or not egress or not answer or not syn:errors.append(f'HTTP ticket{ticket} lacks positive DNS cleanup, actual answer, admission or joined SYN');continue
   if not release.start()<admission.start()<egress.start():errors.append(f'HTTP ticket{ticket} DNS release/admission/egress order is invalid')
   if not observed<=now<expires or not 0<expires-observed<=60000:errors.append(f'HTTP ticket{ticket} was admitted outside its bounded observed lifetime')
   if syn['length']!=int(egress[2])or syn['hash']!=egress[3]:errors.append(f'HTTP ticket{ticket} admission egress does not name its captured first SYN')
   if answer['packet']>=syn['packet']:errors.append(f'HTTP ticket{ticket} emitted SYN before its actual DNS answer')
   joins.append(dict(ticket=ticket,dns_generation=generation,observed_tick=observed,expires_tick=expires,admitted_tick=now,dns_release_log_offset=release.start(),admission_log_offset=admission.start(),egress_log_offset=egress.start(),egress_sequence=int(egress[1]),dns_answer_packet=answer['packet'],tcp_first_syn=syn))
 except(ValueError,IndexError,struct.error)as exc:errors.append(str(exc))
 return dict(ok=not errors,errors=errors,joins=joins,scope='Exact positive DNS RELEASE precedes retained BEGIN_RESOLVED admission; bounded admission time is observed in serial. Subsequent egress hash/length joins the actual first TCP SYN, which follows the actual captured DNS answer. DNS observation is provenance data under held BOOT authority and fixed endpoint policy, not DNS authenticity or new authority.')

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--model',choices=['e1000','e1000e'],default='e1000e');p.add_argument('--mode',choices=MODES,default='echo');a=p.parse_args()
 iso=a.iso.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 contract=load_pixel_contract(Path(__file__).resolve().parents[1]);request=contract['request'];disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';qmp=Path('/tmp')/f'sex-http-{os.getpid()}.sock'
 count=1 if a.mode in DNS_MODES else 4 if a.mode=='switch'else 3 if a.mode=='cancel-retry'else 2
 dns_count=2 if a.mode in('switch','cancel-retry')else 1
 rows=[];attempts=[];dns_rows=[];dns_attempts=[];server_errors=[];connections=[];stopping=threading.Event();http_seen=threading.Event();dns_seen=threading.Event()
 listener=socket.socket();listener.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);listener.bind(('127.0.0.1',46078));listener.listen(4);listener.settimeout(.2)
 dns_peer=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);dns_peer.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);dns_peer.bind(('127.0.0.1',46079));dns_peer.settimeout(.2)
 def serve_dns():
  try:
   while not stopping.is_set():
    try:body,remote=dns_peer.recvfrom(4096)
    except socket.timeout:continue
    ticket=len(dns_rows)+1
    row=dict(ticket=ticket,remote=list(remote),request_hex=body.hex(),request_sha256=hashlib.sha256(body).hexdigest(),host_monotonic=time.monotonic(),response_hex=None);dns_rows.append(row)
    if ticket>dns_count or body!=dns_query(ticket):raise RuntimeError('unexpected, duplicate, or noncanonical actual H DNS query')
    dns_seen.set()
    if a.mode in('dns-silence','dns-cancel','dns-close'):continue
    kind=a.mode[4:]if a.mode in DNS_SUCCESS+('dns-cname-cycle','dns-cname-conflict','dns-bad-pointer','dns-cname-unrelated')else'cname'if a.mode in('dns-cname-disallowed','dns-cname-zero-ttl')else'malformed'if a.mode=='dns-malformed'else'answer'
    reply=bytearray(dns_response(ticket,kind))
    if a.mode in('dns-disallowed','dns-cname-disallowed'):reply[-4:]=bytes([10,0,2,99])
    elif a.mode in('dns-zero-ttl','dns-cname-zero-ttl'):reply[40:44]=b'\0'*4
    row['response_hex']=bytes(reply).hex();attempt=dict(ticket=ticket,kind=a.mode if a.mode in DNS_MODES else'answer',response_hex=row['response_hex'],response_sha256=hashlib.sha256(reply).hexdigest(),remote=list(remote),host_monotonic=time.monotonic())
    try:attempt['sent_bytes']=dns_peer.sendto(reply,remote)
    except OSError as exc:attempt['send_error']=repr(exc);raise
    finally:dns_attempts.append(attempt)
  except Exception as exc:
   if not stopping.is_set():server_errors.append(repr(exc))
 dns_thread=threading.Thread(target=serve_dns,daemon=True);dns_thread.start()
 def write(conn,ticket,data,late=False):
  sent=bytearray();error=None
  try:
   for start in range(0,len(data),13):
    conn.sendall(data[start:start+13]);sent.extend(data[start:start+13]);time.sleep(.002)
  except OSError as exc:error=repr(exc)
  finally:attempts.append(dict(ticket=ticket,late_cancelled_peer=late,attempted_hex=data.hex(),sent_hex=bytes(sent).hex(),sent_sha256=hashlib.sha256(sent).hexdigest(),error=error))
  if error and not late:raise RuntimeError('response write: '+error)
  return bytes(sent)
 def serve():
  old=None
  try:
   for ticket in range(1,count+1):
    while not stopping.is_set():
     try:conn,remote=listener.accept();break
     except socket.timeout:continue
    else:return
    connections.append(conn);conn.settimeout(30);body=bytearray()
    while True:
     part=conn.recv(512)
     if not part:break
     body.extend(part)
     if len(body)>4096:raise RuntimeError('unbounded actual guest request')
    echo=ticket==1 or(a.mode=='switch'and ticket==3);wanted=ECHO if echo else request
    row=dict(ticket=ticket,kind='echo'if echo else'http',remote=list(remote),request_hex=bytes(body).hex(),request_sha256=hashlib.sha256(body).hexdigest(),response_hex='',aborted=False);rows.append(row)
    if bytes(body)!=wanted:raise RuntimeError('actual guest request differs from fixed production request')
    if echo:
     row['response_hex']=write(conn,ticket,ECHO).hex();conn.shutdown(socket.SHUT_WR);conn.close();continue
    http_seen.set()
    silent=ticket==2 and a.mode in('silence','cancel-retry','close')
    if silent:
     row['aborted']=True;old=(conn,row)
     if a.mode!='cancel-retry':
      while not stopping.wait(.1):pass
      return
     continue
    if ticket==2 and a.mode=='reset':row['aborted']=True;conn.setsockopt(socket.SOL_SOCKET,socket.SO_LINGER,struct.pack('ii',1,0));conn.close();continue
    if old is not None:
     old_conn,old_row=old;old_row['response_hex']=wire_response('echo').hex();old_row['host_sent_prefix_hex']=write(old_conn,2,wire_response('echo'),late=True).hex();old_row['response_scope']='attempted late bytes; actual captured prefix is independently qualified';old_conn.close();old=None
    reply=wire_response(a.mode if ticket==2 else'echo');row['response_hex']=write(conn,ticket,reply).hex();conn.shutdown(socket.SHUT_WR);conn.close()
  except Exception as exc:
   if not stopping.is_set():server_errors.append(repr(exc))
 thread=threading.Thread(target=serve,daemon=True);thread.start()
 cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-vga','none','-device','VGA,xres=1280,yres=800','-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{qmp},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}']
 authority_paths=('apps/kaleidoscope/src/dns_probe.rs','servers/sexnet/src/dns_jobs.rs','servers/sexnet/src/jobs.rs','servers/sexnet/src/main.rs','crates/sex-pdx/src/network_job.rs','crates/sex-net/src/dns_wire.rs')
 (out/'manifest.json').write_text(json.dumps(dict(iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),command=cmd,mode=a.mode,model=a.model,qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),pixel_source_sha256=contract['source_sha256'],authority_source_sha256={name:hashlib.sha256((Path(__file__).resolve().parents[1]/name).read_bytes()).hexdigest()for name in authority_paths},runner_sha256={name:hashlib.sha256((Path(__file__).parent/name).read_bytes()).hexdigest()for name in('http_content_gate.py','check_tcp_capture.py','check_dns_capture.py')},http_request_hex=request.hex(),http_body_hex=BODY.hex(),scope='Actual H now obtains a fixed-name UDP DNS observation, positively releases it, then requests BEGIN_RESOLVED under the existing held BOOT route and immutable HTTP endpoint policy. Real J/R/F11 keys, loopback UDP46079/TCP46078 fixtures and full shared PCAP. No DNS authenticity, TTL wall-clock accuracy, new capability authority, TLS, public routing, HTML, redirect following or cache claim.'),indent=2)+'\n')
 proc=subprocess.Popen(cmd,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);actions=[];pixel_results=[];failure=None
 def text():return serial.read_text(errors='replace')if serial.exists()else''
 def health():
  if server_errors:raise RuntimeError(str(server_errors))
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|sexnet.fatal|serial.record.overflow',text()):raise RuntimeError('guest failure')
 def wait(pattern,start=0,timeout=90):
  deadline=time.monotonic()+timeout
  while time.monotonic()<deadline:
   health();match=re.search(pattern,text()[start:])
   if match:return match
   time.sleep(.02)
  raise TimeoutError(pattern)
 def focus(q):
  for _ in range(8):
   current=re.findall(r'\[shell.focus.set\] id=(\d+)',text())
   if current and current[-1]=='300':return
   start=len(text());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':code}}}for code,down in [('alt',True),('tab',True),('tab',False),('alt',False)]]});wait(r'\[shell.app.switch\]',start)
  raise RuntimeError('actual AltTab did not focus Browser')
 def pixels(q,view,name):
  result=wait_http_pixels(q,text,out,view,BODY,contract,health,name);pixel_results.append(result)
  if not result['ok']:raise RuntimeError('actual HTTP pixels: '+'; '.join(result['errors']))
 def ready(q,ticket,start,name):
  wait(rf'\[browser.http.body\] ticket={ticket} status=200 length={len(BODY)} hash={hex(fingerprint(BODY))} controlled_peer=1 tls=0',start)
  wait(rf'\[browser.http.released\] ticket={ticket} view=Ready private_job=0',start)
  wait(rf'\[browser.view.applied\] bytes=\d+ http=2 body_bytes={len(BODY)} pixels=unconfirmed',start);pixels(q,'Ready',name)
 def resolution(ticket,generation,start):
  wait(rf'\[browser.dns.released\] ticket={generation} view=Answer\(\[10, 0, 2, 2\]\) private_job=0',start)
  wait(rf'\[net.job.resolved.admitted\] owner=15 task=15 ticket={ticket} dns_generation={generation} ',start)
 def close_pending(q,start,kind):
  began=time.monotonic();q.key('f11');wait(rf'\[browser.{kind}.closed\] pending=1 terminal=1',start);wait(r'\[sext.app.pool.ready\] target=15 .*ok=1',start)
  trace=text();roles={int(n)for n in re.findall(r'\[app.service.ack\] target=15 .*role=(\d+) ',trace[start:])};ack=trace.rfind('[net.app.cleanup.ack] target=15')
  retired=re.search(r'\[net.dns.records.retired\] target=15 task=15 remaining=0 before_cleanup_ack=1',trace[start:])
  if roles!={1,2,4,8,16}or ack<start or not retired or retired.start()+start>=ack or re.search(r'\[net\.(?:job|dns)\.egress\.submit\]',trace[ack:]):raise RuntimeError('pending Close lacks zero DNS records before ACK and terminal owned-egress cleanup')
  q.cmd('screendump',{'filename':str(out/'browser-closed.ppm')});actions.append(dict(action='actual_f11_close_pending_'+kind,roles=sorted(roles),dns_records_remaining=0,seconds=time.monotonic()-began));pixel_results.append(dict(ok=True,skipped=True,reason='Browser terminally closed; no live HTTP status surface remains. All five cleanup roles, zero retained DNS records before network cleanup ACK, and wire evidence qualify Close.'))
 try:
  wait(r'\[app.surface.presented\] target=15 surface=300');wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0')
  deadline=time.monotonic()+15
  while not qmp.exists():
   health()
   if time.monotonic()>deadline:raise RuntimeError('QMP unavailable')
   time.sleep(.02)
  q=QMP(qmp);focus(q);start=len(text());began=time.monotonic();q.key('h')
  deadline=time.monotonic()+60
  while not dns_seen.wait(.02):
   health()
   if time.monotonic()>deadline:raise RuntimeError('actual H did not emit real DNS query')
  actions.append(dict(action='actual_h_dns_then_resolved_fetch',dns_ticket=1,seconds=time.monotonic()-began))
  if a.mode in DNS_MODES:
   if a.mode=='dns-close':close_pending(q,len(text()),'dns')
   elif a.mode=='dns-cancel':
    start=len(text());began=time.monotonic();q.key('j');wait(r'\[browser.dns.released\] ticket=1 view=Cancelled private_job=0',start);wait(r'\[browser.view.applied\] bytes=\d+ http=4 body_bytes=0 ',start);pixels(q,'Cancelled','browser-http');actions.append(dict(action='actual_j_cancel_waiting_dns',seconds=time.monotonic()-began,http_job_allocated=False))
   else:
    dns_view=r'Answer\(\[10, 0, 2, 99\]\)'if a.mode in('dns-disallowed','dns-cname-disallowed')else r'Answer\(\[10, 0, 2, 2\]\)'if a.mode in('dns-zero-ttl','dns-cname-zero-ttl')else'Unavailable'
    wait(rf'\[browser.dns.released\] ticket=1 view={dns_view} private_job=0',start);wait(r'\[browser.http.view\] state=Unavailable tls=0',start);wait(r'\[browser.view.applied\] bytes=\d+ http=3 body_bytes=0 ',start);pixels(q,'Unavailable','browser-http')
   if any(int(n)>1 for n in re.findall(r'\[net.job.egress.submit\] ticket=(\d+) ',text()))or '[net.job.resolved.admitted]'in text():raise RuntimeError('DNS denial/cancellation/Close admitted new HTTP-owned egress')
  else:
   resolution(2,1,start)
   deadline=time.monotonic()+60
   while not http_seen.wait(.02):
    health()
    if time.monotonic()>deadline:raise RuntimeError('released DNS observation did not produce actual resolved HTTP request')
  if a.mode=='close':close_pending(q,len(text()),'http')
  elif a.mode=='cancel-retry':
   start=len(text());began=time.monotonic();q.key('j');wait(r'\[browser.http.released\] ticket=2 view=Cancelled private_job=0',start);wait(r'\[browser.view.applied\] bytes=\d+ http=4 body_bytes=0 ',start);pixels(q,'Cancelled','browser-cancelled');actions.append(dict(action='actual_j_cancel',ticket=2,seconds=time.monotonic()-began));start=len(text());q.key('h');resolution(3,2,start);ready(q,3,start,'browser-http');actions.append(dict(action='actual_h_fresh_retry',ticket=3,dns_generation=2))
  elif a.mode in('echo','switch')+DNS_SUCCESS:
   ready(q,2,start,'browser-http-first'if a.mode=='switch'else'browser-http')
   if a.mode=='switch':
    start=len(text());q.key('j');wait(r'\[browser.view.applied\] bytes=\d+ http=4 body_bytes=0 ',start);pixels(q,'Cancelled','browser-erased');actions.append(dict(action='actual_j_erase_previous_ready_body',job_allocated=False))
    start=len(text());q.key('r');wait(r'\[browser.net.job.released\] ticket=3 view=Verified private_job=0',start);actions.append(dict(action='actual_r_echo_same_replay_fence',ticket=3));start=len(text());q.key('h');resolution(4,2,start);ready(q,4,start,'browser-http');actions.append(dict(action='actual_h_after_echo',ticket=4,dns_generation=2))
  elif a.mode not in DNS_MODES:
   wait(r'\[browser.http.released\] ticket=2 view=Unavailable private_job=0',start);wait(r'\[browser.view.applied\] bytes=\d+ http=3 body_bytes=0 ',start);pixels(q,'Unavailable','browser-http')
  expected_bodies=[2,4]if a.mode=='switch'else[3]if a.mode=='cancel-retry'else[2]if a.mode in('echo',)+DNS_SUCCESS else[]
  if [int(n)for n in re.findall(r'\[browser.http.body\] ticket=(\d+) ',text())]!=expected_bodies:raise RuntimeError('invalid, incomplete, cancelled or absent HTTP response became usable body')
  health()
 except Exception as exc:failure=repr(exc)
 finally:
  stopping.set()
  for conn in connections:
   try:conn.close()
   except OSError:pass
  listener.close();thread.join(timeout=2)
  dns_peer.close();dns_thread.join(timeout=2)
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=10)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
 (out/'endpoint-exchanges.json').write_text(json.dumps(dict(connections=rows,response_send_attempts=attempts),indent=2)+'\n')
 (out/'dns-endpoint-exchanges.json').write_text(json.dumps(dict(queries=dns_rows,response_send_attempts=dns_attempts),indent=2)+'\n')
 expected=[]
 for row in rows:
  item=dict(payload_hex=row['request_hex'],response_hex=row['response_hex'])
  if row['aborted']:item.update(aborted=True,allow_empty_reply=True)
  expected.append(item)
 raw_pcap=pcap.read_bytes()if pcap.exists()else b''
 validation=capture(raw_pcap,expected,guest_ports=range(48001,65536),initiator='guest')if raw_pcap else dict(ok=False,errors=['missing actual full PCAP'])
 dns_expected=[dict(ticket=row['ticket'],response_hex=row['response_hex'])for row in dns_rows]
 dns_validation=dns_capture(raw_pcap,'echo',dns_attempts,expected=dns_expected)if raw_pcap and dns_expected else dict(ok=False,errors=['missing actual H DNS query/capture'])
 admissions=[]if a.mode in DNS_MODES else[(2,1),(4,2)]if a.mode=='switch'else[(2,1),(3,2)]if a.mode=='cancel-retry'else[(2,1)]
 joined=resolution_joins(text(),raw_pcap,admissions,dns_validation)
 if a.mode in DNS_SUCCESS and joined['ok']:
  # The captured alias fixtures carry23s/7s CNAME TTLs and60s A data.
  # Authority/additional TTL1 is unrelated and must not shorten admission.
  lifetime=60000 if a.mode=='dns-compressed'else 7000
  if len(joined['joins'])!=1 or joined['joins'][0]['expires_tick']-joined['joins'][0]['observed_tick']!=lifetime:
   joined['ok']=False;joined['errors'].append('resolved provenance TTL differs from the exact captured CNAME chain/final A RRset')
 authority=nic(text(),1,True,True)
 if len(rows)!=count:failure=failure or'incomplete actual peer exchanges'
 if len(dns_rows)!=dns_count:failure=failure or'incomplete actual DNS exchanges'
 if a.mode=='reset'and validation.get('ok')and not validation['connections'][1]['rst_in']:failure=failure or'missing captured real peer reset'
 result=dict(ok=failure is None and validation['ok']and dns_validation['ok']and joined['ok']and authority['ok']and not server_errors,error=failure,server_errors=server_errors,capture=validation,dns_capture=dns_validation,resolution_order=joined,authority=authority,pixels=pixel_results,actions=actions,scope='Actual H-driven fixed-name DNS query/answer and positive cleanup precede generation-bound HTTP admission under existing held BOOT authority and fixed endpoint policy. Exact first TCP SYN joins the admitted egress frame hash/length and follows the captured DNS answer. Denial modes require only baseline Echo TCP and no new HTTP-owned egress. Full HTTP/body pixels or explicit terminal Close qualify presentation/cleanup. No injected native expiry/rollback, DNS authenticity, wall-clock TTL accuracy, new capabilities, TLS, public web, HTML, cache or physical/SMP isolation claim.')
 (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(result))
 print(json.dumps(dict(ok=True,mode=a.mode,model=a.model,connections=len(rows))))
if __name__=='__main__':main()
