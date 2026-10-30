#!/usr/bin/env python3
"""Real E/Q seed-custody status without recording secret sample values.

This is functional BOOT/provider/broker/client/glyph qualification, never a
randomness-quality, opcode-exclusivity, replay-security or hardware certificate.
"""
import argparse,ast,hashlib,json,os,re,shutil,socket,subprocess,threading,time
from pathlib import Path
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from network_contention_gate import frozen_binding,owner_joins,cleanup_receipts,MAX_PCAP
from http_content_gate import load_pixel_contract
from check_tcp_capture import inspect as tcp_capture,frames
from check_nic_command import inspect as nic_check
from check_self_wait import inspect as wait_check
from check_desktop_pixels import glyph
from check_disk_persistence import ppm_rgb

ECHO=bytes((n*17+78)&255 for n in range(1025))
LABELS={'Pending':b'checking','Ready':b'ready 32 bytes','Unavailable':b'unavailable','Cancelled':b'cancelled','Uncertain':b'waiting receipt'}

def command_profile(command,mode):
 errors=[];cpu='max,+pku,-rdseed,-rdrand'if mode=='unsupported'else'max,+pku,+rdseed'
 if mode not in('ready','unsupported','close'):errors.append('unsupported explicit seed mode')
 if command.count('-cpu')!=1 or '-cpu'not in command or command[command.index('-cpu')+1]!=cpu:errors.append('wrong exact CPU feature profile')
 if any(re.search(r'(^|[,=])(?:seed|icount|incoming|replay|rr|rrfile|loadvm)(?:=|$)',arg.lstrip('-'),re.I)or arg in('-seed','-icount','-incoming','-replay','-loadvm')for arg in command):errors.append('deterministic seed/replay/state injection flag forbidden')
 return dict(ok=not errors,errors=errors,cpu=cpu,scope='Exact recorded QEMU feature/command profile only; absence of replay flags is not a random-source quality certificate.')

def seed_contract(root):
 contract=load_pixel_contract(root);view=(root/'apps/kaleidoscope/src/browser_view.rs').read_text();app=(root/'apps/kaleidoscope/src/main.rs').read_text()
 if 'pub fn with_seed('not in view or 'start.checked_add(64)'not in view or 'browser_view::with_seed(text,seed_label(self.seed_view))'not in app:raise ValueError('frozen whole64-byte seed extent contract changed')
 if 'if self.intent==Some(Intent::Seed)'not in app or 'else{self.seed.view()};if view!=self.seed_view{self.seed_view=view;'not in app:raise ValueError('queued seed status no longer joins actual client view')
 labels={key:ast.literal_eval(value)for key,value in re.findall(r'entropy_probe::View::(\w+)\s*=>\s*Some\((b"[^"\n]*")\)',app)}
 if labels!=LABELS:raise ValueError('source-bound seed labels differ')
 for literal in ('b"\\n\\nPrivate seed: "','b"\\nE seed  Q clear"'):
  if literal not in view:raise ValueError('source-bound seed chrome changed')
 contract['seed_labels']=labels
 for name in ('apps/kaleidoscope/src/entropy_probe.rs','servers/sexnet/src/entropy.rs','kernel/src/entropy_source.rs','crates/sex-pdx/src/entropy_source.rs','crates/sex-pdx/src/entropy_seed.rs'):
  contract['source_sha256'][name]=hashlib.sha256((root/name).read_bytes()).hexdigest()
 return contract

def seed_geometry(trace,expected,contract):
 layouts=list(re.finditer(r'\[browser.entropy.layout\] base=(\d+) bytes=64 viewport=(\d+) state=(\w+) pixels=unconfirmed',trace))
 if not layouts or layouts[-1][3]!=expected:raise ValueError('latest actual seed layout has wrong state')
 marker=layouts[-1];base,sequence=int(marker[1]),int(marker[2]);previews=re.findall(r'\[browser.preview.layout\] bytes=(\d+) newline=1 cached_or_sample=1',trace)
 if not previews:raise ValueError('missing actual preview extent')
 length=int(previews[-1]);prefix=contract['prefix']
 if base!=length+258 or not len(prefix)<length<=192:raise ValueError('seed extent not appended to complete actual overview')
 queued=re.findall(rf'\[shell.document.viewport\] target=15 task=15 surface=300 sequence={sequence} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) enqueued=1',trace)
 accepted=re.findall(rf'\[browser.viewport\] target=15 task=15 sequence={sequence} w=(\d+) h=(\d+) origin=held_shell',trace)
 if len(queued)!=1 or not accepted:raise ValueError('missing exact held-shell viewport')
 x,y,w,h=map(int,queued[0]);nw,nh=map(int,accepted[-1])
 if(w,h)!=(nw,nh):raise ValueError('seed viewport geometry differs from held notice')
 if not re.search(rf'\[browser.view.applied\] bytes={base+64} http=0 body_bytes=0 pixels=unconfirmed',trace[marker.end():]):raise ValueError('missing positive atomic complete overview Applied after seed layout')
 # This workflow performs only E/Q; baseline Echo is positively verified and
 # neither DNS nor HTTP intent occurs. Preview controls are source-sanitized.
 echo=contract['echo']['Verified'].ljust(64,b' ')
 dns=(contract['dns_prefix']+contract['dns']['Idle']+contract['dns_footer']).ljust(64,b' ')
 http=(contract['http_prefix']+contract['http']['Idle']+contract['http_footer']).ljust(64,b' ')
 preceding=prefix+b'?'*(length-len(prefix))+echo+dns+http+b'\n\n'+b' '*64
 if len(preceding)!=base:raise ValueError('source overview extents changed')
 status=(b'\n\nPrivate seed: '+contract['seed_labels'][expected]+b'\nE seed  Q clear').ljust(64,b' ')
 if len(status)!=64:raise ValueError('seed label overflow')
 data=preceding+status;scale=2 if w>=contract['scale_threshold']else 1;cols=min(80,max(1,(w-2*contract['inset'])//(contract['advance']*scale)))
 starts=[0];column=0
 for index,byte in enumerate(data):
  if byte==10:starts.append(index+1);column=0
  else:
   column+=1
   if column>=cols and index+1<len(data):starts.append(index+1);column=0
 if len(starts)>contract['max_rows']:raise ValueError('seed text exceeds renderer row table')
 rows=[]
 for row,start in enumerate(starts):
  if start<=base:continue
  end=starts[row+1]if row+1<len(starts)else len(data);line=data[start:end].rstrip(b'\n')
  py=y+contract['top']+row*contract['line']*scale
  if py+contract['line']*scale>y+h:raise ValueError('entire64-byte seed status is clipped')
  rows.append(dict(row=row,text=line.decode(),x=x+contract['inset'],y=py))
 if not rows or not any('Private seed:'in row['text']for row in rows):raise ValueError('seed status label not fully visible')
 return dict(tile=dict(x=x,y=y,w=w,h=h),base=base,bytes=64,viewport=sequence,scale=scale,columns=cols,rows=rows)

def verify_seed_pixels(trace,ppm,expected,contract,resolution=None):
 try:
  geometry=seed_geometry(trace,expected,contract);width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1)
  if resolution and(width,height)!=tuple(resolution):raise ValueError('wrong actual framebuffer resolution')
  tile=geometry['tile'];scale=geometry['scale'];checked=ink=mismatch=0;color=contract['color'].to_bytes(3,'big')
  if tile['x']<0 or tile['y']<50 or tile['x']+tile['w']>width or tile['y']+tile['h']>height:raise ValueError('actual seed viewport outside framebuffer')
  for row in geometry['rows']:
   for col in range(geometry['columns']):
    bitmap=glyph(contract['font'],row['text'][col])if col<len(row['text'])else(0,)*7
    for gy in range(contract['line']):
     for gx in range(contract['advance']):
      on=gy<7 and gx<5 and bool(bitmap[gy]&(1<<(4-gx)))
      for dy in range(scale):
       for dx in range(scale):
        px=row['x']+(col*contract['advance']+gx)*scale+dx;py=row['y']+gy*scale+dy
        actual=pixels[(py*width+px)*3:(py*width+px)*3+3]==color;checked+=1;ink+=on;mismatch+=actual!=on
  errors=[]if not mismatch else[f'complete seed glyph/padding/trailing-erasure mismatches={mismatch}']
  if not ink:errors.append('no expected seed status ink')
  return dict(ok=not errors,errors=errors,expected=expected,geometry=geometry,checked_pixels=checked,expected_ink_pixels=ink)
 except(ValueError,KeyError,IndexError,UnicodeError,OverflowError)as exc:return dict(ok=False,errors=[str(exc)],expected=expected)

def inspect_seed(trace,mode,command):
 errors=list(command_profile(command,mode)['errors']);joins=[]
 try:
  boot=list(re.finditer(r'\[boot.entropy.source\] holder=13 task=13 slot=60 consumer=15 consumer_task=15 source=rdseed attempts=8 runtime_mint=0',trace));assert len(boot)==1 and trace.count('[boot.entropy.source]')==1,'wrong/duplicate exact BOOT source grant'
  assert trace.count('[browser.entropy.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1')==1,'raw source denials absent'
  available=0 if mode=='unsupported'else 1
  assert trace.count(f'[sexnet.entropy.source] held=1 available={available} source=rdseed fallback=0 broker=private_seed32')==1,'wrong actual held-source availability'
  lines=[line for line in trace.splitlines()if re.search(r'\[(?:browser\.entropy|entropy\.|sexnet\.entropy|boot\.entropy)',line)]
  assert not any(re.search(r'\b(?:hash|fingerprint|word|value|hex|seed_bytes|seed_value|sample_value|data)=',line)for line in lines),'secret-bearing entropy log field'
  samples=list(re.finditer(r'\[entropy.source.sample\] pd=(\d+) task=(\d+) slot=(\d+) source=rdseed attempts=(\d+) count=(\d+) private=1',trace))
  assert trace.count('[entropy.source.sample]')==len(samples),'malformed secret-source metadata log'
  for m in samples:assert tuple(map(int,m.groups()[:3]))==(13,13,60)and 1<=int(m[4])<=8,'wrong exact source task/slot/attempt bound'
  counts=[int(m[5])for m in samples];assert all(a<b for a,b in zip(counts,counts[1:])),'source metadata count regressed'
  admitted=list(re.finditer(r'\[entropy.seed.admitted\] owner=15 task=15 ticket=(\d+) bytes=32 source=held_boot_grant',trace))
  assert trace.count('[entropy.seed.admitted]')==len(admitted),'foreign/malformed seed admission'
  releases=list(re.finditer(r'\[entropy.seed.released\] owner=15 task=15 ticket=(\d+) service_payload=erased receipt=retained',trace))
  assert trace.count('[entropy.seed.released]')==len(releases),'foreign/malformed seed release'
  ready=list(re.finditer(r'\[browser.entropy.view\] state=Ready private=1 bytes_visible=0',trace))
  if mode=='unsupported':
   assert not samples and not admitted and not releases and not ready,'unsupported CPU sampled/admitted/published seed'
   assert len(re.findall(r'\[browser.entropy.view\] state=Unavailable private=1 bytes_visible=0',trace))==2,'unsupported E/Q/E did not reject both explicit requests'
  elif mode=='ready':
   assert [int(m[1])for m in admitted]==[1,2]and[int(m[1])for m in releases]==[1,2]and len(ready)==2,'wrong explicit seed runs or cleanup count'
   assert counts==[1,2,3,4,8],'sampled provider metadata differs from first4/powers2 source policy'
   for ticket,admission,release,view in zip((1,2),admitted,releases,ready):
    collected=re.search(rf'\[entropy.seed.progress\] owner=Some\(Identity \{{ pd: 15, task: 15 \}}\) ticket={ticket} state=Some\(Ready\) produced=32 consumed=0 failure=0 private=1',trace)
    assert collected and admission.start()<collected.start()<release.start()<view.start(),'Ready did not follow exact produced32/positive service RELEASE'
    joins.append(dict(ticket=ticket,admitted=admission.start(),produced32=collected.start(),release=release.start(),browser_ready=view.start()))
  else:
   assert [int(m[1])for m in admitted]==[1]and not releases and not ready,'Close did not overlap one still-owned seed job'
   assert '[browser.entropy.closed] target=15 task=15 pending=1 terminal=1 private_payload=erased'in trace,'Close missed actual pending client'
   retired=re.search(r'\[entropy.seed.retired\] target=15 task=15 before_cleanup_ack=1 private_payload=erased',trace);ack=re.search(r'\[net.app.cleanup.ack\] target=15 task=15 client_leases=0 shared_cache=retained ok=1',trace)
   assert retired and ack and retired.start()<ack.start(),'private seed retirement did not precede exact cleanup ACK'
  intents=trace.count('[browser.entropy.intent] accepted=1 explicit=1 fallback=0');assert intents==(1 if mode=='close'else 2),'wrong explicit E intent count'
  if mode!='close':assert '[browser.entropy.view] state=Cancelled private=1 bytes_visible=0'in trace,'actual Q did not clear private seed'
 except(AssertionError,AttributeError)as exc:errors.append(str(exc))
 return dict(ok=not errors,errors=errors,joins=joins,samples_observed=[dict(attempts=int(m[4]),ordinal=int(m[5]))for m in locals().get('samples',[])],scope='Exact BOOT/provider metadata, bounded source attempts, produced32 and positive service RELEASE before source-bound ClientReady. Actual32-byte READ completeness is established by frozen Client guards plus actual host client/broker tests; no invented consumed32 native receipt. No values or derivatives are logged; no randomness-quality, instruction exclusivity, SMP, physical or replay-security certification.')

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--mode',choices=['ready','unsupported','close'],default='ready');p.add_argument('--model',choices=['e1000','e1000e'],default='e1000e');p.add_argument('--resolution',choices=['1280x800','1024x768'],default='1280x800');a=p.parse_args()
 root=Path(__file__).resolve().parents[1];iso=a.iso.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 binding=frozen_binding(root,iso,a.binding.resolve(strict=True));contract=seed_contract(root)
 if json.loads(a.binding.read_text())['runner_sha256'].get('scripts/entropy_seed_gate.py')!=hashlib.sha256(Path(__file__).read_bytes()).hexdigest():raise RuntimeError('seed gate absent from frozen runner cohort')
 listener=socket.socket();listener.bind(('127.0.0.1',46078));listener.listen(1);listener.settimeout(.1);stop=threading.Event();server_errors=[];peer_rows=[]
 def peer():
  conn=None
  try:
   while not stop.is_set():
    try:conn,_=listener.accept();break
    except socket.timeout:continue
   if conn is None:return
   conn.settimeout(25);received=bytearray()
   while True:
    part=conn.recv(512)
    if not part:break
    received.extend(part)
    if len(received)>1025:raise RuntimeError('unexpected baseline/entropy network payload')
   if received!=ECHO:raise RuntimeError('wrong public baseline Echo bytes')
   conn.sendall(ECHO);conn.shutdown(socket.SHUT_WR);conn.close();conn=None;peer_rows.append(dict(payload_hex=ECHO.hex(),response_hex=ECHO.hex()))
   while not stop.is_set():
    try:extra,_=listener.accept();extra.close();raise RuntimeError('E/Q unexpectedly opened another TCP flow')
    except socket.timeout:continue
  except Exception as exc:
   if not stop.is_set():server_errors.append(repr(exc))
  finally:
   if conn:conn.close()
 thread=threading.Thread(target=peer,daemon=True);thread.start();disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-seed-{os.getpid()}.sock';cpu='max,+pku,-rdseed,-rdrand'if a.mode=='unsupported'else'max,+pku,+rdseed'
 command=['qemu-system-x86_64','-M','q35','-cpu',cpu,'-m','512M','-vga','none','-device','VGA,xres='+a.resolution.replace('x',',yres='),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}']
 profile=command_profile(command,a.mode)
 if not profile['ok']:raise RuntimeError(str(profile))
 version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True)
 (out/'manifest.json').write_text(json.dumps(dict(binding=binding,command=command,qemu_version=version,mode=a.mode,model=a.model,resolution=a.resolution,cpu_profile=profile,pixel_source_sha256=contract['source_sha256'],input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),secret_capture=False),indent=2)+'\n')
 proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic();failure=None;pixels=[];cleanup=None;actions=[]
 def text():return serial.read_text(errors='replace')if serial.exists()else''
 def health():
  if time.monotonic()-began>180:raise TimeoutError('bounded180-second whole gate')
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if server_errors:raise RuntimeError(str(server_errors))
  if any(p.exists()and p.stat().st_size>MAX_PCAP for p in(serial,pcap)):raise RuntimeError('bounded raw evidence exceeded')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',text()):raise RuntimeError('guest failure')
 def wait(pattern,start=0,timeout=60,interval=.01):
  deadline=time.monotonic()+timeout
  while time.monotonic()<deadline:
   health();match=re.search(pattern,text()[start:])
   if match:return match
   time.sleep(interval)
  raise TimeoutError(pattern)
 def key(code,pattern):
  start=len(text());q.key(code);return wait(pattern,start)
 def picture(state,name):
  deadline=time.monotonic()+20;path=out/(name+'.ppm');result=dict(ok=False,errors=['no frame'])
  while time.monotonic()<deadline:
   health();q.cmd('screendump',{'filename':str(path)});result=verify_seed_pixels(text(),path.read_bytes(),state,contract,tuple(map(int,a.resolution.split('x'))))
   if result['ok']:break
   time.sleep(.1)
  result.update(screenshot=path.name,screenshot_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),source_sha256=contract['source_sha256']);pixels.append(result);(out/(name+'-pixels.json')).write_text(json.dumps(result,indent=2)+'\n')
  if not result['ok']:raise RuntimeError(str(result['errors']))
 try:
  deadline=time.monotonic()+15
  while not sock.exists():
   health()
   if time.monotonic()>deadline:raise TimeoutError('QMP unavailable')
   time.sleep(.05)
  q=QMP(sock)
  if a.resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0');focus_surface(300,text,key)
  key('grave_accent',r'\[shell.palette.item\] idx=0')
  for old,new in[(0,9),(9,8)]:key('k',rf'\[shell.palette.select\] old={old} new={new}')
  start=len(text());q.key('ret');wait(r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1',start);wait(r'\[browser.viewport\] target=15 task=15 sequence=\d+ w=\d+ h=\d+ origin=held_shell',start)
  start=len(text());q.key('e');wait(r'\[shell.browser.input\] code=18 down=1 slot=43 accepted=1',start)
  if a.mode=='close':
   wait(r'\[entropy.seed.admitted\] owner=15 task=15 ticket=1 bytes=32 source=held_boot_grant',start,interval=.002);closed=len(text());q.key_sequence(['f11']);wait(r'\[browser.entropy.closed\] target=15 task=15 pending=1 terminal=1 private_payload=erased',closed);wait(r'\[sext.app.pool.ready\] target=15 task=15 .*ok=1',closed);cleanup=cleanup_receipts(text(),15,15,closed);q.cmd('screendump',{'filename':str(out/'seed-closed.ppm')});actions.append(dict(action='actual_F11_pending_seed',live_pixels='No live seed surface after terminal Close; exact pending, retire-before-ACK and five-role Sext cleanup qualify. Already-submitted DMA remains driver custody.'))
  else:
   expected='Unavailable'if a.mode=='unsupported'else'Ready';wait(rf'\[browser.entropy.view\] state={expected} private=1 bytes_visible=0',start);picture(expected,'seed-first')
   start=len(text());q.key('q');wait(r'\[shell.browser.input\] code=16 down=1 slot=43 accepted=1',start);wait(r'\[browser.entropy.view\] state=Cancelled private=1 bytes_visible=0',start);picture('Cancelled','seed-cleared')
   start=len(text());q.key('e');wait(rf'\[browser.entropy.view\] state={expected} private=1 bytes_visible=0',start);picture(expected,'seed-second');actions.append(dict(action='explicit_E_Q_E',expected=expected))
 except Exception as exc:failure=repr(exc)
 finally:
  stop.set();listener.close();thread.join(timeout=1)
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
 raw=pcap.read_bytes()if pcap.exists()else b'';trace=text();wire=tcp_capture(raw,peer_rows,guest_ports=range(48001,65536),initiator='guest')if raw else dict(ok=False,errors=['missing baseline wire capture']);owners=owner_joins(trace,raw,[(15,15)]);metadata=inspect_seed(trace,a.mode,command);authority=nic_check(trace,expected=1,preparation=True,ethernet=True,network_config=True,routing=True);waiting=wait_check(trace)
 try:
  for frame in frames(raw):
   if len(frame)>=42 and frame[12:14]==b'\x08\0'and frame[23]==17 and frame[26:30]==bytes([10,0,2,15]):
    ip=frame[14:];at=14+(ip[0]&15)*4
    if int.from_bytes(frame[at+2:at+4],'big')==46079:raise ValueError('seed intent emitted DNS query')
 except(ValueError,IndexError)as exc:failure=failure or str(exc)
 if len(peer_rows)!=1:failure=failure or'missing exact sole public startup flow'
 (out/'endpoint-exchanges.json').write_text(json.dumps(dict(public_baseline=peer_rows,seed_values_recorded=False),indent=2)+'\n')
 result=dict(ok=failure is None and not server_errors and all(v['ok']for v in(wire,owners,metadata,authority,waiting)),error=failure,server_errors=server_errors,mode=a.mode,binding=binding,source_metadata=metadata,public_wire=wire,owner_packet_joins=owners,authority=authority,self_wait=waiting,pixels=pixels,cleanup=cleanup,actions=actions,scope='Explicit real E/Q input, exact immutable BOOT source/consumer tasks, bounded source metadata and private broker/client positive cleanup, source-bound full status glyphs or exact pending terminal Close. No seed bytes/hashes/fingerprints captured. Source-bound ClientReady plus host actual READ tests establish32-byte transfer; native metadata does not invent absent consumed32 receipts. No randomness-quality, opcode-exclusivity, replay-security, SMP/physical or cryptographic construction certification.')
 (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(result))
 print(json.dumps(dict(ok=True,mode=a.mode,seed_values_recorded=False)))

if __name__=='__main__':main()
