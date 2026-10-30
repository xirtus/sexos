#!/usr/bin/env python3
"""Actual controlled HTTPS import, create-only journal, user reopen and cold boot."""
import argparse,gzip,hashlib,json,os,re,shutil,socket,ssl,struct,subprocess,time
from pathlib import Path
import https_document_gate as https
import tls_responsiveness_gate as desktop
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from network_contention_gate import frozen_binding,cleanup_receipts,MAX_PCAP
from check_document_disk import recover,object_bytes,fnv
from check_desktop_pixels import final_view,verify as linen_pixels,font_from_source
from check_quil_ui_pixels import verify as quil_pixels,view as quil_view
from check_dns_capture import inspect as dns_capture
from check_nic_command import inspect as nic_check
from check_self_wait import inspect as wait_check
from check_nonunix_target import inspect as target_check

NAME='web-note';BODY=https.BODY;COLLISION_BODY=BODY[:-1]+b'Y';BODIES=(BODY,COLLISION_BODY)
SCOPE='Two actual boots of the same frozen ISO: fixed controlled HTTPS authenticated body/positive network RELEASE, explicit create-only import, positive typed journal COMMIT+END, user-selected Linen reopen in Quil and exact raw durable bytes after cold recovery. A second distinct authenticated body collides with the saved name and may not overwrite it. Deliberately public laboratory root only; no publicInternet/publicCA, arbitrary endpoint, automatic execution, proxy, TLS-secret capture, negative transport EOF, power-loss or physical/SMP claim.'

def response(attempt):
 body=BODIES[attempt-1];return b'HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: '+str(len(body)).encode()+b'\r\nConnection: close\r\n\r\n'+body

class ImportPeer(https.tls.ControlledPeer):
 """Finite genuine TLS endpoint, never a forwarding proxy."""
 def __init__(self,root,connections=2):
  if connections not in(0,2):raise ValueError('finite import/recovery profile required')
  super().__init__(root,'import');self.connections=connections
 def serve_dns(self,sock):
  while not self.stop.is_set():
   try:data,remote=sock.recvfrom(513)
   except socket.timeout:continue
   ticket=len(self.dns)+1
   if ticket>self.connections or data!=https.tls.dns_query(ticket):raise ValueError('unexpected/repeated import DNS request')
   reply=https.tls.dns_response(ticket);sent=sock.sendto(reply,remote);self.dns.append(dict(ticket=ticket,response_hex=reply.hex()));self.attempts.append(dict(ticket=ticket,kind='answer',response_hex=reply.hex(),sent_bytes=sent))
 def serve_tls(self,listener,ctx):
  for attempt in range(1,self.connections+1):
   sock=self.accept(listener)
   if sock is None:return
   row=dict(attempt=attempt,handshake_complete=False,sni=None,tls_version=None,cipher=None,request_hex=None,response_hex=None,application_bytes_received=0,extra_application_bytes=0,unwrap_completed=False);self.rows.append(row);wrapped=ctx.wrap_socket(sock,server_side=True,do_handshake_on_connect=False);self.live.append(wrapped);wrapped.do_handshake();row.update(handshake_complete=True,sni=getattr(wrapped,'controlled_sni',None),tls_version=wrapped.version(),cipher=wrapped.cipher()[0])
   if(row['sni'],row['tls_version'],row['cipher'])!=(https.tls.HOST,'TLSv1.3',https.tls.CIPHER):raise ValueError('actual import TLS endpoint negotiated foreign profile')
   request=bytearray()
   while len(request)<len(https.REQUEST):
    chunk=wrapped.recv(4096)
    if not chunk:raise ValueError('guest closed before exact import GET')
    request.extend(chunk)
    if len(request)>len(https.REQUEST):raise ValueError('extra decrypted import request')
   if bytes(request)!=https.REQUEST:raise ValueError('actual import HTTP request differs')
   wire=response(attempt);row.update(request_hex=bytes(request).hex(),application_bytes_received=len(request))
   for offset in range(0,len(wire),1024):wrapped.sendall(wire[offset:offset+1024])
   row['response_hex']=wire.hex();plain=wrapped.unwrap();row['unwrap_completed']=True;plain.close()
  while not self.stop.is_set():
   try:extra,_=listener.accept();extra.close();raise ValueError('extra unrequested TLS import connection')
   except socket.timeout:pass

def inspect_wire(raw,rows,recovery=False):
 errors=[];joins=[]
 try:
  flows,packets=https.tls.reassemble(raw);expected=0 if recovery else 2;baseline=[f for f in flows if f['peer_port']==46078];tls_flows=sorted((f for f in flows if f['peer_port']==46080),key=lambda f:f['guest_port']);assert len(flows)==expected+1 and len(baseline)==1 and len(rows)==len(tls_flows)==expected,'unexpected/missing import TCP flow'
  echo=baseline[0];assert echo['guest_port']==48001 and echo['outgoing']==echo['incoming']==https.tls.ECHO and echo['fin_out']and echo['fin_in'],'startup Echo differs'
  for attempt,(flow,row)in enumerate(zip(tls_flows,rows),1):
   assert row['attempt']==attempt and flow['guest_port']==48001+attempt and row['handshake_complete']is True and(row['sni'],row['tls_version'],row['cipher'])==(https.tls.HOST,'TLSv1.3',https.tls.CIPHER),'actual endpoint/TLS ticket/profile differs'
   assert row['request_hex']==https.REQUEST.hex()and row['response_hex']==response(attempt).hex()and row['application_bytes_received']==94 and row['extra_application_bytes']==0 and row['unwrap_completed']is True,'actual SSL application/response/guest close_notify proof differs'
   outgoing=https.tls.tls_records(flow['outgoing']);incoming=https.tls.tls_records(flow['incoming']);client=https.tls.hello(outgoing[0],True);server=https.tls.hello(incoming[0],False)
   assert all(record['type']in(20,23)for record in outgoing[1:]+incoming[1:])and any(record['type']==23 for record in outgoing)and any(record['type']==23 for record in incoming),'post-Hello plaintext/missing encrypted records'
   for plaintext in(https.REQUEST,response(attempt),BODIES[attempt-1],b'HTTP/1.1',b'GET /welcome'):
    assert plaintext not in flow['outgoing']and plaintext not in flow['incoming'],'HTTP plaintext escaped the guest TLS flow'
   assert not flow['rst_in']and not flow['rst_out']and flow['fin_in']and flow['fin_out']and sum(r['length']for r in outgoing if r['type']==23)>=len(https.REQUEST)+17 and sum(r['length']for r in incoming if r['type']==23)>=len(response(attempt))+17,'missing complete encrypted request/response flow'
   joins.append(dict(attempt=attempt,ticket=flow['guest_port']-48000,client_hello=client,server_hello=server,initial_tls_bytes=outgoing[0]['length']+5,outgoing_bytes=len(flow['outgoing']),incoming_bytes=len(flow['incoming']),body_bytes=len(BODIES[attempt-1]),body_hash=hex(fnv(BODIES[attempt-1])),body_sha256=hashlib.sha256(BODIES[attempt-1]).hexdigest(),outgoing_records=[{k:r[k]for k in('type','version','length')}for r in outgoing],incoming_records=[{k:r[k]for k in('type','version','length')}for r in incoming]))
 except(AssertionError,ValueError,KeyError,IndexError,TypeError)as exc:errors.append(str(exc))
 return dict(ok=not errors,errors=errors,joins=joins,scope='Independent actual Ethernet/TCP/TLS framing plus genuine endpoint SSL request, distinct response and unwrap observations. Local fixture checks alone are not native proof; no TLS-key log or host forwarding.')

def workspace(disk):
 manifest=disk[2046*512:2047*512];assert len(manifest)==512 and manifest[:8]==b'SDISKMV1'and struct.unpack_from('<HH',manifest,8)==(4,15),'workspace manifest format differs';rows=[]
 for slot in range(15):
  entry=manifest[16+slot*32:48+slot*32]
  if not entry[0]:continue
  name=entry[:24].split(b'\0',1)[0];size,checksum,generation=struct.unpack_from('<IHH',entry,24);h=0x811c9dc5
  for byte in entry[:28]+entry[30:32]:h=((h^byte)*0x01000193)&0xffffffff
  assert checksum==((h^(h>>16))&0xffff)and generation>0,'workspace manifest checksum/identity differs';rows.append(dict(object=slot,name=name.decode('ascii'),size=size,identity=((slot+1)<<16)|generation,entry_sha256=hashlib.sha256(entry).hexdigest()))
 assert len({r['name']for r in rows})==len(rows),'duplicate durable workspace names';return dict(generation=struct.unpack_from('<I',manifest,12)[0],rows=rows,manifest_sha256=hashlib.sha256(manifest).hexdigest())

def import_disk(before,after,name=NAME,expected=BODY):
 old=workspace(before);new=workspace(after);matches=[r for r in new['rows']if r['name']==name];assert len(matches)==1 and not any(r['name']==name for r in old['rows']),'import did not create a fresh uniquely named object';created=matches[0];assert 3<=created['object']<15 and len(new['rows'])==len(old['rows'])+1,'import reused a pre-existing slot or created extra objects';old_by={r['object']:r for r in old['rows']};assert all(next((r for r in new['rows']if r['object']==slot),None)==row for slot,row in old_by.items()),'import changed pre-existing workspace object metadata'
 for slot in old_by:
  if slot>=2:assert object_bytes(before,slot)==object_bytes(after,slot),'import overwrote pre-existing workspace bytes'
 actual=recover(after,created['object']);assert actual['payload']==expected and actual['bytes']==len(expected)and int(actual['hash'],16)==fnv(expected)and actual['identity']==created['identity'],'typed imported journal bytes/identity differ';raw,_=object_bytes(after,created['object']);return dict(object=created['object'],name=name,identity=actual['identity'],bytes=actual['bytes'],hash=actual['hash'],generation=actual['generation'],bank=actual['bank'],raw_object_sha256=hashlib.sha256(raw).hexdigest(),workspace_count=len(new['rows']),workspace_manifest_sha256=new['manifest_sha256'])

def collision_disk(before,after,object_id):
 old=workspace(before);new=workspace(after);a,identity_a=object_bytes(before,object_id);b,identity_b=object_bytes(after,object_id);assert old==new and identity_a==identity_b and a==b,'collision overwrote/renamed/recreated journal or changed workspace metadata';return dict(unchanged=True,object=object_id,identity=identity_a,raw_object_sha256=hashlib.sha256(a).hexdigest(),workspace_manifest_sha256=old['manifest_sha256'])

def raw_document(disk,slot):
 parts=[]
 def take(offset,size):
  data=disk[offset:offset+size];assert len(data)==size,'short raw import disk evidence';parts.append(dict(offset=offset,bytes=size,hex=data.hex(),sha256=hashlib.sha256(data).hexdigest()));return data
 take(2046*512,512);descriptor=take((1925-slot)*512,512);assert descriptor[:4]==b'SFEX';count=struct.unpack_from('<H',descriptor,6)[0];assert 0<count<=8
 for index in range(count):
  start,size=struct.unpack_from('<HH',descriptor,12+index*4);assert size>0;take(start*512,size*512)
 return dict(image_bytes=len(disk),object=slot,segments=parts,scope='Actual manifest, descriptor and all object extents after acknowledged import and before any later mutation; live document snapshot, not crash/power-loss proof.')

def controls(trace,object_id,identity,length=12288):
 """Every actual import control admission has exactly one correlated reply."""
 pattern=r'\[browser.import.control\] phase=(Admitted|Reply) kind=(\d+) sequence=(\d+) object=(\d+) ok=([01]) value=(\d+)';records=list(re.finditer(pattern,trace));assert len(records)==trace.count('[browser.import.control]')==12,'missing/foreign/duplicate/malformed import control';expected=[(9,0,object_id),(1,object_id,identity),(6,object_id,0),(8,object_id,length),(5,object_id,0),(9,0,10)];pairs=[];previous=0
 for index,(kind,obj,value)in enumerate(expected):
  admitted,reply=records[index*2:index*2+2];assert admitted[1]=='Admitted'and reply[1]=='Reply','control reply/admission order differs';a_kind,seq,a_obj,ok,a_value=map(int,admitted.groups()[1:]);r_kind,r_seq,r_obj,r_ok,r_value=map(int,reply.groups()[1:]);assert(a_kind,a_obj,ok,a_value)==(kind,obj,1,0)and seq>previous and(r_kind,r_seq,r_obj)==(kind,seq,obj),'control kind/object/sequence foreign, repeated or unadmitted';assert(r_ok,r_value)==(0 if index==5 else 1,value),'control reply result/value differs';previous=seq;pairs.append(dict(kind=kind,sequence=seq,object=obj,ok=r_ok,value=r_value,admitted_offset=admitted.start(),reply_offset=reply.start()))
 return pairs

def reader_end_anchor(tile,contract):
 frame=https.http.reader_frame(BODY,tile['w'],tile['h'],contract,0,b'/welcome');return https.http.reader_move(frame,'end')

def loaded_quil_frame(trace,ppm,contract,object_id,resolution):
 """Require exact loaded object and full unique body tail despite wrapping."""
 try:
  loaded=list(re.finditer(r'\[quil.persist.load.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)',trace));assert loaded and tuple((int(loaded[-1][1]),int(loaded[-1][2],16),int(loaded[-1][3])))==(len(BODY),fnv(BODY),object_id),'Quil did not load exact imported object bytes'
  view=quil_view(trace);assert https.TAIL.decode()in ''.join(view['text'].split('\n')),'complete imported tail not visible in Quil';errors=quil_pixels(trace,ppm,contract['font'],None,target=9,resolution=resolution);assert not errors,'; '.join(errors);return dict(ok=True,errors=[],geometry=view,object=object_id,tail=https.TAIL.decode())
 except(AssertionError,ValueError,KeyError,IndexError,UnicodeError)as exc:return dict(ok=False,errors=[str(exc)])

def stream_joins(trace,raw,tickets=(2,3)):
 flows,_=https.tls.reassemble(raw);captured={f['guest_port']-48000:len(f['incoming'])for f in flows if f['peer_port']==46080};assert set(captured)==set(tickets),'TLS stream tickets differ from actual captured flows';admissions=list(re.finditer(r'\[net.job.duplex.admitted\] owner=15 task=15 ticket=(\d+) kind=10 ring=1024 tx_ring=4096 max_wire=32768 peer_port=46080 trust_profile=1 authority=held_boot_route',trace));assert [int(m[1])for m in admissions]==list(tickets)and len(admissions)==trace.count('[net.job.duplex.admitted]')and '[net.job.stream.admitted]'not in trace,'foreign/missing/duplicate held duplex admission';offset={int(m[1]):m.start()for m in admissions};rows=[];previous={};complete=set();matches=list(re.finditer(r'\[net.job.stream.drain\] owner=15 task=15 ticket=(\d+) produced=(\d+) consumed=(\d+) buffered=(\d+) ring=1024 state_before=(Receiving|Complete) receipt=retained not_delivery=1',trace));assert len(matches)==trace.count('[net.job.stream.drain]'),'malformed/foreign shared READ sample'
 for m in matches:
  ticket,produced,consumed,buffered=map(int,m.groups()[:4]);assert ticket in captured and offset[ticket]<m.start()and 0<consumed<=produced<=min(32768,captured[ticket])and buffered==produced-consumed<=1024,'READ sample lacks exact prior admission/wire/cursor bound';old=previous.get(ticket,(0,0));assert produced>=old[0]and consumed>old[1]and(ticket not in complete or m[5]=='Complete'),'READ counters/state repeated or regressed';previous[ticket]=(produced,consumed)
  if m[5]=='Complete':complete.add(ticket)
  rows.append(dict(ticket=ticket,produced=produced,consumed=consumed,buffered=buffered,state=m[5],offset=m.start()))
 assert set(previous)==set(tickets),'admitted TLS flow has no actual destructive READ sample';return rows

def source_cycles(trace,contract,host_start,host_end):
 starts=list(re.finditer(r'\[browser.command.started\] kind=TlsDns accepted=1',trace));assert len(starts)==trace.count('[browser.command.started]')==2,'missing/replayed fresh HTTPS commands';policies=list(re.finditer(r'\[trust.policy.snapshot\] owner=15 task=15 seq=(\d+) profile=1 root_bytes=376 origin=held_boot_grant',trace));sources=list(re.finditer(r'\[clock.source.snapshot\] pd=13 task=13 slot=61 seconds=(\d+) attempts=([1-4]) precision=1 profile=q35_cmos_utc ordinal=(\d+)',trace));clocks=list(re.finditer(r'\[certificate.time.snapshot\] owner=15 task=15 seq=(\d+) seconds=(\d+) source=q35_cmos_utc precision=1 receipt=retained',trace));seeds=list(re.finditer(r'\[entropy.seed.admitted\] owner=15 task=15 ticket=(\d+) bytes=32 source=held_boot_grant',trace));prepared=list(re.finditer(r'\[browser.tls.prepared\] task_bytes=(\d+) private_bytes=(\d+) read=4096 write=2048 tx_bytes=(\d+) utc_snapshot=(\d+) authenticated=0 network=0',trace));checks=list(re.finditer(r'\[browser.trust.self_check\] profile=1 passed=1 fixture=public_controlled_profile authenticated_peer=0',trace));docs=list(re.finditer(r'\[browser.https.prepared\] task_bytes=(\d+) private_bytes=(\d+) tx_bytes=(\d+) utc_snapshot=(\d+) request_bytes=94 document_limit=12288 network=0',trace));rows=[];previous=0
 for prefix,matches in(('[trust.policy.snapshot]',policies),('[clock.source.snapshot]',sources),('[certificate.time.snapshot]',clocks),('[entropy.seed.admitted]',seeds),('[browser.tls.prepared]',prepared),('[browser.trust.self_check]',checks),('[browser.https.prepared]',docs)):assert len(matches)==trace.count(prefix)==2,'missing/foreign source or fixture: '+prefix
 assert [int(m[1])for m in policies]==[int(m[1])for m in clocks]==[int(m[1])for m in seeds]==[int(m[3])for m in sources]==[1,2],'source sequence/ordinal not fresh'
 for index,(start,policy,source,clock,seed,prep,check,doc)in enumerate(zip(starts,policies,sources,clocks,seeds,prepared,checks,docs),1):
  task,private,initial,utc=map(int,prep.groups());assert 0<task<=32768 and private==task+contract['pipe_bytes']+12288 and private<49152 and 0<initial<=4096 and tuple(map(int,doc.groups()))==(task,private,initial,utc),'document/private preparation extent differs';assert utc==int(source[1])==int(clock[2])and previous<=utc and int(host_start)-1<=utc<=int(host_end)+2,'UTC source/service/host bracket differs';previous=utc
  release=re.search(rf'\[entropy.seed.released\] owner=15 task=15 ticket={index} service_payload=erased receipt=retained',trace);produced=re.search(rf'\[entropy.seed.progress\] owner=Some\(Identity \{{ pd: 15, task: 15 \}}\) ticket={index} state=Some\(Ready\) produced=32 consumed=0 failure=0 private=1',trace);assert release and produced and start.start()<policy.start()<source.start()<clock.start()<prep.start()and policy.start()<seed.start()<produced.start()<release.start()<check.start()<prep.start(),'source/positive seed cleanup ordering differs';rows.append(dict(document=True,task_bytes=task,private_bytes=private,initial_tls_bytes=initial,utc_snapshot=utc,offset=prep.start(),started_offset=start.start(),seed_release_offset=release.start()))
 assert trace.count('[entropy.seed.released]')==2,'foreign seed release';return rows

STATE_CODES={'Naming':0,'Saving':1,'Saved':2,'Exists':3,'Unavailable':4,'Cancelled':5,'Uncertain':6,'NoDocument':7,'Cleaning':8}
def source_contract(root):
 import ast
 root=Path(root);contract=https.load_contract(root);view=(root/'apps/kaleidoscope/src/import_view.rs').read_text();client=(root/'apps/kaleidoscope/src/document_import.rs').read_text();service=(root/'servers/sexfiles/src/document.rs').read_text();labels={name:ast.literal_eval('b"'+value+'"')for name,value in re.findall(r'Self::(\w+)\s*=>\s*b"((?:[^"\\]|\\.)*)"',view)};hints={}
 for alternatives,value in re.findall(r'((?:Status::\w+\s*\|\s*)*Status::\w+)\s*=>\s*add\(b"((?:[^"\\]|\\.)*)"\)',view):
  for name in re.findall(r'Status::(\w+)',alternatives):hints[name]=ast.literal_eval('b"'+value+'"')
 default=re.search(r'_\s*=>\s*add\(b"((?:[^"\\]|\\.)*)"\)',view)
 if not default or not all(state in labels and state in hints for state in('Naming','Saved','Exists'))or 'pub bytes:[u8;512]'not in view or '.min(32)'not in view:raise ValueError('source-bound standalone import renderer changed')
 if 'w::END=>'not in client or 'self.finish(View::Saved,0)'not in client or 'create_new'not in service or 'caller==15'not in service:raise ValueError('source-bound create-only import/journal client changed')
 contract['import_labels']=labels;contract['import_hints']=hints;contract['import_default_hint']=ast.literal_eval('b"'+default[1]+'"')
 for name in('scripts/https_import_gate.py','scripts/tests/test_https_import_gate.py','apps/kaleidoscope/src/document_import.rs','apps/kaleidoscope/src/import_view.rs','servers/sexfiles/src/document.rs','servers/sexfiles/src/document_journal.rs','servers/sexfiles/src/vfs.rs','servers/silk-shell/src/main.rs','servers/silk-shell/src/document_policy.rs','crates/sex-pdx/src/document.rs','crates/sex-pdx/src/document_view.rs','scripts/check_document_disk.py','scripts/check_quil_ui_pixels.py','scripts/check_desktop_pixels.py'):
  contract['source_sha256'][name]=hashlib.sha256((root/name).read_bytes()).hexdigest()
 return contract

def import_frame(tile,state,name,contract):
 assert state in STATE_CODES and state in contract['import_labels']and re.fullmatch(r'[A-Za-z0-9_.-]{1,16}',name),'unknown import status/unsafe object name';scale=2 if tile['w']>=contract['scale_threshold']else 1;cols=min(79,max(0,(tile['w']-2*contract['inset'])//(contract['advance']*scale)-1));available=tile['h']-contract['top']-7*scale;rows=min(32,512//(cols+1),1+available//(contract['line']*scale))if cols and available>=0 else 0;assert rows>0,'import state/name clipped by viewport';lines=[bytearray(b' '*cols)for _ in range(rows)];source=contract['import_labels'][state]+b'\n'+name.encode()+b'\n'+contract['import_hints'].get(state,contract['import_default_hint']);row=column=0
 for byte in source:
  if row>=rows:break
  if byte==10:row+=1;column=0;continue
  if column==cols:row+=1;column=0
  if row>=rows:break
  lines[row][column]=byte if 32<=byte<=126 else 63;column+=1
 payload=b'\n'.join(lines);assert len(payload)==rows*(cols+1)-1<=512;return dict(tile=tile,scale=scale,cols=cols,rows=rows,bytes=len(payload),hash=hex(fnv(payload)),lines=[bytes(line)for line in lines],payload=payload,state=state,name=name,state_code=STATE_CODES[state])

def import_geometry(trace,state,name,contract):
 layouts=list(re.finditer(r'\[browser.import.layout\] state=(\w+) name=([A-Za-z0-9_.-]{1,16}) bytes=(\d+) cols=(\d+) rows=(\d+) viewport=(\d+) hash=(0x[0-9a-f]+) pixels=unconfirmed',trace));assert layouts and layouts[-1].groups()[:2]==(state,name),'wrong latest import state/name layout';marker=layouts[-1];tile,viewport=https.viewport(trace);frame=import_frame(tile,state,name,contract);assert tuple(map(int,marker.groups()[2:6]))==(frame['bytes'],frame['cols'],frame['rows'],viewport)and marker[7]==frame['hash'],'source-bound full import frame/hash/viewport differs';ack=list(re.finditer(r'\[browser.import.applied\] bytes=(\d+) hash=(0x[0-9a-f]+) rows=(\d+) viewport=(\d+) state_code=(\d+) sequence=(\d+) positive_ack=1',trace[marker.end():]));assert any((int(m[1]),m[2],int(m[3]),int(m[4]),int(m[5]))==(frame['bytes'],frame['hash'],frame['rows'],viewport,frame['state_code'])and int(m[6])>0 for m in ack),'import lacks correlated immutable positive Applied';selected=1024|(frame['state_code']<<11);assert re.search(rf'\[browser.view.applied\] bytes={frame["bytes"]} http={selected} body_bytes=0 pixels=unconfirmed',trace[marker.end():]),'import lacks complete ordinary Applied';frame.update(viewport=viewport);del frame['payload'];return frame

def verify_import_pixels(trace,ppm,state,name,contract,resolution=None):
 try:return https.frame_pixels(ppm,import_geometry(trace,state,name,contract),contract,resolution)
 except(AssertionError,ValueError,KeyError,IndexError,TypeError)as exc:return dict(ok=False,errors=[str(exc)])

def inspect_import(trace,wire,dns,raw,meta,contract,host_start,host_end,recovery=False):
 errors=[];joins={}
 try:
  assert wire['ok'],'actual independent import wire evidence absent';object_id=meta['object'];identity=meta['identity'];digest=hex(fnv(BODY));loaded=list(re.finditer(r'\[quil.persist.load.ok\] bytes=12288 hash=(0x[0-9a-f]+) doc=(\d+)',trace));assert len(loaded)==1 and(loaded[0][1],int(loaded[0][2]))==(digest,object_id),'user-selected Quil import load absent/foreign'
  files=list(re.finditer(r'\[browser.import.files\] target=15 task=15 accepted=1',trace));shell=list(re.finditer(r'\[shell.document.open\] target=15 linen=200',trace));assert len(files)==trace.count('[browser.import.files]')==len(shell)==1,'O did not use actual held Shell route';opened=re.search(rf'\[linen.quil.disk_doc.intent\] object_id=\d+ path_id={object_id} ok=1',trace);assert opened and opened.start()<loaded[0].start(),'Quil load lacks real user-selected Linen disk intent'
  retired=re.search(r'\[browser.import.closed\] target=15 task=15 pending=0 accepted=0 terminal=1 owned_storage=erased',trace);assert retired,'terminal Browser import ownership cleanup absent';cleanup=cleanup_receipts(trace,15,15,retired.start());file_retire=re.search(r'\[files.document.retire\] target=15 staged=0 pending=0 terminal=1',trace);assert file_retire and retired.start()<file_retire.start()<cleanup['pool_offset'],'file import staging not retired before app pool reuse'
  if recovery:
   assert not re.search(r'\[browser.import.(?:intent|submit|control|event)\]|\[browser.https.intent\]|\[trust.policy.snapshot\]|\[net.job.duplex.admitted\]',trace),'cold reopen re-imported data or inherited network/storage mutation';assert not re.search(r'\[files.document.durable\] caller=(?:9|15) ',trace),'read-only cold recovery rewrote imported data';joins=dict(cleanup=cleanup,loaded_object=object_id);return dict(ok=True,errors=[],joins=joins,scope=SCOPE)
  assert dns['ok'],'actual DNS evidence absent';cycles=source_cycles(trace,contract,host_start,host_end);resolution=https.http.resolution_joins(trace,raw,[(2,1),(3,2)],dns);assert resolution['ok'],resolution['errors'];stream=stream_joins(trace,raw);pairs=controls(trace,object_id,identity)
  for marker in('[boot.entropy.source] holder=13 task=13 slot=60 consumer=15 consumer_task=15 source=rdseed attempts=8 runtime_mint=0','[boot.clock.source] holder=13 task=13 slot=61 consumer=15 consumer_task=15 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0','[boot.trust.policy] holder=13 task=13 slot=62 consumer=15 consumer_task=15 profile=1 root_bytes=376 runtime_mint=0','[boot.network.tls] holder=13 task=13 consumer=15 consumer_task=15 profile=1 peer_port=46080 origin=immutable_boot_profile runtime_mint=0'):assert trace.count(marker)==trace.count(marker.split(']')[0]+']')==1,'foreign/missing BOOT fixed authority'
  assert trace.count('[browser.https.intent] accepted=1 explicit=1 public_internet=0')==2,'blocked B replayed HTTPS intent';blocked=list(re.finditer(r'\[browser.command.blocked\] kind=TlsDns import_pending=1 accepted_replay=0',trace));assert len(blocked)==trace.count('[browser.command.blocked]')==1 and pairs[2]['reply_offset']<blocked[0].start()<pairs[4]['reply_offset'],'new B not actually blocked during accepted import'
  intents=list(re.finditer(r'\[browser.import.intent\] source_ticket=(\d+) bytes=12288 hash=(0x[0-9a-f]+) authenticated=1 positive_release=1 create_only=1 accepted=1',trace));submits=list(re.finditer(r'\[browser.import.submit\] name=web-note bytes=12288 hash=(0x[0-9a-f]+) source_ticket=(\d+) accepted=1 create_only=1',trace));assert trace.count('[browser.tls.cleanup]')==2,'missing/foreign actual positive network RELEASE cleanup'
  assert len(intents)==trace.count('[browser.import.intent]')==len(submits)==trace.count('[browser.import.submit]')==2,'missing/foreign/replayed explicit imports'
  for index,(flow,intent,submit,cycle)in enumerate(zip(wire['joins'],intents,submits,cycles)):
   ticket=index+2;wanted=hex(fnv(BODIES[index]));assert(int(intent[1]),intent[2],submit[1],int(submit[2]))==(ticket,wanted,wanted,ticket),'import source ticket/immutable body hash differs';release=re.search(rf'\[browser.https.released\] ticket={ticket} tx={flow["outgoing_bytes"]} rx={flow["incoming_bytes"]} request_bytes=94 body_bytes=12288 body_hash={wanted} positive_release=1 authenticated=1 public_test_root=1',trace);body=re.search(rf'\[browser.https.body\] ticket={ticket} bytes=12288 hash={wanted} parsed=1 authenticated=1 visible=0',trace);auth=re.search(rf'\[browser.https.authenticated\] ticket={ticket} profile=1 expected_name=probe.sexos.test server_finished=1 certificate=1 certificate_verify=1',trace);admit=re.search(rf'\[browser.https.admitted\] ticket={ticket} dns_generation={index+1} seq=(\d+) task_bytes={cycle["task_bytes"]} private_bytes={cycle["private_bytes"]} profile=1 authority=held_route',trace);cleanup_release=re.search(rf'\[browser.tls.cleanup\] ticket={ticket} outcome=Verified authenticated=1 document=1 tls_payload=12288 positive_release=1',trace);assert cleanup_release and body and release and body.start()<cleanup_release.start()<release.start(),'missing exact positive network RELEASE before import'
   assert all((release,body,auth,admit))and cycle['offset']<admit.start()<auth.start()<body.start()<release.start()<intent.start()<submit.start(),'import preceded exact authenticated body/positive RELEASE or held admission'
  created=list(re.finditer(r'\[files.browser.import.created\] target=15 object=(\d+) identity=(\d+) create_only=1',trace));assert len(created)==trace.count('[files.browser.import.created]')==1 and tuple(map(int,created[0].groups()))==(object_id,identity)and submits[0].start()<pairs[0]['admitted_offset']<created[0].start()<pairs[0]['reply_offset'],'CREATE did not create only the exact new object'
  durable=list(re.finditer(r'\[files.document.durable\] caller=15 object=(\d+) identity=(\d+) generation=(\d+) bank=([01]) bytes=12288 hash=(0x[0-9a-f]+) barriers=3 readback=1',trace));assert len(durable)==1 and tuple(map(int,durable[0].groups()[:4]))==(object_id,identity,meta['generation'],meta['bank'])and durable[0][5]==digest and pairs[3]['admitted_offset']<durable[0].start()<pairs[3]['reply_offset'],'actual journal commit/readback differs from independent disk'
  saved=re.search(rf'\[browser.import.event\] state=Saved object={object_id} identity={identity} bytes=12288 hash={digest} error=0 source_ticket=2',trace);exists=re.search(rf'\[browser.import.event\] state=Exists object=0 identity=0 bytes=12288 hash={hex(fnv(COLLISION_BODY))} error=10 source_ticket=3',trace);assert saved and exists and trace.count('[browser.import.event]')==2 and pairs[4]['reply_offset']<saved.start()<files[0].start()<opened.start()<loaded[0].start()<intents[1].start()<submits[1].start()<pairs[5]['admitted_offset']<pairs[5]['reply_offset']<exists.start()<retired.start(),'Saved/collision/O/reopen events lack exact positive COMMIT+END ordering'
  refresh=re.search(r'\[linen.workspace.changed\] target=15 task=15 refresh=queued read_only=1',trace);assert refresh and saved.start()<refresh.start()<opened.start(),'new import did not request independent read-only Linen refresh';assert not re.search(r'\[files.document.durable\] caller=9 ',trace),'Quil reopen mutated imported journal';joins=dict(cycles=cycles,resolution=resolution,stream=stream,controls=pairs,cleanup=cleanup,saved_offset=saved.start(),collision_offset=exists.start(),loaded_object=object_id)
 except(AssertionError,ValueError,KeyError,IndexError,TypeError,AttributeError)as exc:errors.append(str(exc))
 return dict(ok=not errors,errors=errors,joins=joins,scope=SCOPE)

def wait_ports(out):
 import errno
 checks=[];started=time.monotonic()
 while True:
  blocked=[];opened=[]
  try:
   for port,kind in((46078,socket.SOCK_STREAM),(46079,socket.SOCK_STREAM),(46080,socket.SOCK_STREAM),(46079,socket.SOCK_DGRAM)):
    sock=socket.socket(socket.AF_INET,kind);opened.append(sock)
    try:sock.bind(('127.0.0.1',port))
    except OSError as exc:
     if exc.errno!=errno.EADDRINUSE:raise
     blocked.append((port,kind))
  finally:
   for sock in opened:sock.close()
  owners=[]
  for port,kind in blocked:
   command=['lsof','-nP',('-iTCP:'if kind==socket.SOCK_STREAM else'-iUDP:')+str(port)]+(['-sTCP:LISTEN']if kind==socket.SOCK_STREAM else[]);result=subprocess.run(command,capture_output=True,text=True)
   if result.stdout.strip():owners.append(result.stdout.strip())
  checks.append(dict(elapsed_seconds=time.monotonic()-started,blocked=[dict(port=p,protocol='TCP'if k==socket.SOCK_STREAM else'UDP')for p,k in blocked],live_owners=owners));(out/'port-waits.json').write_text(json.dumps(checks,indent=2)+'\n')
  if owners:raise RuntimeError('live fixture port owner; stop before boot: '+repr(owners))
  if not blocked:return
  if time.monotonic()-started>=120:raise TimeoutError('fixture ports remain unavailable after120seconds')
  time.sleep(.5)

def run_boot(root,iso,binding,seed,out,contract,model,resolution_text,recovery=False,expected=None):
 out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh import boot evidence directory required')
 bound=frozen_binding(root,iso,binding);wait_ports(out);peer=ImportPeer(root,0 if recovery else 2);peer.start();disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-https-import-{os.getpid()}.sock';resolution=tuple(map(int,resolution_text.split('x')));command=desktop.native_command(iso,disk,serial,sock,pcap,model,resolution_text);profile=https.tls.command_profile(command)
 if not profile['ok']:raise RuntimeError(str(profile))
 host_start=time.time();manifest=dict(binding=bound,command=command,model=model,resolution=resolution_text,recovery=recovery,host_utc_start=host_start,input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),pixel_source_sha256=contract['source_sha256'],body_sha256=[hashlib.sha256(body).hexdigest()for body in BODIES],keylog_enabled=False,scope=SCOPE);(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic();failure=None;pixels=[];actions=[];meta=expected;collision=None;cleanup=None
 def text():return serial.read_text(errors='replace')if serial.exists()else''
 def health():
  if time.monotonic()-began>420:raise TimeoutError('bounded420second import boot workflow')
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if peer.errors:raise RuntimeError(str(peer.errors))
  if any(p.exists()and p.stat().st_size>MAX_PCAP for p in(serial,pcap)):raise RuntimeError('bounded raw import evidence exceeded')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',text()):raise RuntimeError('guest failure')
 def wait(pattern,start=0,timeout=90):
  end=time.monotonic()+timeout
  while time.monotonic()<end:
   health();m=re.search(pattern,text()[start:])
   if m:return m
   time.sleep(.002)
  raise TimeoutError(pattern)
 def key(code,pattern):start=len(text());q.key(code);return wait(pattern,start)
 def record(name,start,sent,**extra):actions.append(dict(action=name,serial_start=start,serial_end=len(text()),host_seconds=time.monotonic()-sent,**extra))
 def zoom(sid):
  key('grave_accent',r'\[shell.palette.item\] idx=0')
  for old,new in((0,9),(9,8)):key('k',rf'\[shell.palette.select\] old={old} new={new}')
  start=len(text());q.key('ret');wait(rf'\[shell.window.action\] action=ZoomToggle frame=\d+ sid={sid} ok=1',start)
  if sid==300:wait(r'\[browser.viewport\] target=15 task=15 sequence=\d+ w=\d+ h=\d+ origin=held_shell',start)
 def picture(name,kind,state=None,object_id=None,anchor=0,expected_name=NAME):
  path=out/(name+'.ppm');deadline=time.monotonic()+25;check=dict(ok=False,errors=['no complete frame'])
  while time.monotonic()<deadline:
   health();q.cmd('screendump',{'filename':str(path)});snapshot=text();image=path.read_bytes()
   if kind=='import':check=verify_import_pixels(snapshot,image,state,expected_name,contract,resolution)
   elif kind=='reader':check=https.verify_reader_pixels(snapshot,image,contract,anchor,resolution)
   elif kind=='quil':check=loaded_quil_frame(snapshot,image,contract,object_id,resolution)
   else:
    try:
     view,labels=final_view(snapshot);assert any(label==NAME and selected==view['selected']for selected,_,label in labels),'new name is not actual selected Linen row';errors=linen_pixels(snapshot,image,contract['font'],resolution);check=dict(ok=not errors,errors=errors,geometry=view)
    except(AssertionError,ValueError,KeyError,IndexError)as exc:check=dict(ok=False,errors=[str(exc)])
   if check['ok']:break
   time.sleep(.05)
  check.update(kind=kind,screenshot=path.name,screenshot_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),source_sha256=contract['source_sha256'],serial_end=len(snapshot),state=state,name=expected_name,object=object_id,anchor=anchor);pixels.append(check)
  if not check['ok']:raise RuntimeError(str(check['errors']))
 def fetch(ticket):
  start=len(text());sent=time.monotonic();q.key('b');wait(rf'\[browser.https.released\] ticket={ticket} tx=\d+ rx=\d+ request_bytes=94 body_bytes=12288 body_hash={hex(fnv(BODIES[ticket-2]))} positive_release=1 authenticated=1 public_test_root=1',start);record('B'+str(ticket),start,sent)
 def open_linen():
  start=len(text());sent=time.monotonic();q.key('o');wait(r'\[browser.import.files\] target=15 task=15 accepted=1',start);wait(r'\[shell.document.open\] target=15 linen=200',start);record('O_Files',start,sent);focus_surface(200,text,key)
 def select_and_open(object_id):
  for _ in range(40):
   try:
    view,labels=final_view(text())
    if any(label==NAME and identity==view['selected']for identity,_,label in labels):break
   except ValueError:pass
   key('j',r'\[shell.action.select_next_linen\] id=\d+')
  else:raise RuntimeError('actual imported Linen row never selected')
  picture('linen-imported','linen');start=len(text());sent=time.monotonic();q.key('ret');wait(r'\[linen.open.queued\] id=\d+ accepted=1',start);wait(r'\[linen.open.complete\] status=0',start);wait(rf'\[linen.quil.disk_doc.intent\] object_id=\d+ path_id={object_id} ok=1',start);wait(rf'\[quil.persist.load.ok\] bytes=12288 hash={hex(fnv(BODY))} doc={object_id}(?:\s|$)',start);record('Linen_user_Open',start,sent);focus_surface(201,text,key);zoom(201);picture('quil-imported-tail','quil',object_id=object_id);zoom(201)
 try:
  end=time.monotonic()+15
  while not sock.exists():
   health()
   if time.monotonic()>end:raise TimeoutError('QMP unavailable')
   time.sleep(.02)
  q=QMP(sock)
  if resolution_text=='1024x768':wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0');wait(r'\[linen.disk.publish.done\]');start=len(text());q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':30000}},{'type':'abs','data':{'axis':'y','value':30000}}]});wait(r'\[shell.abs.normalize\] raw_x=30000 raw_y=30000 .*reason=(?:ok|duplicate_sample)',start);focus_surface(300,text,key)
  if recovery:
   open_linen();select_and_open(meta['object']);collision=collision_disk(seed.read_bytes(),disk.read_bytes(),meta['object'])
  else:
   zoom(300);fetch(2);picture('https-initial','reader');start=len(text());tile,_=https.viewport(text());anchor=reader_end_anchor(tile,contract);q.key('end');picture('https-tail','reader',anchor=anchor);start=len(text());q.key('i');wait(r'\[browser.import.layout\] state=Naming name=web-note ',start);picture('import-default-name','import','Naming');start=len(text());q.key('backspace');wait(r'\[browser.import.layout\] state=Naming name=web-not ',start);picture('import-name-edited','import','Naming',expected_name='web-not');start=len(text());q.key('e');wait(r'\[browser.import.layout\] state=Naming name=web-note ',start);picture('import-name-restored','import','Naming');before=disk.read_bytes();(out/'before-import.img.gz').write_bytes(gzip.compress(before,mtime=0));start=len(text());sent=time.monotonic();q.key('ret');wait(r'\[browser.import.control\] phase=Reply kind=6 sequence=\d+ object=\d+ ok=1 value=0',start);blocked_start=len(text());q.key_sequence(['b']);wait(r'\[browser.command.blocked\] kind=TlsDns import_pending=1 accepted_replay=0',blocked_start);saved=wait(r'\[browser.import.event\] state=Saved object=(\d+) identity=(\d+) bytes=12288 hash='+hex(fnv(BODY))+r' error=0 source_ticket=2',start);record('I_enter_and_blocked_B',start,sent);after=disk.read_bytes();meta=import_disk(before,after);assert(meta['object'],meta['identity'])==tuple(map(int,saved.groups()));(out/'imported-raw-document.json.gz').write_bytes(gzip.compress((json.dumps(raw_document(after,meta['object']),indent=2)+'\n').encode(),mtime=0));picture('import-saved','import','Saved');zoom(300);picture('import-saved-short-tile','import','Saved');open_linen();select_and_open(meta['object']);focus_surface(300,text,key);zoom(300);fetch(3);start=len(text());q.key('i');wait(r'\[browser.import.layout\] state=Naming name=web-note ',start);picture('collision-name','import','Naming');start=len(text());sent=time.monotonic();q.key('ret');wait(r'\[browser.import.event\] state=Exists object=0 identity=0 bytes=12288 hash='+hex(fnv(COLLISION_BODY))+r' error=10 source_ticket=3',start);record('I_collision',start,sent);picture('collision-not-replaced','import','Exists');collision=collision_disk(after,disk.read_bytes(),meta['object'])
  focus_surface(300,text,key);start=len(text());sent=time.monotonic();q.key_sequence(['f11']);wait(r'\[browser.import.closed\] target=15 task=15 pending=0 accepted=0 terminal=1 owned_storage=erased',start);wait(r'\[sext.app.pool.ready\] target=15 task=15 .*ok=1',start);cleanup=cleanup_receipts(text(),15,15,start);record('Browser_Close',start,sent)
 except Exception as exc:failure=repr(exc)
 finally:
  host_end=time.time();peer.stop.set()
  for handle in peer.sockets+peer.live:
   try:handle.close()
   except OSError:pass
  for thread in peer.threads:thread.join(timeout=1)
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
  with disk.open('rb')as source,(out/'nvme.img.gz').open('wb')as destination:
   with gzip.GzipFile(fileobj=destination,mode='wb',mtime=0)as zipped:shutil.copyfileobj(source,zipped)
 manifest['host_utc_end']=host_end;manifest['final_disk_sha256']=hashlib.sha256(disk.read_bytes()).hexdigest();(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');trace=text();raw=pcap.read_bytes()if pcap.exists()else b'';wire=inspect_wire(raw,peer.rows,recovery);dns=dns_capture(raw,'echo',attempts=peer.attempts,expected=peer.dns)if not recovery and peer.dns else dict(ok=recovery,errors=[]);joined=inspect_import(trace,wire,dns,raw,meta,contract,host_start,host_end,recovery)if meta else dict(ok=False,errors=['no durable new object']);authority=nic_check(trace,expected=1,preparation=True,ethernet=True,network_config=True,routing=True);waiting=wait_check(trace);(out/'peer-exchanges.json').write_text(json.dumps(dict(tls=peer.rows,dns=peer.dns,dns_send_attempts=peer.attempts,keylog_enabled=False),indent=2)+'\n');result=dict(ok=failure is None and not peer.errors and all(c['ok']for c in(wire,dns,joined,authority,waiting)),error=failure,server_errors=peer.errors,binding=bound,wire=wire,dns=dns,import_join=joined,authority=authority,self_wait=waiting,pixels=pixels,actions=actions,document=meta,collision=collision,cleanup=cleanup,scope=SCOPE);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(dict(error=failure,joins=joined['errors'],server_errors=peer.errors)))
 return result

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',type=Path,required=True);p.add_argument('--binding',type=Path,required=True);p.add_argument('--source-disk',type=Path,required=True);p.add_argument('--out',type=Path,required=True);p.add_argument('--model',choices=('e1000','e1000e'),default='e1000e');p.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');a=p.parse_args();root=Path(__file__).resolve().parents[1];iso=a.iso.resolve(strict=True);binding=a.binding.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh import cohort directory required')
 bound=frozen_binding(root,iso,binding);held=json.loads(binding.read_text());contract=source_contract(root);target=target_check(root)
 if not target['ok']or held['runner_sha256'].get('scripts/https_import_gate.py')!=hashlib.sha256(Path(__file__).read_bytes()).hexdigest():raise RuntimeError('import checker/target not exact frozen source')
 imported=run_boot(root,iso,binding,seed,out/'import',contract,a.model,a.resolution);recovered=run_boot(root,iso,binding,out/'import/nvme.img',out/'recovery',contract,a.model,a.resolution,True,imported['document']);first=json.loads((out/'import/manifest.json').read_text());second=json.loads((out/'recovery/manifest.json').read_text());assert first['binding']==second['binding']==bound and first['final_disk_sha256']==second['input_disk_sha256'],'cold recovery did not use same ISO/exact prior terminal disk';result=dict(ok=imported['ok']and recovered['ok'],boots=2,binding=bound,document=imported['document'],write_result='import/result.json',recovery_result='recovery/result.json',same_image=True,same_disk_state=True,scope=SCOPE);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))

if __name__=='__main__':main()
