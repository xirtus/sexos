#!/usr/bin/env python3
"""Actual keyboard→app PDX DNS job→guest UDP, with explicit loopback peer."""
import argparse,ast,hashlib,json,os,re,shutil,socket,subprocess,threading,time
from pathlib import Path
from desktop_close_gate import QMP
from check_dns_capture import MODES,query,response,inspect
from check_nic_command import inspect as nic
from outbound_job_gate import load_pixel_contract as load_probe_contract
from check_desktop_pixels import glyph,verify_kaleido_layout
from check_disk_persistence import ppm_rgb

def load_pixel_contract(root):
 """Bind both preceding text extents and DNS labels to production sources."""
 contract=load_probe_contract(root)
 dns_path='apps/kaleidoscope/src/browser_view.rs';source=(root/dns_path).read_text();app=(root/'apps/kaleidoscope/src/main.rs').read_text()
 def number(pattern):
  match=re.search(pattern,source)
  if not match:raise ValueError('production DNS pixel contract changed: '+pattern)
  return int(match[1])
 def literal(pattern):
  match=re.search(pattern,source)
  if not match:raise ValueError('production DNS status literal changed: '+pattern)
  return ast.literal_eval(match[1])
 if number(r'dns_start=preview\.len\(\)\+(\d+)')!=contract['status_bytes']:raise ValueError('DNS no longer follows complete TCP status')
 if 'dns_probe::View::Answer(address)'not in source or 'decimal(&mut text.bytes[..dns_start+64],&mut at,*byte)'not in source:raise ValueError('DNS decimal renderer changed')
 network_labels={view:ast.literal_eval(label)for view,label in re.findall(r'network_probe::View::(\w+)=>\s*(b"[^"\n]*")',source)}
 dns_labels={view:ast.literal_eval(label)for view,label in re.findall(r'dns_probe::View::(\w+)=>append\([^\n]*,(b"[^"\n]*")\)',source)}
 contract.update(dns_bytes=number(r'http_start=dns_start\+(\d+)'),capacity=number(r'bytes:\[u8;(\d+)\]'),dns_prefix=literal(r'append\([^\n]*,(b"\\n\\nControlled DNS: ")\)'),dns_footer=literal(r'append\([^\n]*,(b"\\nD query  X cancel")\)'),network_labels=network_labels,dns_labels=dns_labels)
 contract['source_sha256'][dns_path]=hashlib.sha256(source.encode()).hexdigest()
 return contract

def dns_status_geometry(text,expected,contract):
 tiles=re.findall(r'\[shell\.tile\.apply\] sid=300 x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text)
 previews=re.findall(r'\[browser\.preview\.layout\] bytes=(\d+) newline=1 cached_or_sample=1',text)
 views=re.findall(r'\[browser\.net\.job\.view\] state=(\w+) web=0',text)
 if not tiles or not previews:raise ValueError('missing actual Browser tile or preview length')
 if not views:
  if '[browser.net.job.released] ticket=1 view=Unavailable private_job=0'not in text:raise ValueError('missing preceding TCP status view')
  network_view='Unavailable'
 else:network_view=views[-1]
 if network_view not in contract['network_labels']:raise ValueError('unsupported preceding TCP status view')
 x,y,w,h=map(int,tiles[-1]);length=int(previews[-1]);prefix=contract['prefix'];offset=length+contract['status_bytes']
 if not len(prefix)<length or offset+contract['dns_bytes']>contract['capacity']:raise ValueError('DNS status exceeds production text extent')
 label=b'10.0.2.2'if expected=='Answer'else contract['dns_labels'].get(expected)
 if label is None:raise ValueError('unsupported DNS status view')
 status=(contract['dns_prefix']+label+contract['dns_footer']).ljust(contract['dns_bytes'],b' ')
 network=contract['network_labels'][network_view].ljust(contract['status_bytes'],b' ')
 if len(status)!=contract['dns_bytes']or len(network)!=contract['status_bytes']:raise ValueError('status exceeds its fixed replacement extent')
 scale=2 if w>=contract['scale_threshold']else 1
 cols=max(1,min(80,(w-2*contract['inset'])//(contract['advance']*scale)))
 data=prefix+b'?'*(length-len(prefix))+network+status;starts=[0];column=0
 for index,byte in enumerate(data):
  if byte==10:starts.append(index+1);column=0
  else:
   column+=1
   if column>=cols and index+1<len(data):starts.append(index+1);column=0
 if len(starts)>contract['max_rows']:raise ValueError('DNS status exceeds production renderer row table')
 rows=[]
 for row,start in enumerate(starts):
  if start<=offset:continue # first appended newline terminates preceding status text
  end=starts[row+1]if row+1<len(starts)else len(data)
  rows.append(dict(row=row,text=data[start:end].rstrip(b'\n').decode('ascii'),x=x+contract['inset'],y=y+contract['top']+row*contract['line']*scale))
 if not rows or not any('Controlled'in row['text']for row in rows):raise ValueError('DNS label is not visible as a supported complete status region')
 if any(row['y']+contract['line']*scale>y+h for row in rows):raise ValueError('complete DNS status is clipped by actual tile')
 return dict(tile=dict(x=x,y=y,w=w,h=h),scale=scale,columns=cols,preview_bytes=length,network_view=network_view,dns_offset=offset,status_bytes=len(status),rows=rows)

def verify_dns_status(text,ppm,expected,contract):
 try:
  geometry=dns_status_geometry(text,expected,contract);width,height,pixels=ppm_rgb(ppm);tile=geometry['tile'];scale=geometry['scale'];errors=[];checked=0;ink=0
  if tile['x']<0 or tile['y']<50 or tile['x']+tile['w']>width or tile['y']+tile['h']>height:raise ValueError('actual Browser tile is outside framebuffer')
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
        actual=pixels[(py*width+px)*3:(py*width+px)*3+3]==b'\xcd\xd6\xf4'
        mismatch+=actual!=on;checked+=1;ink+=on
   if mismatch:errors.append(f'Browser DNS row={row["row"]} glyph_mismatches={mismatch}')
  if not ink:errors.append('empty expected DNS status ink')
  return dict(ok=not errors,expected=expected,errors=errors,geometry=geometry,checked_pixels=checked,expected_ink_pixels=ink)
 except(ValueError,KeyError,OverflowError,UnicodeError)as exc:return dict(ok=False,expected=expected,errors=[str(exc)])

def wait_dns_pixels(q,text,out,expected,contract,health,timeout=30):
 deadline=time.monotonic()+timeout;attempts=0;began=time.monotonic();result=dict(ok=False,errors=['no screenshot']);screenshot=out/'browser.ppm'
 while time.monotonic()<deadline:
  try:
   health();attempts+=1;q.cmd('screendump',{'filename':str(screenshot)});actual=screenshot.read_bytes();trace=text();result=verify_dns_status(trace,actual,expected,contract)
   result['errors']+=verify_kaleido_layout(trace,actual,contract['font']);result['ok']=not result['errors']
   if result['ok']:break
  except Exception as exc:result=dict(ok=False,expected=expected,errors=['DNS pixel poll failed: '+repr(exc)]);break
  time.sleep(.2)
 result.update(attempts=attempts,seconds=time.monotonic()-began,screenshot=screenshot.name,screenshot_sha256=hashlib.sha256(screenshot.read_bytes()).hexdigest()if screenshot.exists()else None,source_sha256=contract['source_sha256'],scope='Actual sid300 DNS status glyph pixels, complete appended status rows including blank/padding/trailing erasure. Source-bound TCP status extent, decimal address, font, style, latest tile and actual preview length. A serial answer alone cannot pass.')
 (out/'browser-dns-status-pixels.json').write_text(json.dumps(result,indent=2)+'\n');return result

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--model',choices=['e1000','e1000e'],default='e1000e');p.add_argument('--mode',choices=MODES,default='echo');a=p.parse_args()
 iso=a.iso.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 pixel_contract=load_pixel_contract(Path(__file__).resolve().parents[1]);pixel_result=None
 disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';qmp=Path('/tmp')/f'sex-dnsjob-{os.getpid()}.sock'
 exchanges=[];attempts=[];server_errors=[];stopping=threading.Event();got_query=threading.Event()
 peer=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);peer.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);peer.bind(('127.0.0.1',46079));peer.settimeout(.2)
 def send(ticket,remote,kind):
  body=response(ticket,kind);row=dict(ticket=ticket,kind=kind,response_hex=body.hex(),response_sha256=hashlib.sha256(body).hexdigest(),remote=list(remote),host_monotonic=time.monotonic())
  try:row['sent_bytes']=peer.sendto(body,remote)
  except OSError as exc:row['send_error']=repr(exc);raise
  finally:attempts.append(row)
 def serve():
  old=None
  try:
   while not stopping.is_set():
    try:body,remote=peer.recvfrom(4096)
    except socket.timeout:continue
    ticket=len(exchanges)+1
    exchanges.append(dict(ticket=ticket,remote=list(remote),request_hex=body.hex(),request_sha256=hashlib.sha256(body).hexdigest(),host_monotonic=time.monotonic()))
    if ticket>(2 if a.mode=='cancel-retry'else 1)or body!=query(ticket):raise RuntimeError('unexpected, duplicate, or noncanonical actual guest DNS query')
    got_query.set()
    if a.mode=='cancel-retry':
     if ticket==1:old=remote;continue
     send(1,old,'answer');time.sleep(.3);send(2,remote,'answer')
    elif a.mode=='wrong-context':
     send(ticket,remote,'wrong-id');time.sleep(.3);send(ticket,remote,'wrong-question');time.sleep(.3);send(ticket,remote,'answer')
    elif a.mode not in('silence','close'):send(ticket,remote,'answer'if a.mode=='echo'else a.mode)
  except Exception as exc:
   if not stopping.is_set():server_errors.append(repr(exc))
 thread=threading.Thread(target=serve,daemon=True);thread.start()
 cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-vga','none','-device','VGA,xres=1280,yres=800','-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{qmp},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}']
 (out/'manifest.json').write_text(json.dumps(dict(iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),command=cmd,mode=a.mode,model=a.model,qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),runner_sha256={name:hashlib.sha256((Path(__file__).parent/name).read_bytes()).hexdigest()for name in('dns_query_gate.py','check_dns_capture.py')},pixel_source_sha256=pixel_contract['source_sha256'],scope='Actual app holds its BOOT PDX route; real D/X/F11 keyboard invokes fixed-name DNS service data jobs. Host fixture binds loopback UDP46079 only. Source-bound live DNS status pixels are required except after terminal Close. No public DNS, host route mutation, cache, HTTP or TLS.'),indent=2)+'\n')
 proc=subprocess.Popen(cmd,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);actions=[];failure=None
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
 def focus_browser(q):
  for _ in range(8):
   focus=re.findall(r'\[shell.focus.set\] id=(\d+)',text())
   if focus and focus[-1]=='300':return
   start=len(text());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':code}}}for code,down in [('alt',True),('tab',True),('tab',False),('alt',False)]]});wait(r'\[shell.app.switch\]',start)
  raise RuntimeError('actual AltTab could not focus Browser')
 try:
  wait(r'\[app.surface.presented\] target=15 surface=300')
  wait(r'\[browser.net.job.released\] ticket=1 view=Unavailable private_job=0')
  deadline=time.monotonic()+15
  while not qmp.exists():
   health()
   if time.monotonic()>deadline:raise RuntimeError('QMP unavailable')
   time.sleep(.02)
  q=QMP(qmp);focus_browser(q);start=len(text());q.key('d');began=time.monotonic()
  deadline=time.monotonic()+60
  while not got_query.wait(.02):
   health()
   if time.monotonic()>deadline:raise RuntimeError('real D key did not emit actual DNS query')
  actions.append(dict(action='actual_d_query',ticket=1,seconds=time.monotonic()-began))
  if a.mode=='close':
   start=len(text());began=time.monotonic();q.key('f11');wait(r'\[browser.dns.closed\].*pending=1',start)
   wait(r'\[sext.app.pool.ready\] target=15 .*ok=1',start)
   trace=text();roles={int(n)for n in re.findall(r'\[app.service.ack\] target=15 .*role=(\d+) ',trace[start:])}
   if roles!={1,2,4,8,16}:raise RuntimeError('pending DNS close lacks all five cleanup roles')
   ack=trace.rfind('[net.app.cleanup.ack] target=15')
   if ack<start or re.search(r'\[net.dns.egress.submit\]',trace[ack:]):raise RuntimeError('DNS owned egress survived cleanup ACK')
   actions.append(dict(action='actual_f11_pending_dns_close',roles=sorted(roles),seconds=time.monotonic()-began))
  elif a.mode=='cancel-retry':
   start=len(text());began=time.monotonic();q.key('x');wait(r'\[browser.dns.released\] ticket=1 view=Cancelled private_job=0',start)
   actions.append(dict(action='actual_x_cancel',ticket=1,seconds=time.monotonic()-began));start=len(text());q.key('d')
   wait(r'\[browser.dns.answer\] ticket=2 address=10\.0\.2\.2 controlled_peer=1 tls=0 web=0',start)
   wait(r'\[browser.dns.released\] ticket=2 view=Answer\(\[10, 0, 2, 2\]\) private_job=0',start)
   actions.append(dict(action='actual_d_fresh_retry',ticket=2))
  elif a.mode in('echo','wrong-context'):
   wait(r'\[browser.dns.answer\] ticket=1 address=10\.0\.2\.2 controlled_peer=1 tls=0 web=0',start)
   wait(r'\[browser.dns.released\] ticket=1 view=Answer\(\[10, 0, 2, 2\]\) private_job=0',start)
  else:wait(r'\[browser.dns.released\] ticket=1 view=Unavailable private_job=0',start)
  answers=[int(t)for t in re.findall(r'\[browser.dns.answer\] ticket=(\d+) ',text())]
  expected_answers=[2]if a.mode=='cancel-retry'else[1]if a.mode in('echo','wrong-context')else[]
  if answers!=expected_answers:raise RuntimeError('bad, unrelated, cancelled or absent DNS packet became usable app answer')
  if a.mode=='close':
   q.cmd('screendump',{'filename':str(out/'browser.ppm')});pixel_result=dict(ok=True,skipped=True,reason='Browser is terminally closed; no live DNS status surface remains. Close is qualified by lifecycle cleanup and captured query evidence.')
   (out/'browser-dns-status-pixels.json').write_text(json.dumps(pixel_result,indent=2)+'\n')
  else:
   expected='Answer'if a.mode in('echo','wrong-context','cancel-retry')else'Unavailable'
   pixel_result=wait_dns_pixels(q,text,out,expected,pixel_contract,health)
   if not pixel_result['ok']:raise RuntimeError('actual Browser DNS status pixels: '+'; '.join(pixel_result['errors']))
  health()
 except Exception as exc:failure=repr(exc)
 finally:
  stopping.set();peer.close();thread.join(timeout=2)
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=10)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
 (out/'endpoint-exchanges.json').write_text(json.dumps(dict(queries=exchanges,response_send_attempts=attempts),indent=2)+'\n')
 validation=inspect(pcap.read_bytes(),a.mode,attempts)if pcap.exists()else dict(ok=False,errors=['missing full actual PCAP'])
 authority=nic(text(),1,True,True)
 if len(exchanges)!=(2 if a.mode=='cancel-retry'else 1):failure=failure or'incomplete actual host-observed queries'
 result=dict(ok=failure is None and validation['ok']and authority['ok']and not server_errors,error=failure,server_errors=server_errors,capture=validation,authority=authority,actions=actions,pixels=pixel_result,scope='Actual keyboard-driven capability-backed app DNS job, wire query/response bytes, cancellation or terminal cleanup as selected. Complete source-bound live DNS status pixels are required except after terminal Close. Host send attempts are distinguished from actual captured packets. No public resolver, DNS cache, HTTP, TLS or physical/SMP containment claim.')
 (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(result))
 print(json.dumps(dict(ok=True,mode=a.mode,model=a.model,queries=len(exchanges),captured_responses=len(validation['responses']))))
if __name__=='__main__':main()
