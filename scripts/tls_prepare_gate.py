#!/usr/bin/env python3
"""Real L/Q private TLS preparation, prerequisite failures and terminal cleanup.

No TLS records leave the guest. Metadata and status pixels prove only this
bounded foundation; they do not prove an authenticated or encrypted connection.
"""
import argparse,hashlib,json,math,os,re,shutil,socket,subprocess,threading,time
from pathlib import Path
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from network_contention_gate import frozen_binding,owner_joins,cleanup_receipts,MAX_PCAP
from entropy_seed_gate import ECHO,command_profile as entropy_profile
from certificate_time_gate import RTC_NORMAL,RTC_INVALID,command_profile as clock_profile
from http_content_gate import load_pixel_contract
from check_tcp_capture import inspect as tcp_capture,frames
from check_nic_command import inspect as nic_check
from check_self_wait import inspect as wait_check
from check_desktop_pixels import glyph
from check_disk_persistence import ppm_rgb
from check_nonunix_target import inspect as target_check

MODES=('ready','close','unsupported','invalid-year')
LABELS={'Preparing':b'preparing','HelloPending':b'hello pending; no peer','Unavailable':b'unavailable','Cancelled':b'cancelled','Rejected':b'rejected','TimedOut':b'timed out'}
SCOPE='Actual L/Q private bounded TLS preparation, exact BOOT entropy/UTC/trust prerequisite receipts and controlled public certificate/name/time/signature fixture self-check, source-bound complete status glyphs and sole public startup Echo flow. private_bytes includes Pipe metadata; erased_payload_bytes counts only owned future plus8192 queue-array bytes. No seed/output bytes or derivatives captured; no TLS wire, handshake, peer authentication, public-CA trust, randomness quality, hostile containment, SMP or physical proof.'

def command_profile(command,mode):
 errors=[]
 if mode not in MODES:errors.append('unknown TLS preparation mode')
 errors.extend(entropy_profile(command,'unsupported'if mode=='unsupported'else'ready')['errors'])
 errors.extend(clock_profile(command,'invalid-year'if mode=='invalid-year'else'normal')['errors'])
 return dict(ok=not errors,errors=errors,scope='Exact explicit CPU/UTC emulator profiles; neither source authenticity nor runtime authority proof.')

def tls_contract(root):
 contract=load_pixel_contract(root);app=(root/'apps/kaleidoscope/src/main.rs').read_text();view=(root/'apps/kaleidoscope/src/browser_view.rs').read_text();probe=(root/'apps/kaleidoscope/src/tls_probe.rs').read_text();backend=(root/'crates/sex-tls/src/lib.rs').read_text()
 if 'pub fn with_tls('not in view or 'browser_view::with_tls(text,'not in app or 'start.checked_add(64)'not in view:raise ValueError('source-bound whole64-byte TLS extent changed')
 for literal in ('b"\\n\\nTLS preparation: "','b"\\nL prepare  Q clear"'):
  if literal not in view:raise ValueError('source-bound TLS chrome changed')
 for label in LABELS.values():
  if ('b"'+label.decode()+'"')not in probe:raise ValueError('source-bound TLS label changed')
 for name,expected in [('RECORD_READ',4096),('RECORD_WRITE',2048),('QUEUE_BYTES',4096),('MAX_TASK_BYTES',32768)]:
  match=re.search(r'pub const '+name+r': usize = ([0-9_]+);',backend)
  if not match or int(match[1].replace('_',''))!=expected:raise ValueError('source-bound TLS capacity changed: '+name)
 names=['apps/kaleidoscope/src/tls_probe.rs','apps/kaleidoscope/src/trust_probe.rs','apps/kaleidoscope/src/entropy_probe.rs','apps/kaleidoscope/src/clock_probe.rs','crates/sex-pdx/src/entropy_seed.rs','crates/sex-pdx/src/certificate_time.rs','kernel/src/entropy_source.rs','kernel/src/clock_source.rs','servers/sexnet/src/entropy.rs','servers/sexnet/src/certificate_time.rs','crates/sex-pdx/src/trust_source.rs','crates/sex-pdx/src/trust_policy.rs','kernel/src/trust_source.rs','servers/sexnet/src/trust_policy.rs','crates/sex-trust-config/Cargo.toml','crates/sex-trust-config/src/lib.rs','crates/sex-tls/Cargo.toml','crates/sex-tls/src/lib.rs','crates/sex-tls/src/verifier.rs','crates/sex-tls/src/probe_fixture.rs','x86_64-sex.json','rust-toolchain.toml','Cargo.lock','apps/kaleidoscope/Cargo.lock']
 for name in names:
  path=root/name
  if path.is_file():contract['source_sha256'][name]=hashlib.sha256(path.read_bytes()).hexdigest()
  elif name not in('apps/kaleidoscope/Cargo.lock',):raise ValueError('missing source-bound TLS prerequisite: '+name)
 policy=(root/'crates/sex-trust-config/src/lib.rs').read_text();source=(root/'crates/sex-pdx/src/trust_source.rs').read_text()
 for name,expected in [('SLOT',62),('SYS',74)]:
  match=re.search(r'pub const '+name+r'\s*:\s*u64\s*=\s*([0-9_]+);',source)
  if not match or int(match[1].replace('_',''))!=expected:raise ValueError('source-bound trust syscall/slot changed: '+name)
 for name in ('PROFILE','VERSION'):
  if not re.search(r'pub const '+name+r'\s*:\s*u32\s*=\s*1;',policy):raise ValueError('source-bound controlled trust profile changed')
 if not re.search(r'EXPECTED_NAME\s*:\s*&str\s*=\s*"probe\.sexos\.test";',policy):raise ValueError('source-bound expected controlled name changed')
 root_der=re.search(r'pub const ROOT_DER[^=]*=\s*&\[(.*?)\];',policy,re.S)
 if not root_der or len(re.findall(r'0x[0-9a-fA-F]{2}',root_der[1]))!=376:raise ValueError('source-bound controlled root extent changed')
 return contract

def tls_geometry(trace,expected,contract):
 layouts=list(re.finditer(r'\[browser.tls.layout\] base=(\d+) bytes=64 viewport=(\d+) state=(\w+) pixels=unconfirmed',trace))
 if not layouts or layouts[-1][3]!=expected:raise ValueError('wrong latest TLS layout')
 marker=layouts[-1];base,sequence=int(marker[1]),int(marker[2]);previews=re.findall(r'\[browser.preview.layout\] bytes=(\d+) newline=1 cached_or_sample=1',trace)
 if not previews:raise ValueError('missing actual preview extent')
 length=int(previews[-1]);prefix=contract['prefix']
 if base!=length+258 or not len(prefix)<length<=192:raise ValueError('TLS extent differs from complete source-bound overview')
 queued=re.findall(rf'\[shell.document.viewport\] target=15 task=15 surface=300 sequence={sequence} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) enqueued=1',trace);accepted=re.findall(rf'\[browser.viewport\] target=15 task=15 sequence={sequence} w=(\d+) h=(\d+) origin=held_shell',trace)
 if len(queued)!=1 or not accepted:raise ValueError('missing exact held-shell TLS viewport')
 x,y,w,h=map(int,queued[0])
 if(w,h)!=tuple(map(int,accepted[-1])):raise ValueError('viewport geometry differs')
 if not re.search(rf'\[browser.view.applied\] bytes={base+64} http=0 body_bytes=0 pixels=unconfirmed',trace[marker.end():]):raise ValueError('missing positive complete Applied after TLS layout')
 preceding=prefix+b'?'*(length-len(prefix))+contract['echo']['Verified'].ljust(64,b' ')+(contract['dns_prefix']+contract['dns']['Idle']+contract['dns_footer']).ljust(64,b' ')+(contract['http_prefix']+contract['http']['Idle']+contract['http_footer']).ljust(64,b' ')+b'\n\n'+b' '*64
 if len(preceding)!=base:raise ValueError('source overview changed')
 status=(b'\n\nTLS preparation: '+LABELS[expected]+b'\nL prepare  Q clear').ljust(64,b' ')
 if len(status)!=64:raise ValueError('TLS status overflow')
 data=preceding+status;scale=2 if w>=contract['scale_threshold']else 1;cols=min(80,max(1,(w-2*contract['inset'])//(contract['advance']*scale)));starts=[0];column=0
 for index,byte in enumerate(data):
  if byte==10:starts.append(index+1);column=0
  else:
   column+=1
   if column>=cols and index+1<len(data):starts.append(index+1);column=0
 if len(starts)>contract['max_rows']:raise ValueError('TLS exceeds renderer row table')
 rows=[]
 for row,start in enumerate(starts):
  if start<=base:continue
  end=starts[row+1]if row+1<len(starts)else len(data);py=y+contract['top']+row*contract['line']*scale
  if py+contract['line']*scale>y+h:raise ValueError('TLS status clipped')
  rows.append(dict(row=row,text=data[start:end].rstrip(b'\n').decode(),x=x+contract['inset'],y=py))
 if not any('TLS preparation:'in row['text']for row in rows):raise ValueError('full TLS label not visible')
 return dict(tile=dict(x=x,y=y,w=w,h=h),base=base,viewport=sequence,scale=scale,columns=cols,rows=rows,status_text=LABELS[expected].decode())

def verify_tls_pixels(trace,ppm,expected,contract,resolution=None):
 try:
  geometry=tls_geometry(trace,expected,contract);width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1);tile=geometry['tile'];scale=geometry['scale'];color=contract['color'].to_bytes(3,'big');checked=ink=mismatch=0
  if resolution and(width,height)!=tuple(resolution):raise ValueError('wrong framebuffer resolution')
  if tile['x']<0 or tile['y']<50 or tile['x']+tile['w']>width or tile['y']+tile['h']>height:raise ValueError('TLS viewport outside framebuffer')
  for row in geometry['rows']:
   for col in range(geometry['columns']):
    bitmap=glyph(contract['font'],row['text'][col])if col<len(row['text'])else(0,)*7
    for gy in range(contract['line']):
     for gx in range(contract['advance']):
      on=gy<7 and gx<5 and bool(bitmap[gy]&(1<<(4-gx)))
      for dy in range(scale):
       for dx in range(scale):
        px=row['x']+(col*contract['advance']+gx)*scale+dx;py=row['y']+gy*scale+dy;actual=pixels[(py*width+px)*3:(py*width+px)*3+3]==color;mismatch+=actual!=on;checked+=1;ink+=on
  errors=[]if not mismatch else[f'full TLS glyph/padding/erasure mismatches={mismatch}']
  if not ink:errors.append('empty expected TLS ink')
  return dict(ok=not errors,errors=errors,expected=expected,geometry=geometry,checked_pixels=checked,expected_ink_pixels=ink)
 except(ValueError,KeyError,IndexError,UnicodeError,OverflowError)as exc:return dict(ok=False,errors=[str(exc)],expected=expected)


def inspect_tls(trace,mode,command,host_start,host_end):
 errors=list(command_profile(command,mode)['errors']);joins=[];trust_joins=[];closed_pending=None
 try:
  for marker in ('[boot.entropy.source] holder=13 task=13 slot=60 consumer=15 consumer_task=15 source=rdseed attempts=8 runtime_mint=0','[boot.clock.source] holder=13 task=13 slot=61 consumer=15 consumer_task=15 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0','[browser.entropy.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[browser.clock.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[sexnet.clock.source] held=1 source=q35_cmos_utc no_ticks_fallback=1 precision=1','[boot.trust.policy] holder=13 task=13 slot=62 consumer=15 consumer_task=15 profile=1 root_bytes=376 runtime_mint=0','[browser.trust.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1'):
   assert trace.count(marker)==1,'missing/duplicate exact BOOT/source denial contract'
  for marker in ('[boot.entropy.source]','[boot.clock.source]','[boot.trust.policy]','[browser.trust.source.denied]'):assert trace.count(marker)==1,'foreign BOOT source grant'
  assert trace.count(f'[sexnet.entropy.source] held=1 available={0 if mode=="unsupported"else 1} source=rdseed fallback=0 broker=private_seed32')==1,'wrong actual source availability/fallback'
  private_lines=[line for line in trace.splitlines()if re.search(r'\[(?:browser\.tls|browser\.entropy|entropy\.|sexnet\.entropy|boot\.entropy)',line)]
  assert not any(re.search(r'\b(?:hash|fingerprint|word|value|hex|seed_bytes|seed_value|sample_value|data|secret|random)=',line)for line in private_lines),'secret-bearing private metadata field'
  samples=list(re.finditer(r'\[entropy.source.sample\] pd=13 task=13 slot=60 source=rdseed attempts=(\d+) count=(\d+) private=1',trace))
  assert len(samples)==trace.count('[entropy.source.sample]'),'foreign/malformed entropy metadata'
  assert all(1<=int(m[1])<=8 for m in samples),'unbounded source attempts'
  counts=[int(m[2])for m in samples];assert all(a<b for a,b in zip(counts,counts[1:])),'sample ordinal regressed'
  admitted=list(re.finditer(r'\[entropy.seed.admitted\] owner=15 task=15 ticket=(\d+) bytes=32 source=held_boot_grant',trace));releases=list(re.finditer(r'\[entropy.seed.released\] owner=15 task=15 ticket=(\d+) service_payload=erased receipt=retained',trace))
  for marker,values in (('[entropy.seed.admitted]',admitted),('[entropy.seed.released]',releases)):assert len(values)==trace.count(marker),'foreign/malformed private seed receipt'
  sources=list(re.finditer(r'\[clock.source.snapshot\] pd=13 task=13 slot=61 seconds=(\d+) attempts=(\d+) precision=1 profile=q35_cmos_utc ordinal=(\d+)',trace));services=list(re.finditer(r'\[certificate.time.snapshot\] owner=15 task=15 seq=(\d+) seconds=(\d+) source=q35_cmos_utc precision=1 receipt=retained',trace))
  for marker,values in (('[clock.source.snapshot]',sources),('[certificate.time.snapshot]',services)):assert len(values)==trace.count(marker),'foreign/malformed UTC receipt'
  assert len(sources)==len(services),'source/service UTC count differs'
  assert [int(m[3])for m in sources]==list(range(1,len(sources)+1))and[int(m[1])for m in services]==list(range(1,len(services)+1)),'UTC ordinal/query sequence reused'
  for source,service in zip(sources,services):
   assert 1<=int(source[2])<=4 and int(source[1])==int(service[2])and source.start()<service.start(),'wrong UTC attempt bound/join/order'
   assert math.floor(host_start)-1<=int(source[1])<=math.ceil(host_end)+1,'UTC outside actual trusted host bracket'
  intents=list(re.finditer(r'\[browser.tls.intent\] accepted=1 explicit=1 network=0',trace));expected=1 if mode=='close'else 2
  assert len(intents)==trace.count('[browser.tls.intent]')==expected,'wrong explicit L count'
  policies=list(re.finditer(r'\[trust.policy.snapshot\] owner=15 task=15 seq=(\d+) profile=1 root_bytes=376 origin=held_boot_grant',trace))
  assert len(policies)==trace.count('[trust.policy.snapshot]')==expected,'missing/foreign/duplicate held trust query'
  assert [int(policy[1])for policy in policies]==list(range(1,expected+1)),'trust query sequence reused or skipped'
  policy_ready=list(re.finditer(r'\[browser.trust.view\] state=Ready\(1\) immutable_boot_profile=1 transport=0',trace))
  all_policy_views=list(re.finditer(r'\[browser.trust.view\] state=(?:Idle|Pending|Ready\(1\)|Unavailable|Cancelled|Uncertain) immutable_boot_profile=1 transport=0',trace))
  assert len(all_policy_views)==trace.count('[browser.trust.view]')and len(policy_ready)==expected,'unknown policy view or missing exact Readyprofile'
  self_checks=list(re.finditer(r'\[browser.trust.self_check\] profile=1 passed=1 fixture=public_controlled_profile authenticated_peer=0',trace))
  assert len(self_checks)==trace.count('[browser.trust.self_check]'),'wrong/foreign/failed/public-peer fixture receipt'
  for index,(intent,policy,ready)in enumerate(zip(intents,policies,policy_ready)):
   end=intents[index+1].start()if index+1<len(intents)else len(trace)
   assert intent.start()<policy.start()<ready.start()<end,'trust profile not correlated to this explicit L'
   for provider in admitted+sources:
    if intent.start()<provider.start()<end:assert ready.start()<provider.start(),'seed/time source started before BOOT trust profile attestation'
   trust_joins.append(dict(sequence=int(policy[1]),profile=1,root_bytes=376,policy_offset=policy.start(),ready_offset=ready.start()))
  views=list(re.finditer(r'\[browser.tls.view\] state=(Preparing|HelloPending|Unavailable|Cancelled|Rejected|TimedOut) authenticated=0 network=0',trace))
  assert len(views)==trace.count('[browser.tls.view]'),'unknown state/authentication/network claim'
  prepared=list(re.finditer(r'\[browser.tls.prepared\] task_bytes=(\d+) private_bytes=(\d+) read=4096 write=2048 tx_bytes=(\d+) utc_snapshot=(\d+) authenticated=0 network=0',trace));hellos=[v for v in views if v[1]=='HelloPending']
  assert len(prepared)==len(self_checks),'TLS preparation missing corresponding controlled verifier self-check'
  assert len(prepared)==trace.count('[browser.tls.prepared]')==len(hellos),'malformed/extra preparation or HelloPending without preparation'
  erased=list(re.finditer(r'\[browser.tls.erased\] bytes=(\d+) owned_storage=1',trace));assert len(erased)==trace.count('[browser.tls.erased]'),'wrong storage erasure receipt'
  assert all(0<=int(m[1])<=40960 for m in erased),'erased storage outside bounded private arena'
  for index,(prep,hello)in enumerate(zip(prepared,hellos)):
   task,private,tx,utc=map(int,prep.groups());assert 0<task<=32768 and private==task+8216 and private<49152 and 0<tx<=4096,'TLS private task/buffer/output bound differs'
   assert index<len(releases)and index<len(services),'TLS prepared without fresh retained prerequisites'
   policy=policies[index];check=self_checks[index]
   assert policy.start()<policy_ready[index].start()<check.start()<prep.start(),'self-check/preparation preceded this exact held trust profile'
   end=intents[index+1].start()if index+1<len(intents)else len(trace)
   assert check.start()<prep.start()<end,'self-check/preparation escaped this explicit L'
   release=releases[index];ticket=int(release[1]);admission=next((m for m in admitted if int(m[1])==ticket),None)
   collected=re.search(rf'\[entropy.seed.progress\] owner=Some\(Identity \{{ pd: 15, task: 15 \}}\) ticket={ticket} state=Some\(Ready\) produced=32 consumed=0 failure=0 private=1',trace)
   assert admission and collected and intents[index].start()<admission.start()<collected.start()<release.start()<prep.start()<hello.start(),'HelloPending before exact produced32/positive seed RELEASE'
   assert release.start()<check.start()and services[index].start()<check.start(),'controlled verifier self-check preceded fresh seed/time prerequisites'
   assert services[index].start()<prep.start()and int(services[index][2])==utc,'prepared UTC differs from fresh held snapshot'
   joins.append(dict(trust_sequence=int(policy[1]),trust_profile=1,self_check_offset=check.start(),ticket=ticket,utc_sequence=int(services[index][1]),utc_snapshot=utc,task_bytes=task,private_bytes=private,tx_bytes=tx,prepared_offset=prep.start(),hello_offset=hello.start(),release_offset=release.start()))
  assert '[certificate.time.rollback]'not in trace,'unexpected native rollback'
  if mode=='ready':
   assert len(joins)==len(admitted)==len(releases)==len(services)==2,'wrong fresh prerequisite/Hello count'
   assert [int(m[1])for m in admitted]==[1,2]and[int(m[1])for m in releases]==[1,2],'private seed ticket reused'
  elif mode=='unsupported':
   assert not samples and not admitted and not releases and not prepared,'unsupported CPU produced/admitted/prepared private TLS'
  elif mode=='invalid-year':assert not sources and not services and not prepared,'invalid UTC prepared TLS or produced positive time'
  else:
   closed=re.search(r'\[browser.tls.closed\] pending=([01]) terminal=1 owned_storage=erased',trace);assert closed and trace.count('[browser.tls.closed]')==1,'missing exact terminal TLS Close'
   closed_pending=int(closed[1]);assert closed_pending==1,'Close did not observe allocated pending TLSfuture'
   assert len(prepared)==len(joins)==1 and len(admitted)==len(releases)==len(services)==1 and hellos[0].start()<closed.start(),'Close did not follow one fully prepared allocated TLSfuture'
   ack=re.search(r'\[net.app.cleanup.ack\] target=15 task=15 client_leases=0 shared_cache=retained ok=1',trace)
   assert ack and closed.start()<ack.start(),'Close did not precede exact cleanup ACK'
   for marker in ('[entropy.seed.retired] target=15 task=15 before_cleanup_ack=1 private_payload=erased','[certificate.time.retired] target=15 task=15 before_cleanup_ack=1 high_water_retained=1','[trust.policy.retired] target=15 task=15 before_cleanup_ack=1'):
    assert marker in trace and closed.start()<trace.index(marker)<ack.start(),'missing prerequisite retirement before cleanup ACK'
   trust_closed=re.search(r'\[browser.trust.closed\] target=15 task=15 pending=0 terminal=1',trace);assert trust_closed and trace.count('[browser.trust.closed]')==1 and closed.start()<trust_closed.start()<ack.start(),'allocated Close did not terminally retire completed policy client'
   assert trace.count('[trust.policy.retired]')==1,'foreign/duplicate trust retirement'
   assert any(closed.start()>=e.start()for e in erased),'Close missing owned storage erasure'
   if joins:assert any(int(e[1])==joins[0]['task_bytes']+8192 and prepared[0].start()<e.start()<closed.start()for e in erased),'completed Close missing exact owned future+queue erasure'
  if mode!='close':
   cancelled=[v for v in views if v[1]=='Cancelled'];assert len(cancelled)==1,'Q did not clear TLS exactly once'
   assert any(int(e[1])==joins[0]['task_bytes']+8192 and hellos[0].start()<e.start()<cancelled[0].start()for e in erased)if mode=='ready'else bool(erased),'Q missing exact owned future+queue erasure'
   if mode=='ready':joins[0]['erased_payload_bytes']=joins[0]['task_bytes']+8192
   assert cancelled[0].start()<intents[1].start(),'fresh retry preceded Q'
   if mode in('unsupported','invalid-year'):assert len([v for v in views if v[1]=='Unavailable'])==2,'missing both explicit prerequisite failures'
  assert len([v for v in views if v[1]=='Preparing'])==expected,'explicit preparations not observed'
 except(AssertionError,ValueError,IndexError,AttributeError)as exc:errors.append(str(exc))
 return dict(ok=not errors,errors=errors,joins=joins,trust_policy=dict(profile=1,root_bytes=376,queries=trust_joins,self_checks=len(locals().get('self_checks',[])),scope='Actual source-bound public controlled-profile verifier fixture only; no live peer or public CA claim.'),closed_pending=closed_pending,host_utc_bracket=dict(start=host_start,end=host_end,precision_slack_seconds=1),scope=SCOPE)

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--mode',choices=list(MODES),default='ready');p.add_argument('--model',choices=['e1000','e1000e'],default='e1000e');p.add_argument('--resolution',choices=['1280x800','1024x768'],default='1280x800');a=p.parse_args()
 root=Path(__file__).resolve().parents[1];iso=a.iso.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 binding=frozen_binding(root,iso,a.binding.resolve(strict=True));contract=tls_contract(root);target=target_check(root)
 if not target['ok']:raise RuntimeError(str(target))
 if json.loads(a.binding.read_text())['runner_sha256'].get('scripts/tls_prepare_gate.py')!=hashlib.sha256(Path(__file__).read_bytes()).hexdigest():raise RuntimeError('TLS gate absent from frozen cohort')
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
    if len(data)>1025:raise RuntimeError('unexpected TLS preparation network payload')
   if data!=ECHO:raise RuntimeError('wrong public baseline Echo bytes')
   conn.sendall(ECHO);conn.shutdown(socket.SHUT_WR);conn.close();conn=None;rows.append(dict(payload_hex=ECHO.hex(),response_hex=ECHO.hex()))
   while not stop.is_set():
    try:extra,_=listener.accept();extra.close();raise RuntimeError('L/Q opened extra TCP')
    except socket.timeout:continue
  except Exception as exc:
   if not stop.is_set():server_errors.append(repr(exc))
  finally:
   if conn:conn.close()
 thread=threading.Thread(target=peer,daemon=True);thread.start();disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-tls-{os.getpid()}.sock';rtc=RTC_INVALID if a.mode=='invalid-year'else RTC_NORMAL;cpu='max,+pku,-rdseed,-rdrand'if a.mode=='unsupported'else'max,+pku,+rdseed'
 command=['qemu-system-x86_64','-M','q35','-cpu',cpu,'-rtc',rtc,'-m','512M','-vga','none','-device','VGA,xres='+a.resolution.replace('x',',yres='),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}'];profile=command_profile(command,a.mode);assert profile['ok']
 manifest=dict(binding=binding,command=command,qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),mode=a.mode,model=a.model,resolution=a.resolution,platform_profile=profile,target=target,secret_capture=False,input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),pixel_source_sha256=contract['source_sha256']);host_start=time.time();manifest['host_utc_start']=host_start
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
   health();q.cmd('screendump',{'filename':str(path)});result=verify_tls_pixels(text(),path.read_bytes(),state,contract,tuple(map(int,a.resolution.split('x'))))
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
  start=len(text());q.key('l');wait(r'\[shell.browser.input\] code=38 down=1 slot=43 accepted=1',start);wait(r'\[browser.tls.view\] state=Preparing authenticated=0 network=0',start)
  if a.mode=='close':
   wait(r'\[browser.tls.prepared\] task_bytes=\d+ private_bytes=\d+ read=4096 write=2048 tx_bytes=\d+ utc_snapshot=\d+ authenticated=0 network=0',start);wait(r'\[browser.tls.view\] state=HelloPending authenticated=0 network=0',start);picture('HelloPending','tls-before-close');closed=len(text());q.key_sequence(['f11']);wait(r'\[browser.tls.closed\] pending=[01] terminal=1 owned_storage=erased',closed);wait(r'\[sext.app.pool.ready\] target=15 task=15 .*ok=1',closed);cleanup=cleanup_receipts(text(),15,15,closed);q.cmd('screendump',{'filename':str(out/'tls-closed.ppm')});actions.append(dict(action='actual_F11_allocated_HelloPending',pending_claim='Require observed pending1 for allocated no-peer TLSfuture; seed/time prerequisites already released.'))
  else:
   expected='HelloPending'if a.mode=='ready'else'Unavailable';wait(rf'\[browser.tls.view\] state={expected} authenticated=0 network=0',start);picture(expected,'tls-first')
   start=len(text());q.key('q');wait(r'\[shell.browser.input\] code=16 down=1 slot=43 accepted=1',start);wait(r'\[browser.tls.view\] state=Cancelled authenticated=0 network=0',start);picture('Cancelled','tls-cleared')
   start=len(text());q.key('l');wait(rf'\[browser.tls.view\] state={expected} authenticated=0 network=0',start);picture(expected,'tls-second');actions.append(dict(action='actual_L_Q_L',expected=expected))
 except Exception as exc:failure=repr(exc)
 finally:
  host_end=time.time();stop.set();listener.close();thread.join(timeout=1)
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
 manifest['host_utc_end']=host_end;(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');raw=pcap.read_bytes()if pcap.exists()else b'';trace=text();wire=tcp_capture(raw,rows,guest_ports=range(48001,65536),initiator='guest')if raw else dict(ok=False,errors=['missing actual wire capture']);owners=owner_joins(trace,raw,[(15,15)]);tls=inspect_tls(trace,a.mode,command,host_start,host_end);authority=nic_check(trace,expected=1,preparation=True,ethernet=True,network_config=True,routing=True);waiting=wait_check(trace)
 try:
  for frame in frames(raw):
   if len(frame)>=42 and frame[12:14]==b'\x08\0'and frame[23]==17 and frame[26:30]==bytes([10,0,2,15]):
    raise ValueError('TLS preparation emitted unexpected guest UDP; baseline permits only TCP Echo')
 except(ValueError,IndexError)as exc:failure=failure or str(exc)
 if len(rows)!=1:failure=failure or'wrong sole public baseline flow'
 (out/'endpoint-exchanges.json').write_text(json.dumps(dict(public_baseline=rows),indent=2)+'\n');result=dict(ok=failure is None and not server_errors and all(v['ok']for v in(wire,owners,tls,authority,waiting)),error=failure,server_errors=server_errors,mode=a.mode,binding=binding,tls=tls,public_wire=wire,owner_packet_joins=owners,authority=authority,self_wait=waiting,pixels=pixels,cleanup=cleanup,actions=actions,scope=SCOPE)
 (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(result))
 print(json.dumps(dict(ok=True,mode=a.mode,closed_pending=tls['closed_pending'])))

if __name__=='__main__':main()
