#!/usr/bin/env python3
"""Real app→PDX job→guest-initiated TCP→explicit loopback peer, wire proof."""
import argparse,ast,hashlib,json,os,re,shutil,socket,struct,subprocess,threading,time
from pathlib import Path
from desktop_close_gate import QMP
from check_tcp_capture import inspect as capture
from check_nic_command import inspect as nic
from check_desktop_pixels import font_from_source,glyph,verify_kaleido_layout
from check_disk_persistence import ppm_rgb

def load_pixel_contract(root):
 """Freeze source-bound text/font metrics once, before starting the guest."""
 paths={'renderer':'servers/sexdisplay/src/main.rs','preview':'apps/kaleidoscope/src/body_text.rs','probe':'apps/kaleidoscope/src/network_probe.rs','app':'apps/kaleidoscope/src/main.rs','style':'crates/sex-pdx/src/desktop_style.rs','surface':'servers/sexdisplay/src/surface_lifecycle.rs'}
 sources={key:(root/path).read_text()for key,path in paths.items()}
 def number(source,pattern):
  match=re.search(pattern,source)
  if not match:raise ValueError('production pixel contract changed: '+pattern)
  return int(match[1],0)
 prefix=re.search(r'preview\.append\((b"[^"\n]*")\)',sources['preview'])
 if not prefix:raise ValueError('missing production preview prefix')
 labels={view:ast.literal_eval(re.search(r'View::'+view+r'=>\s*(b"[^"\n]*")',sources['probe']).group(1))for view in ('Verified','Unavailable')}
 if 'Status::new(text.len())'not in sources['app']:raise ValueError('probe no longer appends to actual preview')
 if 'FRAME_TOP_BAR_HEIGHT_PX + 8'not in sources['renderer']:raise ValueError('renderer text origin changed')
 return dict(font=font_from_source(sources['renderer']),prefix=ast.literal_eval(prefix[1]),labels=labels,
  status_bytes=number(sources['probe'],r'desired:\[u8;(\d+)\]'),
  inset=number(sources['style'],r'INSET: u32 = (\d+)'),advance=number(sources['style'],r'ADVANCE: u32 = (\d+)'),line=number(sources['style'],r'LINE: u32 = (\d+)'),
  scale_threshold=number(sources['style'],r'width >= (\d+)'),top=number(sources['renderer'],r'FRAME_TOP_BAR_HEIGHT_PX: usize = (\d+)')+8,
  max_rows=number(sources['surface'],r'TEXT_MAX_ROWS: usize = (\d+)'),
  source_sha256={paths[key]:hashlib.sha256(value.encode()).hexdigest()for key,value in sources.items()})

def probe_status_geometry(text,expected,contract):
 tiles=re.findall(r'\[shell\.tile\.apply\] sid=300 x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text)
 previews=re.findall(r'\[browser\.preview\.layout\] bytes=(\d+) newline=1 cached_or_sample=1',text)
 if not tiles or not previews:raise ValueError('missing actual Browser tile or preview length')
 x,y,w,h=map(int,tiles[-1]);length=int(previews[-1]);prefix=contract['prefix']
 if not len(prefix)<length<=192:raise ValueError('invalid production Browser preview length')
 scale=2 if w>=contract['scale_threshold']else 1
 cols=max(1,min(80,(w-2*contract['inset'])//(contract['advance']*scale)))
 label=contract['labels'][expected];status=label.ljust(contract['status_bytes'],b' ')
 if len(status)!=contract['status_bytes']or length+len(status)>256:raise ValueError('invalid status replacement extent')
 # clean() collapses all body controls/newlines; only its actual byte length
 # affects wrapping. The unknown body glyphs precede the region checked here.
 data=prefix+b'?'*(length-len(prefix))+status
 starts=[0];column=0
 for index,byte in enumerate(data):
  if byte==10:
   starts.append(index+1);column=0
  else:
   column+=1
   if column>=cols and index+1<len(data):starts.append(index+1);column=0
 if len(starts)>contract['max_rows']:raise ValueError('status exceeds renderer row table')
 rows=[]
 for row,start in enumerate(starts):
  if start<=length:continue # first appended newline finishes the unknown body row
  end=starts[row+1]if row+1<len(starts)else len(data)
  line=data[start:end].rstrip(b'\n').decode('ascii')
  rows.append(dict(row=row,text=line,x=x+contract['inset'],y=y+contract['top']+row*contract['line']*scale))
 if not rows or not any('Peer probe:'in row['text']for row in rows):
  # Narrow layouts may wrap the label; the complete expected byte stream is
  # still checked, but at least its first word must be visible.
  if not rows or not any('Peer'in row['text']for row in rows):raise ValueError('missing expected status glyph region')
 if any(row['y']+contract['line']*scale>y+h for row in rows):raise ValueError('complete status region is clipped by actual tile')
 return dict(tile=dict(x=x,y=y,w=w,h=h),scale=scale,columns=cols,preview_bytes=length,status_bytes=len(status),rows=rows)

def verify_probe_status(text,ppm,expected,contract):
 try:
  geometry=probe_status_geometry(text,expected,contract);width,height,pixels=ppm_rgb(ppm)
  tile=geometry['tile'];scale=geometry['scale'];color=b'\xcd\xd6\xf4';errors=[];checked=0;ink=0
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
        actual=pixels[(py*width+px)*3:(py*width+px)*3+3]==color
        mismatch+=actual!=on;checked+=1;ink+=on
   if mismatch:errors.append(f'Browser probe row={row["row"]} glyph_mismatches={mismatch}')
  if not ink:errors.append('empty expected status ink')
  return dict(ok=not errors,expected=expected,errors=errors,geometry=geometry,checked_pixels=checked,expected_ink_pixels=ink)
 except (ValueError,KeyError,OverflowError,UnicodeError)as exc:return dict(ok=False,expected=expected,errors=[str(exc)])

def wait_probe_pixels(q,text,out,expected,contract,health,timeout=30):
 deadline=time.monotonic()+timeout;attempts=0;began=time.monotonic();result=dict(ok=False,errors=['no screenshot'])
 screenshot=out/'browser.ppm'
 while time.monotonic()<deadline:
  try:
   health();attempts+=1;q.cmd('screendump',{'filename':str(screenshot)})
   actual=screenshot.read_bytes();trace=text();result=verify_probe_status(trace,actual,expected,contract)
   result['errors']+=verify_kaleido_layout(trace,actual,contract['font']);result['ok']=not result['errors']
   if result['ok']:break
  except Exception as exc:
   result=dict(ok=False,expected=expected,errors=['pixel poll failed: '+repr(exc)]);break
  time.sleep(.2)
 result.update(attempts=attempts,seconds=time.monotonic()-began,screenshot=screenshot.name,
  screenshot_sha256=hashlib.sha256(screenshot.read_bytes()).hexdigest()if screenshot.exists()else None,
  source_sha256=contract['source_sha256'],scope='Actual sid300 tiled glyph pixels, entire appended status rows, padding and trailing erasure, frozen production font. Serial completion alone cannot pass.')
 (out/'browser-status-pixels.json').write_text(json.dumps(result,indent=2)+'\n')
 return result


def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--model',choices=['e1000','e1000e'],default='e1000e');p.add_argument('--mode',choices=['echo','cancel-retry','close','reset'],default='echo');a=p.parse_args()
 iso=a.iso.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 pixel_contract=load_pixel_contract(Path(__file__).resolve().parents[1]);pixel_result=None
 disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';qmp=Path('/tmp')/f'sex-netjob-{os.getpid()}.sock'
 payload=bytes((n*17+78)&255 for n in range(1025));rows=[];server_errors=[];stopping=threading.Event();release_first=threading.Event();connections=[]
 listener=socket.socket();listener.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);listener.bind(('127.0.0.1',46078));listener.listen(2);listener.settimeout(.2)
 def serve():
  try:
   for index in range(2 if a.mode=='cancel-retry'else 1):
    while not stopping.is_set():
     try:conn,remote=listener.accept();break
     except socket.timeout:continue
    else:return
    connections.append(conn);conn.settimeout(20);body=b'';sent=b''
    while True:
     part=conn.recv(512)
     if not part:break
     body+=part
     if len(body)>4096:raise RuntimeError('unbounded actual guest request')
     if a.mode=='echo'or index==1:conn.sendall(part);sent+=part
    rows.append(dict(connection=index+1,request_hex=body.hex(),response_hex=sent.hex(),request_sha256=hashlib.sha256(body).hexdigest(),response_sha256=hashlib.sha256(sent).hexdigest()))
    if body!=payload:raise RuntimeError('actual guest request differs from app pattern')
    if a.mode=='echo'or index==1:conn.shutdown(socket.SHUT_WR);conn.close()
    elif a.mode=='reset':conn.setsockopt(socket.SOL_SOCKET,socket.SO_LINGER,struct.pack('ii',1,0));conn.close()
    else:
     while not stopping.is_set()and not release_first.wait(.1):pass
     conn.close()
  except Exception as e:
   if not stopping.is_set():server_errors.append(repr(e))
 thread=threading.Thread(target=serve,daemon=True);thread.start()
 cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-vga','none','-device','VGA,xres=1280,yres=800','-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{qmp},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}']
 (out/'manifest.json').write_text(json.dumps(dict(iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),command=cmd,mode=a.mode,model=a.model,qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),pixel_source_sha256=pixel_contract['source_sha256'],scope='Actual guest app uses its held BOOT PDX network route and fixed controlled endpoint10.0.2.2:46078. Host fixture binds loopback only; no mock guest completion, host route change, DNS, HTTP or TLS.'),indent=2)+'\n')
 proc=subprocess.Popen(cmd,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);actions=[];failure=None
 def text():return serial.read_text(errors='replace')if serial.exists()else''
 def wait(pattern,start=0,timeout=60):
  end=time.monotonic()+timeout
  while time.monotonic()<end:
   if server_errors:raise RuntimeError(str(server_errors))
   if proc.poll()is not None:raise RuntimeError('guest exited')
   t=text()
   if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|sexnet.fatal',t):raise RuntimeError('guest failure')
   m=re.search(pattern,t[start:])
   if m:return m
   time.sleep(.02)
  raise TimeoutError(pattern)
 def focus_browser(q):
  for _ in range(8):
   found=re.findall(r'\[shell.focus.set\] id=(\d+)',text())
   if found and found[-1]=='300':return
   start=len(text());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':code}}}for code,down in [('alt',True),('tab',True),('tab',False),('alt',False)]]});wait(r'\[shell.app.switch\]',start)
  raise RuntimeError('actual global chord did not reach Browser')
 try:
  wait(r'\[app.surface.presented\] target=15 surface=300')
  end=time.monotonic()+15
  while not qmp.exists():
   if time.monotonic()>end:raise RuntimeError('QMP unavailable')
   time.sleep(.02)
  q=QMP(qmp)
  if a.mode in ['close','cancel-retry']:
   wait(r'\[net.job.state\].*ticket=1 .*to=Receiving .*queued=1025 ')
   focus_browser(q);start=len(text());began=time.monotonic()
   if a.mode=='close':
    q.key('f11');wait(r'\[browser.net.job.closed\] target=15 pending=1 terminal=1',start)
    wait(r'\[sext.app.pool.ready\] target=15 .*ok=1',start)
    roles={int(n)for n in re.findall(r'\[app.service.ack\] target=15 .*role=(\d+) ',text()[start:])}
    if roles!={1,2,4,8,16}:raise RuntimeError('missing actual pending-network cleanup roles')
    trace=text();ack=trace.rfind('[net.app.cleanup.ack] target=15')
    if ack<start or re.search(r'\[net.job.egress.submit\]',trace[ack:]):raise RuntimeError('owner egress survived cleanup ACK')
    actions.append(dict(action='actual_f11_close_pending_job',seconds=time.monotonic()-began,roles=sorted(roles)))
   else:
    q.key('c');wait(r'\[browser.net.job.released\] ticket=1 view=Cancelled private_job=0',start);release_first.set()
    actions.append(dict(action='actual_c_cancel',seconds=time.monotonic()-began));start=len(text());q.key('r')
    wait(r'\[browser.net.job.bytes\] ticket=2 length=1025 .*exact=1',start)
    wait(r'\[browser.net.job.released\] ticket=2 view=Verified private_job=0',start)
    actions.append(dict(action='actual_r_retry_fresh_ticket',ticket=2))
  elif a.mode=='reset':wait(r'\[browser.net.job.released\] ticket=1 view=Unavailable private_job=0')
  else:
   wait(r'\[browser.net.job.bytes\] ticket=1 length=1025 .*exact=1')
   wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0')
  if a.mode=='close':
   q.cmd('screendump',{'filename':str(out/'browser.ppm')})
   pixel_result=dict(ok=True,skipped=True,reason='Browser was terminally closed; no live status surface remains to render. Close is qualified by lifecycle cleanup and capture evidence.')
   (out/'browser-status-pixels.json').write_text(json.dumps(pixel_result,indent=2)+'\n')
  else:
   def pixel_health():
    if server_errors:raise RuntimeError(str(server_errors))
    if proc.poll()is not None:raise RuntimeError('guest exited during pixel polling')
   pixel_result=wait_probe_pixels(q,text,out,'Unavailable'if a.mode=='reset'else'Verified',pixel_contract,pixel_health)
   if not pixel_result['ok']:raise RuntimeError('actual Browser status pixels: '+'; '.join(pixel_result['errors']))
 except Exception as e:failure=repr(e)
 finally:
  stopping.set();release_first.set()
  for conn in connections:
   try:conn.close()
   except OSError:pass
  listener.close();thread.join(timeout=2)
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=10)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
 (out/'endpoint-exchanges.json').write_text(json.dumps(rows,indent=2)+'\n')
 expected=[dict(payload_hex=payload.hex(),aborted=a.mode!='echo',allow_empty_reply=True)]
 if a.mode=='cancel-retry':expected.append(dict(payload_hex=payload.hex()))
 validation=capture(pcap.read_bytes(),expected,guest_ports=[48001,48002],initiator='guest')if pcap.exists()else dict(ok=False,errors=['missing actual PCAP'])
 authority=nic(text(),1,True,True)
 if a.mode=='reset'and validation.get('ok')and not validation['connections'][0]['rst_in']:failure='missing captured real reset'
 if len(rows)!=len(expected):failure=failure or'incomplete actual peer exchanges'
 result=dict(ok=failure is None and validation['ok']and authority['ok']and not server_errors,error=failure,server_errors=server_errors,capture=validation,authority=authority,pixels=pixel_result,actions=actions,app_pattern_sha256=hashlib.sha256(payload).hexdigest(),scope='Actual authenticated app job and guest-initiated TCP bytes, or explicitly distinguished cancellation/terminal cleanup/reset. Request/response bytes and full PCAP retained. No web/DNS/HTTP/TLS/hostile/physical/SMP claim.')
 (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(result))
 print(json.dumps(dict(ok=True,mode=a.mode,model=a.model,bytes=1025,connections=len(rows))))
if __name__=='__main__':main()
