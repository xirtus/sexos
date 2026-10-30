#!/usr/bin/env python3
"""Actual q35 CMOS UTC queries, clear, invalid civil date and terminal cleanup.

Trusted host/QEMU configuration supplies UTC. This establishes no NTP/clock
 authenticity, physical RTC, persistent floor or certificate-validation proof.
"""
import argparse,datetime,hashlib,json,math,os,re,shutil,socket,subprocess,threading,time
from pathlib import Path
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from network_contention_gate import frozen_binding,owner_joins,cleanup_receipts,MAX_PCAP
from entropy_seed_gate import ECHO
from http_content_gate import load_pixel_contract
from check_tcp_capture import inspect as tcp_capture,frames
from check_nic_command import inspect as nic_check
from check_self_wait import inspect as wait_check
from check_desktop_pixels import glyph
from check_disk_persistence import ppm_rgb

LABELS={'Pending':b'checking','Unavailable':b'unavailable','Cancelled':b'cancelled','Uncertain':b'waiting receipt','Rollback':b'rollback; blocked'}
RTC_NORMAL='base=utc,clock=host,driftfix=none'
RTC_INVALID='base=1999-12-31T12:00:00,clock=vm,driftfix=none'

def command_profile(command,mode):
 errors=[];rtc=RTC_INVALID if mode=='invalid-year'else RTC_NORMAL
 if mode not in('normal','invalid-year','close'):errors.append('unsupported explicit clock mode')
 for flag,wanted in(('-M','q35'),('-rtc',rtc)):
  if command.count(flag)!=1 or command.index(flag)+1>=len(command)or command[command.index(flag)+1]!=wanted:errors.append('wrong exact '+flag+' profile')
 if any(arg in('-localtime','-startdate','-icount','-incoming','-loadvm','-replay','-seed')or re.search(r'(?:^|,)(?:rr|rrfile|replay)=',arg)for arg in command):errors.append('conflicting timezone/replay/time injection option')
 return dict(ok=not errors,errors=errors,rtc=rtc,scope='Exact named trusted q35/QEMU UTC configuration, not authenticated time.')

def clock_contract(root):
 contract=load_pixel_contract(root);app=(root/'apps/kaleidoscope/src/main.rs').read_text();view=(root/'apps/kaleidoscope/src/browser_view.rs').read_text();formatter=(root/'apps/kaleidoscope/src/clock_view.rs').read_text();abi=(root/'crates/sex-pdx/src/clock_source.rs').read_text()
 if 'pub fn with_clock('not in view or 'browser_view::with_clock(text,'not in app or 'start.checked_add(64)'not in view:raise ValueError('source-bound whole clock status extent changed')
 for literal in('b"\\n\\nCertificate time: "','b"\\nT sample  Q clear"'):
  if literal not in view:raise ValueError('source-bound public clock chrome changed')
 for value in LABELS.values():
  if ('b"'+value.decode()+'"')not in formatter:raise ValueError('source-bound clock label changed')
 if 'b"0000-00-00T00:00:00Z"'not in formatter:raise ValueError('source-bound UTC ISO format changed')
 for name in('MIN_SECONDS','MAX_SECONDS'):
  match=re.search(name+r':\s*u64\s*=\s*([0-9_]+)',abi)
  if not match:raise ValueError('source clock range changed')
  contract[name.lower()]=int(match[1].replace('_',''))
 for name in('apps/kaleidoscope/src/clock_view.rs','apps/kaleidoscope/src/clock_probe.rs','crates/sex-pdx/src/clock_source.rs','crates/sex-pdx/src/certificate_time.rs','servers/sexnet/src/certificate_time.rs','kernel/src/clock_source.rs'):
  contract['source_sha256'][name]=hashlib.sha256((root/name).read_bytes()).hexdigest()
 return contract

def utc_label(state,contract):
 match=re.fullmatch(r'Ready\((\d+)\)',state)
 if not match:
  if state not in LABELS:raise ValueError('unknown clock view')
  return LABELS[state]
 seconds=int(match[1])
 if not contract['min_seconds']<=seconds<=contract['max_seconds']:raise ValueError('out-of-profile UTC seconds')
 return datetime.datetime.fromtimestamp(seconds,datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ').encode()

def clock_geometry(trace,expected,contract):
 layouts=list(re.finditer(r'\[browser.clock.layout\] base=(\d+) bytes=64 viewport=(\d+) state=(Ready\(\d+\)|\w+) pixels=unconfirmed',trace))
 if not layouts or layouts[-1][3]!=expected:raise ValueError('wrong latest clock layout')
 marker=layouts[-1];base,sequence=int(marker[1]),int(marker[2]);previews=re.findall(r'\[browser.preview.layout\] bytes=(\d+) newline=1 cached_or_sample=1',trace)
 if not previews:raise ValueError('missing actual preview extent')
 length=int(previews[-1]);prefix=contract['prefix']
 if base!=length+258 or not len(prefix)<length<=192:raise ValueError('clock extent differs from complete source-bound overview')
 queued=re.findall(rf'\[shell.document.viewport\] target=15 task=15 surface=300 sequence={sequence} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) enqueued=1',trace);accepted=re.findall(rf'\[browser.viewport\] target=15 task=15 sequence={sequence} w=(\d+) h=(\d+) origin=held_shell',trace)
 if len(queued)!=1 or not accepted:raise ValueError('missing exact held-shell clock viewport')
 x,y,w,h=map(int,queued[0])
 if(w,h)!=tuple(map(int,accepted[-1])):raise ValueError('viewport geometry differs')
 if not re.search(rf'\[browser.view.applied\] bytes={base+64} http=0 body_bytes=0 pixels=unconfirmed',trace[marker.end():]):raise ValueError('missing positive complete Applied after clock layout')
 preceding=prefix+b'?'*(length-len(prefix))+contract['echo']['Verified'].ljust(64,b' ')+(contract['dns_prefix']+contract['dns']['Idle']+contract['dns_footer']).ljust(64,b' ')+(contract['http_prefix']+contract['http']['Idle']+contract['http_footer']).ljust(64,b' ')+b'\n\n'+b' '*64
 if len(preceding)!=base:raise ValueError('source overview changed')
 status=(b'\n\nCertificate time: '+utc_label(expected,contract)+b'\nT sample  Q clear').ljust(64,b' ')
 if len(status)!=64:raise ValueError('clock status overflow')
 data=preceding+status;scale=2 if w>=contract['scale_threshold']else 1;cols=min(80,max(1,(w-2*contract['inset'])//(contract['advance']*scale)));starts=[0];column=0
 for index,byte in enumerate(data):
  if byte==10:starts.append(index+1);column=0
  else:
   column+=1
   if column>=cols and index+1<len(data):starts.append(index+1);column=0
 if len(starts)>contract['max_rows']:raise ValueError('clock exceeds renderer row table')
 rows=[]
 for row,start in enumerate(starts):
  if start<=base:continue
  end=starts[row+1]if row+1<len(starts)else len(data);py=y+contract['top']+row*contract['line']*scale
  if py+contract['line']*scale>y+h:raise ValueError('clock status clipped')
  rows.append(dict(row=row,text=data[start:end].rstrip(b'\n').decode(),x=x+contract['inset'],y=py))
 if not any('Certificate time:'in row['text']for row in rows):raise ValueError('full clock label not visible')
 return dict(tile=dict(x=x,y=y,w=w,h=h),base=base,viewport=sequence,scale=scale,columns=cols,rows=rows,utc_text=utc_label(expected,contract).decode())

def verify_clock_pixels(trace,ppm,expected,contract,resolution=None):
 try:
  geometry=clock_geometry(trace,expected,contract);width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1);tile=geometry['tile'];scale=geometry['scale'];color=contract['color'].to_bytes(3,'big');checked=ink=mismatch=0
  if resolution and(width,height)!=tuple(resolution):raise ValueError('wrong framebuffer resolution')
  if tile['x']<0 or tile['y']<50 or tile['x']+tile['w']>width or tile['y']+tile['h']>height:raise ValueError('clock viewport outside framebuffer')
  for row in geometry['rows']:
   for col in range(geometry['columns']):
    bitmap=glyph(contract['font'],row['text'][col])if col<len(row['text'])else(0,)*7
    for gy in range(contract['line']):
     for gx in range(contract['advance']):
      on=gy<7 and gx<5 and bool(bitmap[gy]&(1<<(4-gx)))
      for dy in range(scale):
       for dx in range(scale):
        px=row['x']+(col*contract['advance']+gx)*scale+dx;py=row['y']+gy*scale+dy;actual=pixels[(py*width+px)*3:(py*width+px)*3+3]==color;mismatch+=actual!=on;checked+=1;ink+=on
  errors=[]if not mismatch else[f'full clock glyph/padding/erasure mismatches={mismatch}']
  if not ink:errors.append('empty expected clock ink')
  return dict(ok=not errors,errors=errors,expected=expected,geometry=geometry,checked_pixels=checked,expected_ink_pixels=ink)
 except(ValueError,KeyError,IndexError,UnicodeError,OverflowError)as exc:return dict(ok=False,errors=[str(exc)],expected=expected)

def inspect_clock(trace,mode,command,host_start,host_end):
 errors=list(command_profile(command,mode)['errors']);joins=[];closed_pending=None
 try:
  assert trace.count('[boot.clock.source] holder=13 task=13 slot=61 consumer=15 consumer_task=15 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0')==1 and trace.count('[boot.clock.source]')==1,'wrong exact BOOT clock grant'
  assert trace.count('[browser.clock.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1')==1,'raw source denial probes absent'
  assert trace.count('[sexnet.clock.source] held=1 source=q35_cmos_utc no_ticks_fallback=1 precision=1')==1,'held source metadata absent or fallback selected'
  sources=list(re.finditer(r'\[clock.source.snapshot\] pd=13 task=13 slot=61 seconds=(\d+) attempts=(\d+) precision=1 profile=q35_cmos_utc ordinal=(\d+)',trace));services=list(re.finditer(r'\[certificate.time.snapshot\] owner=15 task=15 seq=(\d+) seconds=(\d+) source=q35_cmos_utc precision=1 receipt=retained',trace));ready=list(re.finditer(r'\[browser.clock.view\] state=Ready\((\d+)\) source=q35_cmos_utc snapshot=1',trace))
  assert len(sources)==trace.count('[clock.source.snapshot]')and len(services)==trace.count('[certificate.time.snapshot]'),'foreign or malformed source/service identity'
  assert '[certificate.time.rollback]'not in trace,'unexpected rollback; native fixture does not inject rollback'
  assert '[entropy.source.sample]'not in trace and '[browser.entropy.intent]'not in trace,'clock query sampled unrelated entropy source'
  expected=1 if mode=='close'else 2;assert trace.count('[browser.clock.intent] accepted=1 explicit=1 no_ticks_fallback=1')==expected,'wrong explicit T count'
  if mode=='invalid-year':
   assert not sources and not services and not ready,'invalid1999 civil date became positive clock snapshot'
   assert trace.count('[browser.clock.view] state=Unavailable source=q35_cmos_utc snapshot=1')==2,'invalid-year did not reject both explicit reads'
  else:
   assert len(sources)==len(services)==expected,'wrong fresh source/query count'
   assert [int(m[3])for m in sources]==list(range(1,expected+1))and[int(m[1])for m in services]==list(range(1,expected+1)),'source ordinal or fresh request sequence reused'
   for source,service in zip(sources,services):
    seconds=int(source[1]);assert 1<=int(source[2])<=4 and seconds==int(service[2]),'source attempt bound or seconds join differs'
    assert math.floor(host_start)-1<=seconds<=math.ceil(host_end)+1,'snapshot outside actual trusted host UTC bracket'
    assert source.start()<service.start(),'broker snapshot preceded source'
    joins.append(dict(sequence=int(service[1]),seconds=seconds,source_offset=source.start(),service_offset=service.start(),attempts=int(source[2])))
   if mode=='normal':
    assert len(ready)==2 and[int(m[1])for m in ready]==[j['seconds']for j in joins],'ClientReady does not match each exact snapshot'
    assert all(service.start()<view.start()for service,view in zip(services,ready)),'Ready before retained snapshot'
    assert joins[1]['seconds']>joins[0]['seconds'],'second fresh UTC snapshot did not progress'
   else:
    closed=re.search(r'\[browser.clock.closed\] target=15 task=15 pending=([01]) terminal=1',trace);retired=re.search(r'\[certificate.time.retired\] target=15 task=15 before_cleanup_ack=1 high_water_retained=1',trace);ack=re.search(r'\[net.app.cleanup.ack\] target=15 task=15 client_leases=0 shared_cache=retained ok=1',trace)
    assert closed and retired and ack and closed.start()<retired.start()<ack.start(),'exact cache retirement did not precede ACK'
    closed_pending=int(closed[1]);assert len(ready)<=1,'Close received unrelated snapshots'
    if ready:assert int(ready[0][1])==joins[0]['seconds']and services[0].start()<ready[0].start()<closed.start(),'completed Close snapshot does not match retained UTC'
    if closed_pending:assert not ready,'pending Close contradicted completed Ready snapshot'
  if mode!='close':assert '[browser.clock.view] state=Cancelled source=q35_cmos_utc snapshot=1'in trace,'Q did not clear clock status'
 except(AssertionError,ValueError)as exc:errors.append(str(exc))
 return dict(ok=not errors,errors=errors,joins=joins,closed_pending=closed_pending,host_utc_bracket=dict(start=host_start,end=host_end,precision_slack_seconds=1),scope='Fresh exact source/service/client UTC snapshots under explicit trusted q35 host-clock configuration; independent Python UTC glyph conversion. No injected native rollback, NTP/authenticity, physical RTC, persistent cross-reboot floor or certificate validation proof.')

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--mode',choices=['normal','invalid-year','close'],default='normal');p.add_argument('--model',choices=['e1000','e1000e'],default='e1000e');p.add_argument('--resolution',choices=['1280x800','1024x768'],default='1280x800');a=p.parse_args()
 root=Path(__file__).resolve().parents[1];iso=a.iso.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 binding=frozen_binding(root,iso,a.binding.resolve(strict=True));contract=clock_contract(root)
 if json.loads(a.binding.read_text())['runner_sha256'].get('scripts/certificate_time_gate.py')!=hashlib.sha256(Path(__file__).read_bytes()).hexdigest():raise RuntimeError('clock gate absent from frozen cohort')
 listener=socket.socket();listener.bind(('127.0.0.1',46078));listener.listen(1);listener.settimeout(.1);stop=threading.Event();rows=[];server_errors=[]
 def peer():
  conn=None
  try:
   while not stop.is_set():
    try:conn,_=listener.accept();break
    except socket.timeout:continue
   if conn is None:return
   conn.settimeout(25);data=bytearray()
   while True:
    part=conn.recv(512)
    if not part:break
    data.extend(part)
    if len(data)>1025:raise RuntimeError('unexpected clock network payload')
   if data!=ECHO:raise RuntimeError('wrong public baseline Echo bytes')
   conn.sendall(ECHO);conn.shutdown(socket.SHUT_WR);conn.close();conn=None;rows.append(dict(payload_hex=ECHO.hex(),response_hex=ECHO.hex()))
   while not stop.is_set():
    try:extra,_=listener.accept();extra.close();raise RuntimeError('T/Q opened extra TCP')
    except socket.timeout:continue
  except Exception as exc:
   if not stop.is_set():server_errors.append(repr(exc))
  finally:
   if conn:conn.close()
 thread=threading.Thread(target=peer,daemon=True);thread.start();disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-time-{os.getpid()}.sock';rtc=RTC_INVALID if a.mode=='invalid-year'else RTC_NORMAL
 command=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-rtc',rtc,'-m','512M','-vga','none','-device','VGA,xres='+a.resolution.replace('x',',yres='),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}'];profile=command_profile(command,a.mode);assert profile['ok']
 manifest=dict(binding=binding,command=command,qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),mode=a.mode,model=a.model,resolution=a.resolution,rtc_profile=profile,input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),pixel_source_sha256=contract['source_sha256']);host_start=time.time();manifest['host_utc_start']=host_start
 (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic();pixels=[];actions=[];failure=None;cleanup=None
 def text():return serial.read_text(errors='replace')if serial.exists()else''
 def health():
  if time.monotonic()-began>180:raise TimeoutError('bounded180-second whole gate')
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if server_errors:raise RuntimeError(str(server_errors))
  if any(p.exists()and p.stat().st_size>MAX_PCAP for p in(serial,pcap)):raise RuntimeError('bounded raw evidence exceeded')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',text()):raise RuntimeError('guest failure')
 def wait(pattern,start=0,timeout=60,interval=.01):
  end=time.monotonic()+timeout
  while time.monotonic()<end:
   health();m=re.search(pattern,text()[start:])
   if m:return m
   time.sleep(interval)
  raise TimeoutError(pattern)
 def key(code,pattern):
  start=len(text());q.key(code);return wait(pattern,start)
 def picture(state,name):
  end=time.monotonic()+20;path=out/(name+'.ppm');result=dict(ok=False,errors=['no frame'])
  while time.monotonic()<end:
   health();q.cmd('screendump',{'filename':str(path)});result=verify_clock_pixels(text(),path.read_bytes(),state,contract,tuple(map(int,a.resolution.split('x'))))
   if result['ok']:break
   time.sleep(.1)
  result.update(screenshot=path.name,screenshot_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),source_sha256=contract['source_sha256']);pixels.append(result);(out/(name+'-pixels.json')).write_text(json.dumps(result,indent=2)+'\n')
  if not result['ok']:raise RuntimeError(str(result['errors']))
 try:
  end=time.monotonic()+15
  while not sock.exists():
   health()
   if time.monotonic()>end:raise TimeoutError('QMP unavailable')
   time.sleep(.05)
  q=QMP(sock)
  if a.resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0');focus_surface(300,text,key);key('grave_accent',r'\[shell.palette.item\] idx=0')
  for old,new in[(0,9),(9,8)]:key('k',rf'\[shell.palette.select\] old={old} new={new}')
  start=len(text());q.key('ret');wait(r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1',start);wait(r'\[browser.viewport\] target=15 task=15 sequence=\d+ w=\d+ h=\d+ origin=held_shell',start)
  start=len(text());q.key('t');wait(r'\[shell.browser.input\] code=20 down=1 slot=43 accepted=1',start)
  if a.mode=='close':
   wait(r'\[certificate.time.snapshot\] owner=15 task=15 seq=1 seconds=\d+ source=q35_cmos_utc precision=1 receipt=retained',start,interval=.002);closed=len(text());q.key_sequence(['f11']);wait(r'\[browser.clock.closed\] target=15 task=15 pending=[01] terminal=1',closed);wait(r'\[sext.app.pool.ready\] target=15 task=15 .*ok=1',closed);cleanup=cleanup_receipts(text(),15,15,closed);q.cmd('screendump',{'filename':str(out/'clock-closed.ppm')});actions.append(dict(action='actual_F11_after_retained_snapshot',pending_claim='Observed pending0or1 reported exactly; no race-dependent pending claim.'))
  else:
   expected=r'Ready\(\d+\)'if a.mode=='normal'else'Unavailable';first=wait(rf'\[browser.clock.view\] state=({expected}) source=q35_cmos_utc snapshot=1',start);picture(first[1],'clock-first')
   start=len(text());q.key('q');wait(r'\[shell.browser.input\] code=16 down=1 slot=43 accepted=1',start);wait(r'\[browser.clock.view\] state=Cancelled source=q35_cmos_utc snapshot=1',start);picture('Cancelled','clock-cleared');delay=time.monotonic();time.sleep(1.25);elapsed=time.monotonic()-delay
   start=len(text());q.key('t');second=wait(rf'\[browser.clock.view\] state=({expected}) source=q35_cmos_utc snapshot=1',start);picture(second[1],'clock-second');actions.append(dict(action='actual_T_Q_T',between_clear_and_next_query_seconds=elapsed));assert elapsed>=1.2
 except Exception as exc:failure=repr(exc)
 finally:
  host_end=time.time();stop.set();listener.close();thread.join(timeout=1)
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
 manifest['host_utc_end']=host_end;(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');raw=pcap.read_bytes()if pcap.exists()else b'';trace=text();wire=tcp_capture(raw,rows,guest_ports=range(48001,65536),initiator='guest')if raw else dict(ok=False,errors=['missing actual wire capture']);owners=owner_joins(trace,raw,[(15,15)]);clock=inspect_clock(trace,a.mode,command,host_start,host_end);authority=nic_check(trace,expected=1,preparation=True,ethernet=True,network_config=True,routing=True);waiting=wait_check(trace)
 try:
  for frame in frames(raw):
   if len(frame)>=42 and frame[12:14]==b'\x08\0'and frame[23]==17 and frame[26:30]==bytes([10,0,2,15]):
    at=14+(frame[14]&15)*4
    if int.from_bytes(frame[at+2:at+4],'big')==46079:raise ValueError('clock intent emitted DNS')
 except(ValueError,IndexError)as exc:failure=failure or str(exc)
 if len(rows)!=1:failure=failure or'wrong sole public baseline flow'
 (out/'endpoint-exchanges.json').write_text(json.dumps(dict(public_baseline=rows),indent=2)+'\n');result=dict(ok=failure is None and not server_errors and all(v['ok']for v in(wire,owners,clock,authority,waiting)),error=failure,server_errors=server_errors,mode=a.mode,binding=binding,clock=clock,public_wire=wire,owner_packet_joins=owners,authority=authority,self_wait=waiting,pixels=pixels,cleanup=cleanup,actions=actions,scope='Actual T/Q/T public UTC snapshots through exact typed BOOT source, sequence-bound service/client and independent Python UTC full glyphs; source seconds are bracketed by observed host time under explicit trusted RTC config. Invalid1999 profile rejects reads without tick fallback. Close reports actualpending0or1 and exact cache retirement before five-role cleanup. No native rollback injection, authenticated/NTP/physical time, persistent cross-reboot floor or certificate validation proof.')
 (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(result))
 print(json.dumps(dict(ok=True,mode=a.mode,closed_pending=clock['closed_pending'])))

if __name__=='__main__':main()
