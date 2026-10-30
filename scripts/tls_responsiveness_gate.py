#!/usr/bin/env python3
"""Finite actual desktop progress during local TLS work and held HTTPS."""
import argparse,gzip,hashlib,json,math,os,re,shutil,struct,subprocess,time
from pathlib import Path
import https_document_gate as https
import tls_lifecycle_gate as lifecycle
import tls_prepare_gate as preparation
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from network_contention_gate import frozen_binding,cleanup_receipts,MAX_PCAP
from check_dns_capture import inspect as dns_capture
from check_nic_command import inspect as nic_check
from check_self_wait import inspect as wait_check
from check_nonunix_target import inspect as target_check
from check_quil_ui_pixels import verify as quil_pixels,view as quil_view
from check_desktop_pixels import font_from_source,glyph
from check_document_disk import recover,fnv
from check_disk_persistence import ppm_rgb

TOKEN=b'r97';AFTER=b'z97'
SCOPE='Actual two local TLS preparations followed by one controlled partial HTTPS response, live Quil input/full glyphs/durable saves and Browser terminal Close. Guest work ticks and host observation times are measurements, not a hard cryptographic/scheduler latency bound. Shared addresses supply no authority; BOOT/held routes, source receipts, peer SSL, ciphertext, pixels and durable bytes are separately checked. No public Internet/CA, proxy, seed/TLS-secret capture, negative transport EOF, physical/SMP or daily-driver claim.'

def source_contract(root):
 root=Path(root);contract=https.load_contract(root);source=(root/'servers/quil/src/main.rs').read_text();match=re.search(r'const QUIL_TEXT_INIT: &\[u8\] = b"((?:[^"\\]|\\.)*)";',source)
 if not match or 'static mut QUIL_CURSOR_POS: usize = 0;'not in source or 'static mut QUIL_DOC_ID: u64 = 2;'not in source:raise ValueError('source-bound initial Quil document/cursor differs')
 display=(root/'servers/sexdisplay/src/main.rs').read_text();model=(root/'crates/silkbar-model/src/lib.rs').read_text();app=(root/'apps/kaleidoscope/src/main.rs').read_text();probe=(root/'apps/kaleidoscope/src/tls_probe.rs').read_text()
 for literal in('const CLOCK_PREFIX:&[u8]=b"DEMO ";','const CLOCK_DIGIT_OFFSET:usize=30;','FONT_ASCII_5X7[glyph][y-cy]','let cx=cx+CLOCK_DIGIT_OFFSET;'):
  if literal not in display:raise ValueError('source-bound DEMO clock renderer changed')
 if 'for_desktop_width'not in model or 'let shift=edge.saturating_sub(width);'not in model or 'sequence={} app_turn={} begin_ticks={}'not in app or 'PrepWork::FixtureCheck'not in probe or 'PrepWork::ClientHello'not in probe:raise ValueError('source-bound clock layout/work producer changed')
 dimensions={}
 for name in('CHIP_X3','CHIP_Y','CLOCK_W','CHIP_H'):
  found=re.search(r'pub const '+name+r': usize = (\d+);',model)
  if not found:raise ValueError('source-bound DEMO geometry unavailable')
  dimensions[name]=int(found[1])
 digits=re.search(r'const FONT: \[\[u8; 7\]; 10\] = \[(.*?)\n\];',display,re.S)
 if not digits:raise ValueError('clock digit font unavailable')
 digit_font=[tuple(int(bit,0)for bit in re.findall(r'0b[01]+|0x[0-9a-fA-F]+|[0-9]+',row))for row in re.findall(r'\[([^\[\]]+)\]',digits[1])]
 if len(digit_font)!=10 or any(len(row)!=7 for row in digit_font):raise ValueError('clock digit font extent differs')
 color=re.search(r'pub const DEFAULT_THEME: Theme = Theme \{.*?text:\s*(0x[0-9A-Fa-f]+)',model,re.S)
 if not color:raise ValueError('clock theme unavailable')
 contract['clock']=dict(dimensions=dimensions,digits=digit_font,color=int(color[1],16)&0xffffff)
 contract['quil_initial']=bytes(match[1],'ascii').decode('unicode_escape').encode('ascii');contract['font']=font_from_source((root/'servers/sexdisplay/src/main.rs').read_text())
 for name in('scripts/tls_responsiveness_gate.py','servers/quil/src/main.rs','crates/silkbar-model/src/lib.rs','scripts/check_quil_ui_pixels.py','scripts/check_document_disk.py','scripts/tls_lifecycle_gate.py','scripts/desktop_focus.py','scripts/desktop_close_gate.py','servers/silk-shell/src/main.rs'):
  contract['source_sha256'][name]=hashlib.sha256((root/name).read_bytes()).hexdigest()
 return contract

def work_receipts(trace,prepared):
 """No missing, duplicate, malformed, reversed or same-turn heavy phase."""
 pattern=r'\[browser.tls.work\] kind=(FixtureCheck|ClientHello) sequence=(\d+) app_turn=(\d+) begin_ticks=(\d+) end_ticks=(\d+) elapsed_ticks=(\d+) clock_valid=([01]) payload=(\d+) seed_ready=([01])'
 matches=list(re.finditer(pattern,trace));assert len(matches)==trace.count('[browser.tls.work]')==6,'missing/foreign/malformed heavy-work receipt';assert[m[1]for m in matches]==['FixtureCheck','ClientHello']*3,'heavy work order/count differs';rows=[]
 checks=list(re.finditer(r'\[browser.trust.self_check\] profile=1 passed=1 fixture=public_controlled_profile authenticated_peer=0',trace));assert len(checks)==trace.count('[browser.trust.self_check]')==3,'missing/foreign actual fixture self-check'
 for index,m in enumerate(matches):
  kind=m[1];sequence,turn,begin,end,elapsed,valid,payload,seed=map(int,m.groups()[1:]);assert sequence>0 and turn>0 and valid==1 and end>=begin and elapsed==end-begin,'invalid/reversed/unmeasured work clock'
  if rows:assert sequence>rows[-1]['sequence']and turn>rows[-1]['app_turn']and begin>=rows[-1]['end_ticks'],'work reused probe/App turn or reversed elapsed clock'
  cycle=index//2;stats=prepared[cycle]
  if kind=='FixtureCheck':assert stats['seed_release_offset']<m.start() and stats['started_offset']<m.start() and payload==0 and seed==1 and m.end()<checks[cycle].start(),'fixture phase retained allocated TLS or lacked fresh owned seed'
  else:assert payload==stats['task_bytes']+8192+(12288 if stats['document']else 0)and seed==0 and checks[cycle].end()<m.start()<stats['offset'],'ClientHello lacks separate later turn, exact allocation or moved-seed erasure'
  rows.append(dict(kind=kind,sequence=sequence,app_turn=turn,begin_ticks=begin,end_ticks=end,elapsed_ticks=elapsed,payload=payload,seed_ready=seed,offset=m.start(),end=m.end()))
 return rows

def source_cycles(trace,contract,host_start,host_end):
 starts=list(re.finditer(r'\[browser.command.started\] kind=(TlsPrepare|TlsDns) accepted=1',trace));assert len(starts)==trace.count('[browser.command.started]')==3 and[m[1]for m in starts]==['TlsPrepare','TlsPrepare','TlsDns'],'lost/replayed explicit source command'
 policies=list(re.finditer(r'\[trust.policy.snapshot\] owner=15 task=15 seq=(\d+) profile=1 root_bytes=376 origin=held_boot_grant',trace));sources=list(re.finditer(r'\[clock.source.snapshot\] pd=13 task=13 slot=61 seconds=(\d+) attempts=([1-4]) precision=1 profile=q35_cmos_utc ordinal=(\d+)',trace));clocks=list(re.finditer(r'\[certificate.time.snapshot\] owner=15 task=15 seq=(\d+) seconds=(\d+) source=q35_cmos_utc precision=1 receipt=retained',trace));seeds=list(re.finditer(r'\[entropy.seed.admitted\] owner=15 task=15 ticket=(\d+) bytes=32 source=held_boot_grant',trace));prepared=list(re.finditer(r'\[browser.tls.prepared\] task_bytes=(\d+) private_bytes=(\d+) read=4096 write=2048 tx_bytes=(\d+) utc_snapshot=(\d+) authenticated=0 network=0',trace));rows=[];previous=0
 for prefix,values in(('[trust.policy.snapshot]',policies),('[clock.source.snapshot]',sources),('[certificate.time.snapshot]',clocks),('[entropy.seed.admitted]',seeds),('[browser.tls.prepared]',prepared)):assert len(values)==trace.count(prefix)==3,'missing/foreign source/preparation: '+prefix
 assert [int(m[1])for m in policies]==[int(m[1])for m in clocks]==[int(m[1])for m in seeds]==[int(m[3])for m in sources]==[1,2,3],'fresh source ordinal/sequence reused or skipped'
 for index,(start,policy,source,clock,seed,prep)in enumerate(zip(starts,policies,sources,clocks,seeds,prepared),1):
  task,private,initial,utc=map(int,prep.groups());document=index==3;assert 0<task<=32768 and private==task+contract['pipe_bytes']+(12288 if document else 0)and private<49152 and 0<initial<=4096,'bounded preparation allocation differs'
  assert utc==int(source[1])==int(clock[2])and previous<=utc and math.floor(host_start)-1<=utc<=math.ceil(host_end)+1,'UTC source/service/host bracket differs';previous=utc
  release=re.search(rf'\[entropy.seed.released\] owner=15 task=15 ticket={index} service_payload=erased receipt=retained',trace);produced=re.search(rf'\[entropy.seed.progress\] owner=Some\(Identity \{{ pd: 15, task: 15 \}}\) ticket={index} state=Some\(Ready\) produced=32 consumed=0 failure=0 private=1',trace)
  assert release and produced and start.start()<policy.start()<source.start()<clock.start()<prep.start()and policy.start()<seed.start()<produced.start()<release.start()<prep.start(),'source/positive seed cleanup ordering differs'
  rows.append(dict(cycle=index,document=document,task_bytes=task,private_bytes=private,initial_tls_bytes=initial,utc_snapshot=utc,offset=prep.start(),started_offset=start.start(),seed_release_offset=release.start()))
 assert trace.count('[entropy.seed.released]')==3,'extra seed release';return rows

def inspect_progress(trace,wire,dns,raw,host_start,host_end,contract):
 errors=[];joins={}
 try:
  assert wire['ok']and dns['ok'],'independent actual TLS/DNS evidence missing'
  for marker in('[boot.entropy.source] holder=13 task=13 slot=60 consumer=15 consumer_task=15 source=rdseed attempts=8 runtime_mint=0','[boot.clock.source] holder=13 task=13 slot=61 consumer=15 consumer_task=15 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0','[boot.trust.policy] holder=13 task=13 slot=62 consumer=15 consumer_task=15 profile=1 root_bytes=376 runtime_mint=0','[boot.network.tls] holder=13 task=13 consumer=15 consumer_task=15 profile=1 peer_port=46080 origin=immutable_boot_profile runtime_mint=0','[browser.trust.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[browser.entropy.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[browser.clock.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1'):assert trace.count(marker)==trace.count(marker.split(']')[0]+']')==1,'BOOT/source denial identity differs'
  assert not re.search(r'\b(?:keylog|traffic_secret|master_secret|seed_bytes|seed_value|secret)=',trace),'private source/TLS bytes exposed'
  cycles=source_cycles(trace,contract,host_start,host_end);work=work_receipts(trace,cycles);samples=lifecycle.resources(trace);drains=partial_drains(trace,raw)
  queued=list(re.finditer(r'\[browser.command.queued\] kind=(TlsPrepare|TlsDns) document=([01]) retained=1 accepted_replay=0',trace));assert len(queued)==trace.count('[browser.command.queued]')==3 and[m.groups()for m in queued]==[('TlsPrepare','0')]*2+[('TlsDns','1')],'unexpected/replayed queued command'
  for code,count in((38,2),(16,2),(48,1)):
   assert len(re.findall(rf'\[shell.browser.input\] code={code} down=1 slot=43 accepted=1',trace))==count,'actual held Shell input receipt count differs'
  doc_preps=list(re.finditer(r'\[browser.https.prepared\] task_bytes=(\d+) private_bytes=(\d+) tx_bytes=(\d+) utc_snapshot=(\d+) request_bytes=94 document_limit=12288 network=0',trace));assert len(doc_preps)==trace.count('[browser.https.prepared]')==1 and tuple(map(int,doc_preps[0].groups()))==(cycles[2]['task_bytes'],cycles[2]['private_bytes'],cycles[2]['initial_tls_bytes'],cycles[2]['utc_snapshot']),'document-specific prepared metadata differs'
  assert trace.count('[browser.tls.intent] accepted=1 explicit=1 network=0')==2 and trace.count('[browser.https.intent] accepted=1 explicit=1 public_internet=0')==1,'explicit input count differs'
  cancels=list(re.finditer(r'\[browser.tls.view\] state=Cancelled authenticated=0 network=0',trace));assert len(cancels)==2,'both local Q cancellations absent'
  for index,cancel in enumerate(cancels):
   erased=re.search(rf'\[browser.tls.erased\] bytes={cycles[index]["task_bytes"]+8192} owned_storage=1',trace[cycles[index]['offset']:cancel.start()]);assert erased and cancel.start()<queued[index+1].start(),'Q lost exact allocated local future/queues or retry ordering'
  resolved=https.http.resolution_joins(trace,raw,[(2,1)],dns);assert resolved['ok'],resolved['errors'];admitted=re.findall(r'\[browser.https.admitted\] ticket=(\d+) dns_generation=(\d+) seq=(\d+) task_bytes=(\d+) private_bytes=(\d+) profile=1 authority=held_route',trace);assert len(admitted)==trace.count('[browser.https.admitted]')==1 and tuple(map(int,admitted[0][:2]))==(2,1)and tuple(map(int,admitted[0][3:]))==(cycles[2]['task_bytes'],cycles[2]['private_bytes']),'foreign HTTPS held admission'
  auth=re.search(r'\[browser.https.authenticated\] ticket=2 profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1',trace);progress=list(re.finditer(r'\[browser.https.progress\] ticket=2 body_received=(\d+) authenticated=1 body_visible=0',trace));assert auth and doc_preps[0].start()<trace.index('[browser.https.admitted]')<auth.start() and trace.count('[browser.https.authenticated]')==1 and progress and progress[-1][1]=='1024'and auth.start()<progress[0].start(),'actual guest authenticated partial body absent'
  assert not re.search(r'\[browser.https.(?:body|released|rejected)\]|\[browser.reader.layout\]|\[browser.https.view\] state=Verified',trace),'partial body published or falsely completed'
  closed=re.search(r'\[browser.https.closed\] pending=1 terminal=1 owned_storage=erased body_visible=0',trace);assert closed and trace.count('[browser.https.closed]')==1,'allocated pending HTTPS Close absent';cleanup=cleanup_receipts(trace,15,15,closed.start());close_samples=[s for s in samples if s['reason']=='close'];assert len(close_samples)==1 and closed.start()<close_samples[0]['offset']<cleanup['ack_offset']and close_samples[0]['used']==close_samples[0]['payload']==0,'Close retains local allocation'
  saves=list(re.finditer(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=2',trace));expected=[TOKEN+contract['quil_initial'],TOKEN+AFTER+contract['quil_initial']];assert len(saves)==trace.count('[quil.persist.save.ok]')==2,'missing/foreign/duplicate Quil save'
  edits=[]
  for save,payload in zip(saves,expected):
   assert(int(save[1]),int(save[2],16))==(len(payload),fnv(payload)),'durableSave acknowledged wrong revision'
   edit=list(re.finditer(rf'\[quil.document.edit\] revision=(\d+) pending=([01]) bytes={len(payload)}(?:\s|$)',trace[:save.start()]));assert edit,'saved token lacks actual Quil edit receipt';edits.append(dict(revision=int(edit[-1][1]),offset=edit[-1].start()))
  assert edits[0]['revision']<edits[1]['revision'] and closed.start()<edits[1]['offset'],'post-Close edit revision did not advance'
  assert progress[-1].start()<saves[0].start()<closed.start()<cleanup['pool_offset']<saves[1].start(),'Quil did not save during held remote partial and again after Browser retirement'
  for marker in('[trust.policy.retired] target=15 task=15 before_cleanup_ack=1','[entropy.seed.retired] target=15 task=15 before_cleanup_ack=1 private_payload=erased','[certificate.time.retired] target=15 task=15 before_cleanup_ack=1 high_water_retained=1'):assert closed.start()<trace.index(marker)<cleanup['ack_offset'],'source retired after terminal ACK'
  assert not re.search(r'\[net\.(?:job|dns)\.egress\.submit\]|\[browser.tls.prepared\]',trace[cleanup['ack_offset']:]),'retired Browser emitted new owned work'
  joins=dict(cycles=cycles,work=work,resources=samples,drains=drains,resolution=resolved,cleanup=cleanup,saves=[dict(bytes=int(s[1]),hash=s[2],offset=s.start())for s in saves],edits=edits,first_Quil_edit_relative_to_remote_work=['before'if edits[0]['offset']<w['offset']else'after'for w in work[-2:]])
 except(AssertionError,ValueError,TypeError,KeyError,IndexError,AttributeError)as exc:errors.append(str(exc))
 return dict(ok=not errors,errors=errors,joins=joins,scope=SCOPE)

def partial_drains(trace,raw):
 flows,_=https.tls.reassemble(raw);actual=[f for f in flows if f['peer_port']==46080];assert len(actual)==1 and actual[0]['guest_port']==48002,'wrong captured partial duplex';captured=len(actual[0]['incoming'])
 admitted=list(re.finditer(r'\[net.job.duplex.admitted\] owner=15 task=15 ticket=2 kind=10 ring=1024 tx_ring=4096 max_wire=32768 peer_port=46080 trust_profile=1 authority=held_boot_route',trace));assert len(admitted)==trace.count('[net.job.duplex.admitted]')==1 and '[net.job.stream.admitted]'not in trace,'wrong/malformed/foreign duplex admission'
 matches=list(re.finditer(r'\[net.job.stream.drain\] owner=15 task=15 ticket=2 produced=(\d+) consumed=(\d+) buffered=(\d+) ring=1024 state_before=(Receiving|Complete) receipt=retained not_delivery=1',trace));assert len(matches)==trace.count('[net.job.stream.drain]')and matches,'missing/foreign/malformed shared READ sample';old=(0,0);rows=[];complete=False;closed=trace.index('[browser.https.closed]')
 for m in matches:
  produced,consumed,buffered=map(int,m.groups()[:3]);assert admitted[0].start()<m.start()<closed and 0<consumed<=produced<=min(32768,captured)and buffered==produced-consumed<=1024 and produced>=old[0]and consumed>old[1],'READ sample lacks exact admission/wire/cursor bound';assert not complete or m[4]=='Complete','READ state regressed';complete|=m[4]=='Complete';old=(produced,consumed);rows.append(dict(produced=produced,consumed=consumed,buffered=buffered,state=m[4],offset=m.start()))
 assert any(m[4]=='Receiving'for m in matches),'partial held response was never read while receiving';return dict(captured_incoming=captured,samples=rows)

def verify_quil_frame(trace,ppm,expected,contract,resolution):
 """One immutable serial snapshot per capture; incomplete paints retry."""
 try:
  errors=quil_pixels(trace,ppm,contract['font'],expected,target=9,resolution=resolution)
  if errors:return dict(ok=False,errors=errors)
  return dict(ok=True,errors=[],geometry=quil_view(trace))
 except ValueError as exc:return dict(ok=False,errors=[str(exc)])

def clock_panel(trace,contract,resolution):
 d=contract['clock']['dimensions'];width,height=resolution;x=d['CHIP_X3']-max(0,d['CHIP_X3']+d['CLOCK_W']+16-width);clicks=list(re.finditer(r'\[shell.silkbar.click\] target=clock x=(\d+) y=(\d+)',trace));assert len(clicks)==2,'actual clock hit-test open/close clicks absent'
 for click in clicks:assert x<=int(click[1])<x+d['CLOCK_W'] and d['CHIP_Y']<=int(click[2])<d['CHIP_Y']+d['CHIP_H'],'actual click missed source-bound clock module'
 panels=list(re.finditer(r'\[shell.panel.geometry\] surface=148 x=(\d+) y=(\d+) w=(\d+) h=(\d+) desktop_w=(\d+) desktop_h=(\d+) submitted=1',trace));assert len(panels)==1,'missing/foreign clock panel geometry';panel=panels[0];px,py,pw,ph,desktop_w,desktop_h=map(int,panel.groups());assert (desktop_w,desktop_h)==resolution and (pw,ph)==(240,300) and 0<=px<=width-pw and 50<=py<=height-ph,'submitted clock panel is clipped/outside desktop'
 opened=re.search(r'\[shell.clock.open.ok\] id=0x94',trace);closed=re.search(r'\[shell.clock.close.ok\] id=0x94',trace);started=re.search(r'\[browser.command.started\] kind=TlsPrepare accepted=1',trace);assert opened and closed and started and clicks[0].start()<panel.start()<opened.start()<clicks[1].start()<closed.start()<started.start(),'clock panel open/close did not precede TLS workload'
 return dict(clicks=[dict(x=int(c[1]),y=int(c[2]),offset=c.start())for c in clicks],submitted_geometry=dict(surface=148,x=px,y=py,w=pw,h=ph,desktop_w=width,desktop_h=height),pixels='unconfirmed; submitted geometry and actual Shell hit-test only')

def verify_demo_clock(trace,ppm,contract,resolution=None):
 try:
  layouts=list(re.finditer(r'\[sexdisplay.clock.presentation\] label=DEMO wall_time=0 x=(\d+) y=(\d+) w=(\d+) h=(\d+) digits=30 width=(\d+)',trace));assert len(layouts)==trace.count('[sexdisplay.clock.presentation]')==1,'missing/foreign DEMO presentation'
  x,y,w,h,declared=map(int,layouts[0].groups());width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1);d=contract['clock']['dimensions'];wanted_x=d['CHIP_X3']-max(0,d['CHIP_X3']+d['CLOCK_W']+16-width)
  assert (width,height)==tuple(resolution)if resolution else True,'wrong framebuffer resolution'
  assert (x,y,w,h,declared)==(wanted_x,d['CHIP_Y'],d['CLOCK_W'],d['CHIP_H'],width)and x>=0 and x+w<=width and y+8<=height and w>=76,'clock geometry differs from source-bound supported width'
  color=contract['clock']['color'].to_bytes(3,'big');actual=lambda xx,yy:pixels[((y+1+yy)*width+x+xx)*3:((y+1+yy)*width+x+xx)*3+3]==color;expected=set()
  for col,char in enumerate('DEMO '):
   for yy,bits in enumerate(glyph(contract['font'],char)):
    for xx in range(5):
     if bits&(1<<(4-xx)):expected.add((col*6+xx,yy))
  values=[]
  for offset in(0,7,17,24,34,41):
   found=[]
   for digit,bitmap in enumerate(contract['clock']['digits']):
    if all(actual(30+offset+xx,yy)==bool(bitmap[yy]&(1<<(4-xx)))for yy in range(7)for xx in range(5)):found.append(digit)
   assert len(found)==1,'clock digit absent/torn/ambiguous';values.append(found[0])
   for yy,bits in enumerate(contract['clock']['digits'][found[0]]):
    for xx in range(5):
     if bits&(1<<(4-xx)):expected.add((30+offset+xx,yy))
  for colon in(44,61):
   for yy in(1,5):expected.add((colon,yy))
  assert all(actual(xx,yy)==((xx,yy)in expected)for xx in range(76)for yy in range(7)),'DEMO prefix/colon/digit spacing/erasure differs'
  hh,mm,ss=(values[0]*10+values[1],values[2]*10+values[3],values[4]*10+values[5]);assert hh<24 and mm<60 and ss<60,'invalid software demonstration counter'
  return dict(ok=True,errors=[],geometry=dict(x=x,y=y,w=w,h=h,width=width),label='DEMO',wall_time=False,software_counter=f'{hh:02d}:{mm:02d}:{ss:02d}',checked_pixels=76*7,expected_ink_pixels=len(expected))
 except(AssertionError,ValueError,KeyError,IndexError,TypeError)as exc:return dict(ok=False,errors=[str(exc)])

def durable_snapshot(disk,expected):
 actual=recover(disk,2);assert actual['payload']==expected and actual['bytes']==len(expected)and int(actual['hash'],16)==fnv(expected),'independent durable QJ02 bytes differ';return {k:v for k,v in actual.items()if k!='payload'}

def document_evidence(disk):
 """Raw manifest/descriptor/all object extents, sufficient for real recovery."""
 segments=[]
 def take(offset,size):
  raw=disk[offset:offset+size];assert len(raw)==size,'truncated raw disk evidence';segments.append(dict(offset=offset,bytes=size,hex=raw.hex(),sha256=hashlib.sha256(raw).hexdigest()));return raw
 take(2046*512,512);descriptor=take(1923*512,512);assert descriptor[:4]==b'SFEX','raw document descriptor missing';count=struct.unpack_from('<H',descriptor,6)[0];assert 0<count<=8,'raw document extent count differs'
 for index in range(count):
  sector,size=struct.unpack_from('<HH',descriptor,12+index*4);assert size>0,'empty raw document extent';take(sector*512,size*512)
 return dict(image_bytes=len(disk),object=2,segments=segments,scope='Actual raw doc2 manifest, descriptor and all extents after positive Save ACK, before any further edit/save; guest remains live and idle for this document. Sufficient to independently reconstruct a sparse image and run check_document_disk.recover. No power-loss/crash durability inference.')

def native_command(iso,disk,serial,sock,pcap,model,resolution):
 return ['qemu-system-x86_64','-M','q35','-cpu','max,+pku,+rdseed','-rtc',preparation.RTC_NORMAL,'-m','512M','-vga','none','-device','VGA,xres='+resolution.replace('x',',yres='),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',f'{model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}']

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--iso',type=Path,required=True);parser.add_argument('--binding',type=Path,required=True);parser.add_argument('--source-disk',type=Path,required=True);parser.add_argument('--out',type=Path,required=True);parser.add_argument('--model',choices=('e1000','e1000e'),default='e1000e');parser.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');a=parser.parse_args()
 root=Path(__file__).resolve().parents[1];iso=a.iso.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh responsiveness evidence directory required')
 bound=frozen_binding(root,iso,a.binding.resolve(strict=True));held=json.loads(a.binding.read_text());contract=source_contract(root);target=target_check(root)
 if not target['ok']or held['runner_sha256'].get('scripts/tls_responsiveness_gate.py')!=hashlib.sha256(Path(__file__).read_bytes()).hexdigest():raise RuntimeError('responsiveness source/target not exact frozen cohort')
 peer=https.ControlledHTTPSPeer(root,'partial-close');peer.start();disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-tls-responsive-{os.getpid()}.sock';resolution=tuple(map(int,a.resolution.split('x')));command=native_command(iso,disk,serial,sock,pcap,a.model,a.resolution);profile=https.tls.command_profile(command)
 if not profile['ok']:raise RuntimeError(str(profile))
 host_start=time.time();manifest=dict(binding=bound,command=command,qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),model=a.model,resolution=a.resolution,platform_profile=profile,target=target,host_utc_start=host_start,input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),pixel_source_sha256=contract['source_sha256'],keylog_enabled=False,typing_token=TOKEN.decode(),after_close_token=AFTER.decode(),scope=SCOPE)
 (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic();failure=None;pixels=[];actions=[];durable=[];cleanup=None
 def text():return serial.read_text(errors='replace')if serial.exists()else''
 def health():
  if time.monotonic()-began>300:raise TimeoutError('bounded300second responsiveness workflow')
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if peer.errors:raise RuntimeError(str(peer.errors))
  if any(p.exists()and p.stat().st_size>MAX_PCAP for p in(serial,pcap)):raise RuntimeError('bounded responsiveness raw evidence exceeded')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',text()):raise RuntimeError('guest failure')
 def wait(pattern,start=0,timeout=60):
  end=time.monotonic()+timeout
  while time.monotonic()<end:
   health();m=re.search(pattern,text()[start:])
   if m:return m
   time.sleep(.002)
  raise TimeoutError(pattern)
 def key(code,pattern):
  start=len(text());q.key(code);return wait(pattern,start)
 def record(action,start,sent,**fields):
  actions.append(dict(action=action,serial_start=start,serial_end=len(text()),host_seconds=time.monotonic()-sent,**fields))
 def picture(name,kind,expected=None):
  path=out/(name+'.ppm');deadline=time.monotonic()+20;result=dict(ok=False,errors=['no frame']);sent=time.monotonic()
  while time.monotonic()<deadline:
   health();q.cmd('screendump',{'filename':str(path)})
   if kind=='quil':
    snapshot=text();result=verify_quil_frame(snapshot,path.read_bytes(),expected,contract,resolution)
   elif kind=='private':result=lifecycle.verify_private_pixels(text(),path.read_bytes(),expected,contract,resolution)
   elif kind=='https':result=https.verify_status_pixels(text(),path.read_bytes(),expected,'partial-close',contract,resolution)
   else:result=verify_demo_clock(text(),path.read_bytes(),contract,resolution)
   if result['ok']:break
   time.sleep(.05)
  result.update(kind=kind,screenshot=path.name,screenshot_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),source_sha256=contract['source_sha256'],serial_end=len(text()),host_seconds=time.monotonic()-sent,expected=expected);pixels.append(result)
  if not result['ok']:raise RuntimeError(str(result['errors']))
  return result
 def click_clock():
  d=contract['clock']['dimensions'];x=d['CHIP_X3']-max(0,d['CHIP_X3']+d['CLOCK_W']+16-resolution[0])+d['CLOCK_W']//2;y=d['CHIP_Y']+d['CHIP_H']//2
  for expected in('open','close'):
   start=len(text());sent=time.monotonic();q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(resolution[0]-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(resolution[1]-1))}}]});wait(r'\[shell.pointer.abs\]|\[silk-shell.pointer.recv\].*class=|\[shell.abs.normalize\]',start);q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':True,'button':'left'}}]});wait(r'\[shell.clock.'+expected+r'.ok\] id=0x94',start);q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':False,'button':'left'}}]});record('clock_panel_'+expected,start,sent)
  start=len(text());q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':30000}},{'type':'abs','data':{'axis':'y','value':30000}}]});wait(r'\[shell.abs.normalize\] raw_x=30000 raw_y=30000 .*reason=(?:ok|duplicate_sample)',start)
 def zoom():
  key('grave_accent',r'\[shell.palette.item\] idx=0')
  for old,new in((0,9),(9,8)):key('k',rf'\[shell.palette.select\] old={old} new={new}')
  start=len(text());q.key('ret');wait(r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1',start);wait(r'\[browser.viewport\] target=15 task=15 sequence=\d+ w=\d+ h=\d+ origin=held_shell',start)
 def save(payload,name):
  start=len(text());sent=time.monotonic();q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'s'}],'hold-time':80});receipt=wait(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=2',start)
  if(int(receipt[1]),int(receipt[2],16))!=(len(payload),fnv(payload)):raise RuntimeError('actual Quil saved wrong revision')
  snapshot_bytes=disk.read_bytes();observed=durable_snapshot(snapshot_bytes,payload);evidence=document_evidence(snapshot_bytes);evidence_path=out/(name+'-raw-document.json.gz');evidence_path.write_bytes(gzip.compress((json.dumps(evidence,indent=2)+'\n').encode(),mtime=0));durable.append(dict(name=name,raw_document=evidence_path.name,raw_document_sha256=hashlib.sha256(evidence_path.read_bytes()).hexdigest(),**observed));(out/(name+'-disk.json')).write_text(json.dumps(observed,indent=2)+'\n');record(name,start,sent,bytes=len(payload),hash=hex(fnv(payload)))
 try:
  end=time.monotonic()+15
  while not sock.exists():
   health()
   if time.monotonic()>end:raise TimeoutError('QMP unavailable')
   time.sleep(.02)
  q=QMP(sock)
  if a.resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0');wait(r'\[linen.disk.publish.done\]');focus_surface(201,text,key);key('esc',r'\[quil.palette.action\] kind=esc clear=1');focus_surface(300,text,key);zoom();picture('clock-before','clock');click_clock()
  for attempt in(1,2):
   start=len(text());sent=time.monotonic();q.key('l');wait(r'\[browser.tls.view\] state=HelloPending authenticated=0 network=0',start);record('L'+str(attempt),start,sent);picture('local-'+str(attempt),'private','HelloPending');start=len(text());sent=time.monotonic();q.key_sequence(['q']);wait(r'\[browser.tls.view\] state=Cancelled authenticated=0 network=0',start);wait(r'\[browser.resources\] reason=clear heap_used=0 heap_free=65536 heap_total=65536 tls_payload=0 ',start);record('Q'+str(attempt),start,sent);picture('cleared-'+str(attempt),'private','Cancelled')
  zoom() # Restore tiling so Quil remains visible while HTTPS owns its tile.
  route=[]
  def planned_key(code,pattern):route.append(code);return key(code,pattern)
  focus_surface(201,text,planned_key);focus_surface(300,text,key)
  if not route or len(route)>24:raise RuntimeError('actual Browser-to-Quil focus path unavailable')
  start=len(text());sent=time.monotonic();q.key_sequence(['b']+route+list(TOKEN.decode()));wait(r'\[quil.document.edit\].*bytes='+str(len(TOKEN)+len(contract['quil_initial']))+r'(?:\s|$)',start);record('B_and_queued_Quil_input',start,sent,focus_keys=route);wait(r'\[browser.https.progress\] ticket=2 body_received=1024 authenticated=1 body_visible=0',start);picture('quil-during-https','quil',TOKEN.decode());save(TOKEN+contract['quil_initial'],'save-during-https');picture('clock-held-https','clock')
  focus_surface(300,text,key);zoom();picture('https-held','https','Pending');start=len(text());sent=time.monotonic();q.key_sequence(['f11']);wait(r'\[browser.https.closed\] pending=1 terminal=1 owned_storage=erased body_visible=0',start);wait(r'\[sext.app.pool.ready\] target=15 task=15 .*ok=1',start);cleanup=cleanup_receipts(text(),15,15,start);record('Close_partial',start,sent)
  focus_surface(201,text,key);start=len(text());sent=time.monotonic();q.key_sequence(list(AFTER.decode()));wait(r'\[quil.document.edit\].*bytes='+str(len(TOKEN)+len(AFTER)+len(contract['quil_initial']))+r'(?:\s|$)',start);record('Quil_after_Close',start,sent);picture('quil-after-close','quil',(TOKEN+AFTER).decode());save(TOKEN+AFTER+contract['quil_initial'],'save-after-close');picture('clock-after-close','clock');clocks=[p['software_counter']for p in pixels if p['kind']=='clock']
  if len(clocks)!=3 or len(set(clocks))<2:raise RuntimeError('actual DEMO clock pixels did not progress across workflow')
 except Exception as exc:failure=repr(exc)
 finally:
  host_end=time.time();peer.stop.set()
  for listener in peer.sockets:
   try:listener.close()
   except OSError:pass
  for live in peer.live:
   try:live.close()
   except OSError:pass
  for thread in peer.threads:thread.join(timeout=1)
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
  # Final disk is captured only after QEMU has exited and all writes stopped.
  with disk.open('rb')as source,(out/'nvme.img.gz').open('wb')as destination:
   with gzip.GzipFile(fileobj=destination,mode='wb',mtime=0)as zipped:shutil.copyfileobj(source,zipped)
 manifest['host_utc_end']=host_end;(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');raw=pcap.read_bytes()if pcap.exists()else b'';trace=text();wire=https.inspect_wire(raw,peer.rows,'partial-close');dns=dns_capture(raw,'echo',attempts=peer.attempts,expected=peer.dns)if peer.dns else dict(ok=False,errors=['missing actual DNS']);progress=inspect_progress(trace,wire,dns,raw,host_start,host_end,contract);panel_check=dict(ok=False,errors=['workflow incomplete'])
 try:panel_check=dict(ok=True,errors=[],joins=clock_panel(trace,contract,resolution))
 except(AssertionError,ValueError,KeyError,IndexError)as exc:panel_check['errors']=[str(exc)]
 authority=nic_check(trace,expected=1,preparation=True,ethernet=True,network_config=True,routing=True);waiting=wait_check(trace)
 (out/'peer-exchanges.json').write_text(json.dumps(dict(tls=peer.rows,dns=peer.dns,dns_send_attempts=peer.attempts,keylog_enabled=False),indent=2)+'\n');result=dict(ok=failure is None and not peer.errors and all(v['ok']for v in(wire,dns,progress,panel_check,authority,waiting)),error=failure,server_errors=peer.errors,binding=bound,wire=wire,dns=dns,progress=progress,clock_panel=panel_check,authority=authority,self_wait=waiting,pixels=pixels,actions=actions,durable=durable,cleanup=cleanup,scope=SCOPE);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(result))
 print(json.dumps(dict(ok=True,actions=actions,cleanup=cleanup)))

if __name__=='__main__':main()
