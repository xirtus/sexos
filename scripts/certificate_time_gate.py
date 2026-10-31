#!/usr/bin/env python3
"""Actual q35 CMOS UTC queries, clear, invalid civil date and terminal cleanup.

Trusted host/QEMU configuration supplies UTC. This establishes no NTP/clock
 authenticity, physical RTC, persistent floor or certificate-validation proof.
"""
import argparse,datetime,gzip,hashlib,json,math,os,re,shutil,socket,subprocess,threading,time
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
from check_document_disk import fnv
from http_content_gate import frame_prefix

LABELS={'Pending':b'checking','Unavailable':b'unavailable','Cancelled':b'cancelled','Uncertain':b'waiting receipt','Rollback':b'rollback; blocked'}
RTC_NORMAL='base=utc,clock=host,driftfix=none'
RTC_INVALID='base=1999-12-31T12:00:00,clock=vm,driftfix=none'
SERIAL_UNITS='Complete UTF-8 prefix characters; universal newlines; incomplete trailing scalar excluded'

def clock_serial_text(path):
 raw=Path(path).read_bytes()
 try:text=raw.decode('utf-8',errors='strict')
 except UnicodeDecodeError as error:
  if error.reason!='unexpected end of data'or error.end!=len(raw):raise ValueError('invalid interior UTF-8 in clock serial')from error
  text=raw[:error.start].decode('utf-8',errors='strict')
 return text.replace('\r\n','\n').replace('\r','\n')

def require_clock_inputs(root,held,contract):
 from https_asset_gate import script_dependencies
 required=dict(contract['source_sha256'])
 for name in script_dependencies(root,('scripts/certificate_time_gate.py','scripts/tests/test_certificate_time_gate.py')):
  required[name]=hashlib.sha256((Path(root)/name).read_bytes()).hexdigest()
 for name,digest in required.items():
  if held.get('sources',{}).get(name,held.get('runner_sha256',{}).get(name))!=digest:raise ValueError('clock dependency missing/changed in sealed binding: '+name)
 return required

def command_profile(command,mode):
 errors=[];rtc=RTC_INVALID if mode=='invalid-year'else RTC_NORMAL
 if mode not in('normal','invalid-year','close'):errors.append('unsupported explicit clock mode')
 for flag,wanted in(('-M','q35'),('-rtc',rtc)):
  if command.count(flag)!=1 or command.index(flag)+1>=len(command)or command[command.index(flag)+1]!=wanted:errors.append('wrong exact '+flag+' profile')
 if any(arg in('-localtime','-startdate','-icount','-incoming','-loadvm','-replay','-seed')or re.search(r'(?:^|,)(?:rr|rrfile|replay)=',arg)for arg in command):errors.append('conflicting timezone/replay/time injection option')
 return dict(ok=not errors,errors=errors,rtc=rtc,scope='Exact named trusted q35/QEMU UTC configuration, not authenticated time.')

def clock_contract(root):
 contract=load_pixel_contract(root);app=(root/'apps/kaleidoscope/src/main.rs').read_text();formatter=(root/'apps/kaleidoscope/src/clock_view.rs').read_text();abi=(root/'crates/sex-pdx/src/clock_source.rs').read_text();calendar=(root/'crates/sex-time/src/lib.rs').read_text()
 if 'clock_view::render(self.import_viewport.0,self.import_viewport.1,self.clock_view)'not in app or '[browser.clock.applied]'not in app:raise ValueError('actual dedicated clock frame/Applied binding changed')
 for literal in('Clock / last query snapshot','SexTime ','Long Count ','UTC day / GMT 584283','q35 CMOS / 1-second precision','Snapshot only / not live','T sample  Q clear'):
  if ('b"'+literal+'"')not in formatter:raise ValueError('source-bound calendar label changed: '+literal)
 if 'UNIX_OFFSET: i64 = 1_356_048_000'not in calendar or 'GMT_CORRELATION: i64 = 584_283'not in calendar:raise ValueError('declared SexTime epoch/correlation changed')
 if '.min(63)'not in formatter or 'cols < 20 || rows < 8'not in formatter:raise ValueError('bounded clock measure/fallback changed')
 for value in LABELS.values():
  if ('b"'+value.decode()+'"')not in formatter:raise ValueError('source-bound clock label changed')
 if 'b"0000-00-00T00:00:00Z"'not in formatter:raise ValueError('source-bound UTC ISO format changed')
 for name in('MIN_SECONDS','MAX_SECONDS'):
  match=re.search(name+r':\s*u64\s*=\s*([0-9_]+)',abi)
  if not match:raise ValueError('source clock range changed')
  contract[name.lower()]=int(match[1].replace('_',''))
 for name in('apps/kaleidoscope/Cargo.toml','apps/kaleidoscope/src/clock_view.rs','apps/kaleidoscope/src/clock_probe.rs','crates/sex-time/src/lib.rs','crates/sex-time/Cargo.toml','crates/sex-pdx/src/clock_source.rs','crates/sex-pdx/src/certificate_time.rs','servers/sexnet/src/certificate_time.rs','kernel/src/clock_source.rs'):
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

def calendar_frame(w,h,state,contract):
 """Independent Python civil calendar and bounded physical row model."""
 scale=2 if w>=contract['scale_threshold']else 1
 cols=min(63,max(0,(w-2*contract['inset'])//(contract['advance']*scale)-1))
 available=h-contract['top']-7*scale
 rows=min(24,512//(cols+1),1+available//(contract['line']*scale))if cols and available>=0 else 0
 match=re.fullmatch(r'Ready\((\d+)\)',state);sample=None
 if match:
  seconds=int(match[1])
  if contract['min_seconds']<=seconds<=contract['max_seconds']:sample=datetime.datetime.fromtimestamp(seconds,datetime.timezone.utc)
  label='Ready / enlarge tile'if sample else'Invalid snapshot'
 else:
  labels=dict(Idle='No sample',Pending='Pending',Unavailable='Unavailable',Cancelled='Cancelled',Uncertain='Waiting receipt',Rollback='Rollback / blocked')
  if state not in labels:raise ValueError('unknown calendar snapshot state')
  label=labels[state]
 if cols<20 or rows<8:lines=[label,'T sample  Q clear','q35 CMOS / 1s','Snapshot / not live']
 else:
  wide=cols>=32;lines=['Clock / last query snapshot'if wide else'Clock / last query']
  if sample:
   # datetime/ordinal are independent of the guest era arithmetic. Explicit
   # Unix input adaptation preserves the existing wire epoch and trust.
   days=sample.date().toordinal()+1721425-584283
   places=(days//144000,days%144000//7200,days%7200//360,days%360//20,days%20)
   lines += [sample.strftime('%Y-%m-%dT%H:%M:%SZ'),f'SexTime {seconds-1356048000} s',('Long Count 'if wide else'LC ')+'.'.join(map(str,places))]
  else:lines += [label,'No date snapshot','SexTime / Long Count unavailable'if wide else'No calendar values']
  lines += ['UTC day / GMT 584283'if wide else'UTC day GMT584283','q35 CMOS / 1-second precision'if wide else'q35 CMOS / 1s','Snapshot only / not live'if wide else'Snapshot / not live','T sample  Q clear']
 padded=[(lines[row]if row<len(lines)else'').encode()[:cols].ljust(cols,b' ')for row in range(rows)]
 data=b'\n'.join(padded)
 return dict(data=data,lines=padded,cols=cols,rows=rows,scale=scale,hash=hex(fnv(data)),utc_text=sample.strftime('%Y-%m-%dT%H:%M:%SZ')if sample else None,full_calendar=cols>=20 and rows>=8)

def clock_geometry(trace,expected,contract):
 layouts=list(re.finditer(r'\[browser.clock.layout\] bytes=(\d+) hash=(0x[0-9a-f]+) cols=(\d+) rows=(\d+) viewport=(\d+) state=(Ready\(\d+\)|\w+) snapshot_only=1 pixels=unconfirmed',trace))
 if not layouts or layouts[-1][6]!=expected:raise ValueError('wrong latest clock layout')
 marker=layouts[-1];length,cols,rows,sequence=map(int,(marker[1],marker[3],marker[4],marker[5]))
 queued=re.findall(rf'\[shell.document.viewport\] target=15 task=15 surface=300 sequence={sequence} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) enqueued=1',trace);accepted=re.findall(rf'\[browser.viewport\] target=15 task=15 sequence={sequence} w=(\d+) h=(\d+) origin=held_shell',trace)
 if len(queued)!=1 or not accepted:raise ValueError('missing exact held-shell clock viewport')
 x,y,w,h=map(int,queued[0])
 if(w,h)!=tuple(map(int,accepted[-1])):raise ValueError('viewport geometry differs')
 frame=calendar_frame(w,h,expected,contract);scale=frame['scale']
 if not frame['full_calendar']:raise ValueError('clock tile too small for complete calendar qualification')
 if(length,cols,rows,marker[2])!=(len(frame['data']),frame['cols'],frame['rows'],frame['hash']):raise ValueError('whole calendar frame/measure/hash differs')
 applied=rf'\[browser.clock.applied\] bytes={length} hash={frame["hash"]} rows={rows} viewport={sequence} sequence=([1-9]\d*) positive_ack=1'
 if not re.search(applied,trace[marker.end():]):raise ValueError('missing positive exact clock Applied after layout')
 checked_rows=[]
 for row,line in enumerate(frame['lines']):
  py=y+contract['top']+row*contract['line']*scale
  if py+7*scale>y+h:raise ValueError('clock status clipped')
  checked_rows.append(dict(row=row,text=line.decode(),x=x+contract['inset'],y=py))
 return dict(tile=dict(x=x,y=y,w=w,h=h),viewport=sequence,scale=scale,columns=cols,rows=checked_rows,utc_text=frame['utc_text'],frame_hash=frame['hash'],frame_bytes=length)

def verify_clock_pixels(trace,ppm,expected,contract,resolution=None):
 try:
  geometry=clock_geometry(trace,expected,contract);width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1);tile=geometry['tile'];scale=geometry['scale'];color=contract['color'].to_bytes(3,'big');checked=ink=mismatch=0;wanted=set()
  if resolution and(width,height)!=tuple(resolution):raise ValueError('wrong framebuffer resolution')
  if tile['x']<0 or tile['y']<50 or tile['x']+tile['w']>width or tile['y']+tile['h']>height:raise ValueError('clock viewport outside framebuffer')
  for row in geometry['rows']:
   for col in range(geometry['columns']):
    bitmap=glyph(contract['font'],row['text'][col])if col<len(row['text'])else(0,)*7
    for gy in range(contract['line']):
     for gx in range(contract['advance']):
      on=gy<7 and gx<5 and bool(bitmap[gy]&(1<<(4-gx)))
      if on:
       for dy in range(scale):
        for dx in range(scale):
         px=row['x']+(col*contract['advance']+gx)*scale+dx;py=row['y']+gy*scale+dy
         if py<tile['y']+tile['h']:wanted.add(py*width+px)
  # Inspect the entire owned Browser text body, including blank columns to
  # the right of the capped measure and all old rows below this snapshot.
  # No cursor masks: the native runner parks it outside the body first.
  for py in range(tile['y']+contract['top'],tile['y']+tile['h']):
   for px in range(tile['x']+contract['inset'],tile['x']+tile['w']-contract['inset']):
    index=py*width+px;on=index in wanted;actual=pixels[index*3:index*3+3]==color;mismatch+=actual!=on;checked+=1;ink+=on
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

def replay_evidence(root,iso,binding_path,out):
 """Read-only verification of terminal raw clock evidence in a fresh process."""
 from https_asset_gate import verify_parking
 root=Path(root);out=Path(out);contract=clock_contract(root);manifest=json.loads((out/'manifest.json').read_text());result=json.loads((out/'result.json').read_text());bound=frozen_binding(root,Path(iso),Path(binding_path))
 if manifest.get('error')is not None or result.get('ok')is not True:raise ValueError('failed native attempt remains diagnostic')
 require_clock_inputs(root,json.loads(Path(binding_path).read_text()),contract)
 if manifest['binding']!=bound or manifest['pixel_source_sha256']!=contract['source_sha256']or manifest['serial_offset_units']!=SERIAL_UNITS:raise ValueError('clock source/image/prefix binding differs')
 command=manifest['command'];resolution=tuple(map(int,manifest['resolution'].split('x')));mode=manifest['mode']
 if command[command.index('-cdrom')+1]!=str(Path(iso).resolve())or command.count('VGA,xres='+manifest['resolution'].replace('x',',yres='))!=1 or command.count(manifest['model']+',netdev=peer,mac=52:54:00:12:34:56')!=1 or not command_profile(command,mode)['ok']:raise ValueError('actual clock device/image/RTC profile differs')
 captures=result['pixels'];names=['clock-first','clock-cleared','clock-second']if mode!='close'else[]
 if [f['name']for f in captures]!=names:raise ValueError('missing/reordered clock capture phases')
 required={'serial.log','wire.pcap','endpoint-exchanges.json','actions.json','seed.img.gz','nvme.img.gz'}|{f['screenshot']for f in captures}
 if mode=='close':required.add('clock-closed.ppm')
 if set(manifest['raw_sha256'])!=required:raise ValueError('clock raw closure differs')
 for name,digest in manifest['raw_sha256'].items():
  path=out/name
  if path.is_symlink()or path.resolve().parent!=out.resolve()or hashlib.sha256(path.read_bytes()).hexdigest()!=digest:raise ValueError('changed/unsafe raw clock evidence: '+name)
 for name,expected in [('seed.img.gz',manifest['input_disk_sha256']),('nvme.img.gz',manifest['final_disk_sha256'])]:
  if hashlib.sha256(gzip.decompress((out/name).read_bytes())).hexdigest()!=expected:raise ValueError('clock disk hash differs')
 trace=clock_serial_text(out/'serial.log');raw=(out/'wire.pcap').read_bytes();endpoints=json.loads((out/'endpoint-exchanges.json').read_text())['public_baseline'];actions=json.loads((out/'actions.json').read_text())
 if len(endpoints)!=1 or re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow|\[fault.kill\]',trace):raise ValueError('clock guest/endpoint failure')
 clock=inspect_clock(trace,mode,command,manifest['host_utc_start'],manifest['host_utc_end']);wire=tcp_capture(raw,endpoints,guest_ports=range(48001,65536),initiator='guest');owners=owner_joins(trace,raw,[(15,15)]);authority=nic_check(trace,expected=1,preparation=True,ethernet=True,network_config=True,routing=True);waiting=wait_check(trace)
 for check in(clock,wire,owners,authority,waiting):
  if not check['ok']:raise ValueError('clock replay join failed: '+str(check.get('errors')))
 for frame in frames(raw):
  if len(frame)>=42 and frame[12:14]==b'\x08\0'and frame[23]==17 and frame[26:30]==bytes([10,0,2,15]):
   at=14+(frame[14]&15)*4
   if int.from_bytes(frame[at+2:at+4],'big')==46079:raise ValueError('clock query emitted DNS')
 expected_states=([f'Ready({clock["joins"][0]["seconds"]})','Cancelled',f'Ready({clock["joins"][1]["seconds"]})']if mode=='normal'else['Unavailable','Cancelled','Unavailable'])if captures else[]
 checked=[];previous=0
 for frame,state in zip(captures,expected_states):
  prefix=frame_prefix(trace,frame)
  if frame['serial_end']<=previous or frame['expected']!=state or frame['screenshot']!=frame['name']+'.ppm':raise ValueError('clock capture reused/foreign phase')
  proof=verify_clock_pixels(prefix,(out/frame['screenshot']).read_bytes(),state,contract,resolution)
  if not proof['ok']:raise ValueError('clock raw pixels failed: '+str(proof['errors']))
  checked.append(dict(name=frame['name'],**proof));previous=frame['serial_end']
 parks=[a for a in actions if a.get('kind')=='park']
 if captures:verify_parking(trace,captures,parks,resolution)
 cleanup=None
 if mode=='close':
  requests=list(re.finditer(r'\[shell.app.close.request\] target=15 task=15 surface=300',trace))
  if len(requests)!=1:raise ValueError('missing/duplicate exact Browser Close request')
  cleanup=cleanup_receipts(trace,15,15,requests[0].start())
 return dict(ok=True,binding=bound,mode=mode,captures=len(checked),pixels=checked,clock=clock,wire=wire,owners=owners,authority=authority,self_wait=waiting,cleanup=cleanup,scope='Fresh-process replay of exact clock source/service/client snapshots, Gregorian/SexTime/Long Count whole-body foreground and erasure, parked real pointer, raw wire and terminal disks. Calendar formatting changes no clock trust or wire epoch.')

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',type=Path);p.add_argument('--binding',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--mode',choices=['normal','invalid-year','close'],default='normal');p.add_argument('--model',choices=['e1000','e1000e'],default='e1000e');p.add_argument('--resolution',choices=['1280x800','1024x768'],default='1280x800');p.add_argument('--check',action='store_true');a=p.parse_args()
 root=Path(__file__).resolve().parents[1];iso=a.iso.resolve(strict=True);out=a.out.resolve()
 if a.check:
  result=replay_evidence(root,iso,a.binding.resolve(strict=True),out);print(json.dumps(dict(ok=True,mode=result['mode'],captures=result['captures'])));return
 if a.source_disk is None:p.error('native clock run requires --source-disk')
 seed=a.source_disk.resolve(strict=True);out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 binding=frozen_binding(root,iso,a.binding.resolve(strict=True));contract=clock_contract(root)
 require_clock_inputs(root,json.loads(a.binding.read_text()),contract)
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
 manifest=dict(binding=binding,command=command,qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),mode=a.mode,model=a.model,resolution=a.resolution,rtc_profile=profile,input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),pixel_source_sha256=contract['source_sha256'],serial_offset_units=SERIAL_UNITS);host_start=time.time();manifest['host_utc_start']=host_start
 (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic();pixels=[];actions=[];failure=None;cleanup=None;park_phase=0
 def text():return clock_serial_text(serial)if serial.exists()else''
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
  nonlocal park_phase
  from launcher_workflow_gate import pointer_park
  pointer_park(q,tuple(map(int,a.resolution.split('x'))),text,wait,actions.append,park_phase);park_phase+=1;actions[-1].update(capture=name,serial_end=len(text()))
  end=time.monotonic()+20;path=out/(name+'.ppm');result=dict(ok=False,errors=['no frame'])
  while time.monotonic()<end:
   health();q.cmd('screendump',{'filename':str(path)});snapshot=text();result=verify_clock_pixels(snapshot,path.read_bytes(),state,contract,tuple(map(int,a.resolution.split('x'))))
   if result['ok']:break
   time.sleep(.1)
  result.update(name=name,screenshot=path.name,screenshot_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),source_sha256=contract['source_sha256'],serial_end=len(snapshot),serial_sha256=hashlib.sha256(snapshot.encode()).hexdigest(),requested_anchor=None);pixels.append(result);(out/(name+'-pixels.json')).write_text(json.dumps(result,indent=2)+'\n')
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
 manifest['host_utc_end']=host_end;raw=pcap.read_bytes()if pcap.exists()else b'';trace=text();wire=tcp_capture(raw,rows,guest_ports=range(48001,65536),initiator='guest')if raw else dict(ok=False,errors=['missing actual wire capture']);owners=owner_joins(trace,raw,[(15,15)]);clock=inspect_clock(trace,a.mode,command,host_start,host_end);authority=nic_check(trace,expected=1,preparation=True,ethernet=True,network_config=True,routing=True);waiting=wait_check(trace)
 try:
  for frame in frames(raw):
   if len(frame)>=42 and frame[12:14]==b'\x08\0'and frame[23]==17 and frame[26:30]==bytes([10,0,2,15]):
    at=14+(frame[14]&15)*4
    if int.from_bytes(frame[at+2:at+4],'big')==46079:raise ValueError('clock intent emitted DNS')
 except(ValueError,IndexError)as exc:failure=failure or str(exc)
 if len(rows)!=1:failure=failure or'wrong sole public baseline flow'
 (out/'seed.img.gz').write_bytes(gzip.compress(seed.read_bytes(),mtime=0));terminal=disk.read_bytes();(out/'nvme.img.gz').write_bytes(gzip.compress(terminal,mtime=0));manifest['final_disk_sha256']=hashlib.sha256(terminal).hexdigest();manifest['error']=failure
 (out/'actions.json').write_text(json.dumps(actions,indent=2)+'\n')
 (out/'endpoint-exchanges.json').write_text(json.dumps(dict(public_baseline=rows),indent=2)+'\n');result=dict(ok=failure is None and not server_errors and all(v['ok']for v in(wire,owners,clock,authority,waiting)),error=failure,server_errors=server_errors,mode=a.mode,binding=binding,clock=clock,public_wire=wire,owner_packet_joins=owners,authority=authority,self_wait=waiting,pixels=pixels,cleanup=cleanup,actions=actions,scope='Actual T/Q/T public UTC snapshots through exact typed BOOT source, sequence-bound service/client and independent Python UTC full glyphs; source seconds are bracketed by observed host time under explicit trusted RTC config. Invalid1999 profile rejects reads without tick fallback. Close reports actualpending0or1 and exact cache retirement before five-role cleanup. No native rollback injection, authenticated/NTP/physical time, persistent cross-reboot floor or certificate validation proof.')
 names=['serial.log','wire.pcap','endpoint-exchanges.json','actions.json','seed.img.gz','nvme.img.gz']+[f['screenshot']for f in pixels]+(['clock-closed.ppm']if(out/'clock-closed.ppm').exists()else[]);manifest['raw_sha256']={name:hashlib.sha256((out/name).read_bytes()).hexdigest()for name in names if(out/name).is_file()};(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
 (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(dict(error=failure,clock_errors=clock['errors'],server_errors=server_errors)))
 result['replay']=replay_evidence(root,iso,a.binding.resolve(strict=True),out);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 print(json.dumps(dict(ok=True,mode=a.mode,closed_pending=clock['closed_pending'])))

if __name__=='__main__':main()
