#!/usr/bin/env python3
"""Finite native Browser15→30→31 qualification; never builds an image.

Hardware launcher/Close, actual held source inventory, three startup echo flows,
local TLS preparation and complete cleanup precede each next reservation.
Each held Browser is checked for complete TLS status pixels before Zoom, after
Zoom, and after restore; matching metadata alone never proves pixels.
Quiesced mode clears with Q; allocated-close keeps the local future live until
F11, then joins owned-payload erasure and zero private heap through pool reuse.
Host monotonic intervals are observations including QMP/transport/polling delay.
The exhausted launcher is independently checked against every visible glyph.
No public TLS handshake, arbitrary restart, runtime grants, process VAS or POSIX.
"""
import argparse
# Browser private heap (sex-rt/src/heap.rs HEAP_BYTES) and the bytes it keeps
# for its whole life: the page-link table (16 x 16 B) and history (8 x 64 B).
HEAP,BASELINE=131072,768
from contextlib import contextmanager
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import threading
import time
import browser_source_evidence as sources
from clipboard_workflow_gate import complete_prefix,hardware_keys,failure_summary as safe_exception
from desktop_close_gate import QMP
from network_contention_gate import frozen_binding,MAX_PCAP

ROOT=Path(__file__).resolve().parents[1]
SERIAL_UNITS='strict UTF-8 decoded complete-scalar prefix; universal newlines; character offsets; UTF-8 prefix SHA256'
END=r'(?=\n)'
KEYS={'grave_accent':0x29,'j':0x24,'k':0x25,'ret':0x1c,'l':0x26,'q':0x10,'f11':0x57}
ECHO=bytes((17*i+78)&255 for i in range(1025))
SCOPE=__doc__.strip()
MAX_DISK=512*1024*1024
EVIDENCE_VERSION=2
MODES=('quiesced','allocated-close')
MONOTONIC_CLOCK='host time.monotonic_ns; observations include QMP, serial transport and polling'

def mode_value(mode):
 if not isinstance(mode,str)or mode not in MODES:raise ValueError('unknown explicit Browser close mode')
 return mode
def evidence_mode(manifest,result,requested):
 mode_value(requested)
 if not isinstance(manifest,dict)or not isinstance(result,dict):raise ValueError('malformed Browser evidence schema')
 if manifest.get('evidence_version')!=EVIDENCE_VERSION or result.get('evidence_version')!=EVIDENCE_VERSION or manifest.get('options')!={'mode':requested}or result.get('mode')!=requested:
  raise ValueError('Browser evidence schema/requested close mode differs; old evidence cannot be relabeled')
 return requested

def sha(raw):return hashlib.sha256(raw).hexdigest()
def serial_text(path):
 raw=Path(path).read_bytes()if Path(path).exists()else b''
 return complete_prefix(raw).decode('utf8').replace('\r\n','\n').replace('\r','\n')
def window(trace,start,end):
 if any(type(n)is not int for n in(start,end))or not 0<=start<end<=len(trace):raise ValueError('invalid fresh Browser window')
 return trace[start:end]
def one(trace,pattern,start,end):
 span=window(trace,start,end);matches=list(re.finditer(pattern+END,span))
 if len(matches)!=1:raise ValueError('missing/duplicate exact Browser receipt: '+pattern)
 return matches[0]
def browser_rows(root):
 plans=sources.plans_from_catalog(root)
 if plans!=sources.FUTURE_PLANS:raise ValueError('native reserve requires actual three-row Browser catalog')
 return [dict(target=p.target,surface=p.surface,lifecycle=p.lifecycle,entropy=p.entropy,clock=p.clock,trust=p.trust,
              ordinal=i,predecessor=0 if i==0 else plans[i-1].target,activation=None if i==0 else(156,160)[i-1])for i,p in enumerate(plans)]
def tls_accounting(root):
 """Match Session::Stats task/Pipe accounting, including Pipe's metadata.

 L calls Probe::start(false,None), so Session::prepare owns no Document.
 This is reported payload accounting, not allocator bookkeeping.
 """
 root=Path(root);backend=(root/'crates/sex-tls/src/lib.rs').read_text()
 code=re.sub(r'\s+','',re.sub(r'//[^\n]*','',backend));values={}
 for name,expected in [('RECORD_READ',4096),('RECORD_WRITE',2048),('QUEUE_BYTES',4096),('MAX_TASK_BYTES',32768),('PIPE_METADATA_BYTES',56)]:
  found=re.findall(r'pub const '+name+r'\s*:\s*usize\s*=\s*([0-9_]+)\s*;',backend)
  if len(found)!=1 or int(found[0].replace('_',''))!=expected:raise ValueError('unreviewed source TLS accounting: '+name)
  values[name]=expected
 for fragment in ('pubconstPIPE_BYTES:usize=2*QUEUE_BYTES+PIPE_METADATA_BYTES;',
                  'const_:()=assert!(core::mem::size_of::<Pipe>()==PIPE_BYTES);',
                  'private_bytes:task.layout.size()+core::mem::size_of::<Pipe>()+ifresource.is_some(){MAX_DOCUMENT}else{0}',
                  'pubfnowned_payload_bytes(&self)->usize{self.stats.task_bytes+2*QUEUE_BYTES+ifself.resource.is_some(){MAX_DOCUMENT}else{0}}'):
  if fragment not in code:raise ValueError('source Session::Stats/Pipe formula changed')
 probe=re.sub(r'\s+','',(root/'apps/kaleidoscope/src/tls_probe.rs').read_text())
 if 'self.start_mode(policy,seed,time,now,false,None)'not in probe or 'Session::prepare(&mutbytes,seconds,profile)'not in probe:
  raise ValueError('local TLS preparation resource accounting changed')
 return dict(record_read=values['RECORD_READ'],record_write=values['RECORD_WRITE'],queue_bytes=values['QUEUE_BYTES'],
             pipe_metadata_bytes=values['PIPE_METADATA_BYTES'],pipe_bytes=2*values['QUEUE_BYTES']+values['PIPE_METADATA_BYTES'],
             queue_payload_bytes=2*values['QUEUE_BYTES'],max_task_bytes=values['MAX_TASK_BYTES'],max_private_bytes=49152)
def source_contract(root):
 from https_asset_gate import script_dependencies
 root=Path(root)
 paths=('crates/sex-pdx/src/app_catalog.rs','crates/sex-pdx/src/browser_sources.rs','crates/sex-pdx/src/app_lifecycle.rs',
 'kernel/src/init.rs','kernel/src/app_resources.rs','kernel/src/entropy_source.rs','kernel/src/clock_source.rs',
 'servers/sexnet/src/main.rs','servers/sexnet/src/source_binding.rs','servers/sexnet/src/entropy.rs','servers/sexnet/src/certificate_time.rs',
 'servers/silk-shell/src/main.rs','servers/silk-shell/src/launcher.rs','servers/sexdisplay/src/main.rs',
 'apps/kaleidoscope/src/main.rs','apps/kaleidoscope/src/tls_probe.rs','apps/kaleidoscope/src/network_probe.rs','crates/sex-tls/src/lib.rs',
 'scripts/browser_reserve_gate.py','scripts/tests/test_browser_reserve_gate.py','scripts/browser_source_evidence.py',
 'scripts/tests/test_browser_source_evidence.py')
 paths=set(paths)|set(script_dependencies(root,('scripts/browser_reserve_gate.py','scripts/tests/test_browser_reserve_gate.py')))
 from tls_prepare_gate import tls_contract
 paths.update(tls_contract(root)['source_sha256'])
 return dict(rows=browser_rows(root),tls_accounting=tls_accounting(root),source_sha256={name:sha((root/name).read_bytes())for name in sorted(paths)})
def require_inputs(binding,contract):
 for name,digest in contract['source_sha256'].items():
  if binding.get('sources',{}).get(name,binding.get('runner_sha256',{}).get(name))!=digest:raise ValueError('source/checker missing from frozen cohort: '+name)

def boot_order(trace,root):
 declared=re.search(r'pub const INCARNATIONS:\s*\[Incarnation;\s*(\d+)\]',(Path(root)/'crates/sex-pdx/src/app_catalog.rs').read_text())
 if declared is None:raise ValueError('missing source catalog bound')
 catalog=sources.records(trace,'boot.app.catalog');complete=sources.records(trace,'boot.app.catalog.complete')
 if len(catalog)!=int(declared[1])or len(complete)!=1:raise ValueError('incomplete actual BOOT catalog')
 sources.exact(complete[0][1],dict(rows=int(declared[1]),actual_grants=1,runtime_mint=0))
 tasks=[sources.number(f,'task',2**32-1)for _,f in catalog]
 if 0 in tasks or len(set(tasks))!=len(tasks):raise ValueError('duplicate/zero BOOT task')
 seals=list(re.finditer(r'\[boot.memory.authority.sealed\] runtime_alloc=0 runtime_map=capability_leaf runtime_mint=0'+END,trace))
 if len(seals)!=1 or trace.count('[boot.memory.authority.sealed]')!=1:raise ValueError('missing/duplicate/malformed BOOT seal')
 handoffs=list(re.finditer(r'\[bootgraph.boot.handoff\] target=[A-Za-z0-9_-]+ id=\d+ entry=0x[0-9a-fA-F]+'+END,trace))
 readbacks=sources.records(trace,'boot.browser.sources.readback');bindings=sources.records(trace,'net.source.binding.held')
 if not handoffs or not readbacks or not bindings or not max(p for p,_ in catalog)<complete[0][0]<seals[0].start()<handoffs[0].start() or max(p for p,_ in readbacks)>=seals[0].start()or min(p for p,_ in bindings)<=handoffs[0].start():raise ValueError('catalog/readback/seal/runtime binding order differs')

def cleanup(trace,row,start,end):
 t,k,s=row['target'],row['task'],row['surface'];span=window(trace,start,end)
 requested=one(trace,rf'\[shell.app.close.request\] target={t} task={k} surface={s}',start,end)
 finished=one(trace,rf'\[task.finish\] pd={t} task={k} reason=0 terminal=1',start,end)
 mask=0;roles={};positions=[]
 for at,fields in sources.records(span,'app.service.ack'):
  if sources.number(fields,'target')!=t or sources.number(fields,'task')!=k:continue
  if set(fields)!={'target','task','holder','role','acked','required'}:raise ValueError('unexpected cleanup ACK fields')
  holder,role,acked,required=(sources.number(fields,key)for key in('holder','role','acked','required'))
  if role in roles or {1:3,2:1,4:11,8:13,16:10}.get(role)!=holder or required!=31 or acked!=mask|role:raise ValueError('cleanup role/holder/accumulated mask differs')
  roles[role]=holder;mask=acked;positions.append(at)
 if set(roles)!={1,2,4,8,16}or mask!=31:raise ValueError('incomplete exact five-role cleanup')
 retired=one(trace,rf'\[app.private.retired\] target={t} task={k} pages=([1-9]\d*) bytes=([1-9]\d*) acked=31 required=31 all_flushed=1 pool_reusable=1 messages=\d+ signals=\d+ metadata=retained runtime_alloc=0',start,end)
 pool=one(trace,rf'\[sext.app.pool.ready\] target={t} task={k} bytes=([1-9]\d*) zeroed=([1-9]\d*) workspace_records=\d+ reused=1 zero_owner=sext ok=1',start,end)
 size=int(pool[1])
 if size%4096 or size!=int(pool[2])or size!=int(retired[2])or size!=int(retired[1])*4096:raise ValueError('retired/pool bytes or exact erasure differs')
 if not requested.start()<finished.start()<min(positions)<=max(positions)<retired.start()<pool.start():raise ValueError('Close/finish/ACK/retirement/reuse order differs')
 for tag,suffix in [('net.dns.records.retired','remaining=0 before_cleanup_ack=1'),('entropy.seed.retired','before_cleanup_ack=1 private_payload=erased'),('certificate.time.retired','before_cleanup_ack=1 high_water_retained=1'),('trust.policy.retired','before_cleanup_ack=1')]:
  effect=one(trace,rf'\[{tag}\] target={t} task={k} {suffix}',start,end)
  if effect.start()>=positions[[r for r in roles].index(8)]:raise ValueError('source retirement after Net cleanup ACK')
 return dict(target=t,task=k,bytes=size,close_offset=start+requested.start(),finish_offset=start+finished.start(),pool_offset=start+pool.start(),roles=sorted(roles))

def workload(trace,row,start,end,inventory,accounting):
 t,k=row['target'],row['task'];window(trace,start,end)
 command=one(trace,r'\[browser.command.started\] kind=TlsPrepare accepted=1',start,end)
 policy=one(trace,rf'\[trust.policy.snapshot\] owner={t} task={k} seq=([1-9]\d*) profile=1 root_bytes=376 origin=held_boot_grant',start,end)
 clock=one(trace,rf'\[certificate.time.snapshot\] owner={t} task={k} seq=([1-9]\d*) seconds=(\d+) source=q35_cmos_utc precision=1 receipt=retained',start,end)
 admitted=one(trace,rf'\[entropy.seed.admitted\] owner={t} task={k} ticket=([1-9]\d*) bytes=32 source=held_boot_grant',start,end);ticket=int(admitted[1])
 ready=one(trace,rf'\[entropy.seed.progress\] owner=Some\(Identity \{{ pd: {t}, task: {k} \}}\) ticket={ticket} state=Some\(Ready\) produced=32 consumed=0 failure=0 private=1',start,end)
 release=one(trace,rf'\[entropy.seed.released\] owner={t} task={k} ticket={ticket} service_payload=erased receipt=retained',start,end)
 prepared=one(trace,r'\[browser.tls.prepared\] task_bytes=(\d+) private_bytes=(\d+) read=4096 write=2048 tx_bytes=(\d+) utc_snapshot=(\d+) authenticated=0 network=0',start,end)
 if not 0<int(prepared[1])<=accounting['max_task_bytes'] or int(prepared[2])!=int(prepared[1])+accounting['pipe_bytes'] or int(prepared[2])>=accounting['max_private_bytes'] or not 0<int(prepared[3])<=accounting['queue_bytes'] or int(prepared[4])!=int(clock[2]):raise ValueError('local TLS bounds/source UTC differ')
 source=[r for r in sources.validate_clock_stream(trace,inventory)if r['consumer']==(t,k)and start<=r['offset']<end]
 if len(source)!=1 or source[0]['holder'][2]!=row['clock']or source[0]['seconds']!=int(clock[2]):raise ValueError('exact source/consumer clock join missing')
 if not command.start()<policy.start()<clock.start()<prepared.start()or not policy.start()<admitted.start()<ready.start()<release.start()<prepared.start()or not start+policy.start()<source[0]['offset']<start+clock.start():raise ValueError('typed source/preparation order differs')
 return dict(target=t,task=k,seed_ticket=ticket,utc=int(clock[2]),clock_source_offset=source[0]['offset'],prepared_offset=start+prepared.start(),task_bytes=int(prepared[1]),private_bytes=int(prepared[2]),owned_payload_bytes=int(prepared[1])+accounting['queue_payload_bytes'],authenticated_peer=False,network=False)

def resource_rows(trace,start,end):
 rows=[]
 pattern=r'\[browser.resources\] reason=(command|change|clear|close) heap_used=(\d+) heap_free=(\d+) heap_total=(\d+) tls_payload=(\d+) tls_pending=([01]) route_busy=([01]) providers_busy=([01]) queued=(None|Some\(\w+\)) body_visible=([01])\n'
 span=window(trace,start,end)
 matches=list(re.finditer(r'\[browser.resources\][^\n]*(?:\n|$)',span))
 if len(matches)!=span.count('[browser.resources]'):raise ValueError('malformed Browser resource marker')
 for match in matches:
  exact=re.fullmatch(pattern,match[0])
  if exact is None:raise ValueError('malformed/incomplete Browser resource receipt')
  reason,used,free,total,payload,pending,route,providers,queued,body=exact.groups()
  values=dict(used=int(used),free=int(free),total=int(total),payload=int(payload),pending=int(pending),route=int(route),providers=int(providers),body=int(body))
  if values['total']!=HEAP or values['used']+values['free']!=values['total']:raise ValueError('Browser private heap accounting differs')
  rows.append(dict(values,reason=reason,queued=queued,offset=start+match.start(),end=start+match.end()))
 return rows

def allocated_close_evidence(trace,row,step,prepared):
 """Local future custody only; source seed RELEASE already preceded Hello.

 Anonymous TLS/resource records are admitted only in this exact held subject's
 sequential phase, fenced by its typed Close, tagged client closes and finish.
 They do not supply authority or prove pending transport/device I/O.
 """
 start,cut,end=(step[k]for k in('work_start','close_start','end'));t,k,s=(row[x]for x in('target','task','surface'))
 prior=window(trace,prepared['prepared_offset'],cut)
 views=list(re.finditer(r'\[browser.tls.view\] state=(\w+) authenticated=0 network=0'+END,prior))
 if len(views)!=1 or prior.count('[browser.tls.view]')!=1 or any(m[1]!='HelloPending'for m in views):raise ValueError('allocated Close lost fresh HelloPending')
 forbidden=r'\[browser.tls\.(?:erased|closed|transfer\w*)\]|\[browser\.(?:https.prepared|tls.peer)|\[browser.command\.(?:queued|started)\]|\[sexinput.ps2.scancode\] raw=0x10'+END
 if re.search(forbidden,prior):raise ValueError('allocated Close followed Q/cancel/timeout/reprepare/transfer/erase')
 resources=resource_rows(trace,start,end);before=[r for r in resources if r['offset']<cut]
 if not before:raise ValueError('missing live allocated TLS resource receipt')
 live=before[-1];payload=prepared['owned_payload_bytes']
 if live['offset']<=prepared['prepared_offset']or live['reason']!='change'or not 0<payload<=live['used']<=HEAP or (live['payload'],live['pending'],live['route'],live['providers'],live['queued'],live['body'])!=(payload,1,0,0,'None',0):raise ValueError('latest TLS resource snapshot is stale/quiesced/not allocated')
 request=one(trace,rf'\[shell.app.close.request\] target={t} task={k} surface={s}',cut,end)
 erased=one(trace,r'\[browser.tls.erased\] bytes=(\d+) owned_storage=1',cut,end)
 closed=one(trace,r'\[browser.tls.closed\] pending=1 terminal=1 owned_storage=erased',cut,end)
 finish=one(trace,rf'\[task.finish\] pd={t} task={k} reason=0 terminal=1',cut,end)
 span=window(trace,cut,end)
 if span.count('[browser.tls.erased]')!=1 or span.count('[browser.tls.closed]')!=1 or int(erased[1])!=payload or payload<=0:raise ValueError('allocated Close missing exact nonzero owned-payload erasure')
 bindings=[]
 for tag,suffix in(('trust',''),('clock',''),('entropy',' private_payload=erased')):
  binding=one(trace,rf'\[browser.{tag}.closed\] target={t} task={k} pending=0 terminal=1{suffix}',cut,end);bindings.append(cut+binding.start())
 clipboard=one(trace,rf'\[browser.clipboard.closed\] target={t} task={k} owned_scratch=erased terminal=1',cut,end)
 after=[r for r in resources if r['offset']>=cut]
 if len(after)!=1:raise ValueError('missing/duplicate/stale terminal close resource receipt')
 zero=after[0]
 if (zero['reason'],zero['used'],zero['free'],zero['payload'],zero['pending'],zero['route'],zero['providers'],zero['queued'],zero['body'])!=('close',BASELINE,HEAP-BASELINE,0,0,0,0,'None',0):raise ValueError('terminal allocated Close did not free isolated private heap/payload')
 offsets=dict(request=cut+request.start(),erased=cut+erased.start(),closed=cut+closed.start(),clipboard=cut+clipboard.start(),resources=zero['offset'],finish=cut+finish.start())
 if not offsets['request']<offsets['erased']<offsets['closed']<min(bindings)<=max(bindings)<offsets['clipboard']<offsets['resources']<offsets['finish']:raise ValueError('allocated Close/drop/client-close/resources/finish order differs')
 if re.search(r'\[sexinput.ps2.scancode\] raw=0x10'+END,window(trace,start,end)):raise ValueError('allocated Close contains actual Q make')
 return dict(mode='allocated-close',target=t,task=k,surface=s,task_bytes=prepared['task_bytes'],private_bytes=prepared['private_bytes'],erased_payload_bytes=payload,live_resources=live,zero_resources=zero,offsets=offsets,scope='retained local TLSfuture/owned queues; not pending transport/device I/O or register/cache erasure')

def startup_echo(trace,row,start,end):
 from check_tcp_capture import fingerprint
 t,k=row['target'],row['task'];span=window(trace,start,end)
 tickets={int(m[1])for m in re.finditer(rf'\[net.job.state\] owner=Some\(Identity \{{ pd: {t}, task: {k} \}}\) ticket=([1-9]\d*) ',span)}
 if len(tickets)!=1:raise ValueError('startup echo lacks one exact held-owner ticket')
 ticket=tickets.pop()
 body=one(trace,rf'\[browser.net.job.bytes\] ticket={ticket} length=1025 hash={hex(fingerprint(ECHO))} exact=1 endpoint=controlled_peer tls=0 web=0',start,end)
 released=one(trace,rf'\[browser.net.job.released\] ticket={ticket} view=Verified private_job=0',start,end)
 if body.start()>=released.start():raise ValueError('startup echo verified before positive release')
 return ticket

def verify(trace,result,root,mode='quiesced'):
 mode_value(mode)
 if result.get('evidence_version')!=EVIDENCE_VERSION or result.get('mode')!=mode:raise ValueError('explicit Browser result schema/close mode differs')
 rows=browser_rows(root);inv=sources.validate_inventory(trace,sources.plans_from_catalog(root),require_version=True)
 accounting=tls_accounting(root)
 boot_order(trace,root)
 sources.validate_clock_stream(trace,inv);sources.validate_entropy_stream(trace,inv)
 chain=result.get('chain')
 if not isinstance(chain,list)or len(chain)!=3:raise ValueError('missing/skipped Browser incarnation')
 previous=0;joins=[]
 for expected,step in zip(rows,chain):
  if not isinstance(step,dict):raise ValueError('malformed Browser phase')
  row=dict(expected,task=inv['roster'][expected['target']]['task'])
  if step.get('identity')!=row:raise ValueError('Browser catalog/held task differs')
  a,b,c,d=(step.get(key)for key in('start','work_start','close_start','end'))
  if any(type(n)is not int for n in(a,b,c,d))or not previous<=a<b<c<d<=len(trace):raise ValueError('overlapping/stale Browser phase windows')
  t,k,s=row['target'],row['task'],row['surface']
  if row['ordinal']:
   activated=one(trace,rf'\[app.activation.committed\] logical=15 incarnation={row["ordinal"]+1} predecessor={row["predecessor"]} successor={t} task={k} surface={s} pages=[1-9]\d* runtime_alloc=0 runtime_mint=0 bsp=1',a,b)
   relaunched=one(trace,rf'\[shell.app.relaunch\] predecessor={row["predecessor"]} successor={t} task={k} surface={s} receipt_replayed=1',a,b)
   if activated.start()>=relaunched.start():raise ValueError('UI relaunch before exact kernel commit')
  one(trace,rf'\[shell.app.registered.reply\] target={t} task={k} surface={s} committed=1',a,b)
  if not re.search(rf'\[shell.focus.set\] id={s}'+END,window(trace,a,b)):raise ValueError('missing current exact Browser focus')
  one(trace,rf'\[app.surface.presented\] target={t} surface={s} sequence=1 ok=1',a,b)
  ticket=startup_echo(trace,row,a,b);proof=workload(trace,row,b,c,inv,accounting);closed=cleanup(trace,row,c,d)
  pending=allocated_close_evidence(trace,row,step,proof)if mode=='allocated-close'else None
  joins.append(dict(identity=row,echo_ticket=ticket,workload=proof,cleanup=closed,close_evidence=pending));previous=d
 start,end=result.get('exhaustion_start'),result.get('exhaustion_end');span=window(trace,start,end)
 if start<previous:raise ValueError('exhaustion precedes final cleanup')
 if not re.search(r'\[launcher.text.row\] index=10 row=\d+ name=Open Browser status=CLOSED / RESERVE EXHAUSTED'+END,span):raise ValueError('missing current exhausted Browser row')
 one(trace,r'\[launcher.execute.denied\] idx=10 status=CLOSED / RESERVE EXHAUSTED reason=boot_reserve_exhausted',start,end)
 commits=list(re.finditer(r'\[app.activation.committed\] logical=15 [^\n]*'+END,trace))
 if len(commits)!=2 or re.search(r'\[app.activation.committed\] logical=15 ',trace[start:]):raise ValueError('extra/replayed activation or activation after exhaustion')
 return dict(ok=True,evidence_version=EVIDENCE_VERSION,mode=mode,targets=[r['target']for r in rows],joins=joins,reserve_exhausted=True,browser_pixels_qualified=False,scope=SCOPE)


def palette(c):
 c['key']('grave_accent',r'\[shell.palette.open\] ok=1 selected=0 count=11'+END)
 for old in range(10):c['key']('j',rf'\[shell.palette.select\] old={old} new={old+1}'+END)
def launch(c,row):
 """Use index10 and return the fresh launch start; row includes actual held task."""
 start=0 if row['ordinal']==0 else len(c['text']())
 palette(c);c['key']('ret',rf'\[shell.palette.exec\] idx=10 action=Open Browser ok=1 reason=ok'+END)
 for pattern in[rf'\[shell.app.registered.reply\] target={row["target"]} task={row["task"]} surface={row["surface"]} committed=1'+END,
                rf'\[app.surface.presented\] target={row["target"]} surface={row["surface"]} sequence=1 ok=1'+END,
                rf'\[shell.focus.set\] id={row["surface"]}'+END]:c['wait'](pattern,start)
 return start
def close(c,row,mode='quiesced'):
 """Exact focused incarnation F11; no next launch until real all-role/pool fence."""
 mode_value(mode)
 start=len(c['text']());c['key']('f11',rf'\[shell.app.close.request\] target={row["target"]} task={row["task"]} surface={row["surface"]}'+END)
 key_index=len(c['actions'])-1 if 'actions'in c else None
 pool=c['wait'](rf'\[sext.app.pool.ready\] target={row["target"]} task={row["task"]} bytes=\d+ zeroed=\d+ workspace_records=\d+ reused=1 zero_owner=sext ok=1'+END,start)
 observed_ns=time.monotonic_ns();trace=c['text']();end=len(trace);checked=cleanup(trace,row,start,end)
 if mode=='allocated-close'and 'actions'in c:
  key=c['actions'][key_index]
  if key.get('kind')!='key'or key.get('key')!='f11':raise ValueError('Close lacks actual retained F11 observation')
  c['actions'].append(dict(kind='browser_close',mode=mode,target=row['target'],task=row['task'],surface=row['surface'],key_index=key_index,
   start=start,end=end,serial_sha256=sha(trace[:end].encode()),request_offset=checked['close_offset'],pool_offset=start+pool.start(),pool_end=start+pool.end(),
   issued_ns=key['issued_ns'],request_observed_ns=key['receipt_observed_ns'],pool_observed_ns=observed_ns,
   key_to_request_ns=key['receipt_observed_ns']-key['issued_ns'],key_to_pool_ns=observed_ns-key['issued_ns']))
 elif mode=='allocated-close':raise ValueError('allocated Close needs retained hardware/timing actions')
 return start,end

def run(c,mode='quiesced'):
 """Reusable operation context: root,text,wait,key,capture; caller owns QEMU/peer.

 key(code,pattern) must perform real hardware down/up plus a release barrier;
 capture(name,kind,target,expected) retains immutable raw PPM+serial prefixes.
 Caller must arrange the normal Browser startup echo peer for all3 identities.
 """
 mode_value(mode)
 if mode=='allocated-close'and not isinstance(c.get('actions'),list):raise ValueError('allocated Close needs shared retained actions')
 root=Path(c['root']);inv=sources.validate_inventory(c['text'](),sources.plans_from_catalog(root),require_version=True)
 chain=[]
 for expected in browser_rows(root):
  row=dict(expected,task=inv['roster'][expected['target']]['task']);start=launch(c,row)
  # The automatic real fixed-peer echo must settle before local preparation.
  c['wait'](r'\[browser.net.job.released\] ticket=\d+ view=Verified private_job=0'+END,start)
  work_start=len(c['text']());c['key']('l',r'\[browser.command.started\] kind=TlsPrepare accepted=1'+END)
  c['wait'](r'\[browser.tls.view\] state=HelloPending authenticated=0 network=0'+END,work_start)
  c['wait'](r'\[browser.tls.prepared\] [^\n]* authenticated=0 network=0'+END,work_start)
  paint_starts=[work_start]
  def painted(name,begin,viewport=None):
   stamp=rf'\[browser.tls.applied\] target={row["target"]} task={row["task"]} surface={row["surface"]} viewport='+(str(viewport)if viewport is not None else r'\d+')
   c['wait'](stamp+r' [^\r\n]*state_code=2 complete=1 sequence=\d+ positive_ack=1 pixels=unconfirmed'+END,begin)
   return c['capture'](name,kind='browser_tls',target=row['target'],expected='HelloPending',window_start=begin)
  painted(f'browser-{row["target"]}-prepared',work_start)
  from quil_workflow_gate import palette_commands,palette_select
  commands=palette_commands(root)
  for phase in ('expanded','restored'):
   begin=len(c['text']());palette_select(c['key'],'ZoomToggle',commands)
   c['key']('ret',rf'\[shell.window.action\] action=ZoomToggle frame=\d+ sid={row["surface"]} ok=1 reason=ok'+END)
   viewport=c['wait'](rf'\[browser.viewport\] target={row["target"]} task={row["task"]} sequence=(\d+) w=\d+ h=\d+ origin=held_shell'+END,begin)
   paint_starts.append(begin);painted(f'browser-{row["target"]}-{phase}',begin,int(viewport[1]))
  if mode=='quiesced':
   c['key']('q',r'\[browser.tls.view\] state=Cancelled authenticated=0 network=0'+END)
   c['wait'](rf'\[browser.resources\] reason=clear heap_used={BASELINE} heap_free={HEAP-BASELINE} heap_total={HEAP} tls_payload=0 [^\n]*'+END,work_start)
  else:
   # Check the current snapshot immediately before the hardware request, not
   # merely peak preparation statistics or a preceding screenshot.
   now=c['text']();prepared=workload(now,row,work_start,len(now),inv,tls_accounting(root))
   live=resource_rows(now,work_start,len(now))
   if not live or live[-1]['payload']!=prepared['owned_payload_bytes']or live[-1]['pending']!=1:raise ValueError('TLS allocation expired/quiesced before physical Close')
  close_start,end=close(c,row,mode);step=dict(identity=row,start=start,work_start=work_start,close_start=close_start,end=end,paint_starts=paint_starts)
  if mode=='allocated-close':step['close_observation_index']=len(c['actions'])-1
  chain.append(step)
 start=len(c['text']());palette(c);c['wait'](r'\[launcher.text.row\] index=10 row=\d+ name=Open Browser status=CLOSED / RESERVE EXHAUSTED'+END,start)
 c['capture']('browser-reserve-exhausted',kind='launcher',target=15,expected='Open Browser')
 c['key']('ret',r'\[launcher.execute.denied\] idx=10 status=CLOSED / RESERVE EXHAUSTED reason=boot_reserve_exhausted'+END)
 result=dict(evidence_version=EVIDENCE_VERSION,mode=mode,chain=chain,exhaustion_start=start,exhaustion_end=len(c['text']()))
 checked=verify(c['text'](),result,root,mode)
 if mode=='allocated-close':result['close_evidence']=[j['close_evidence']for j in checked['joins']]
 return result

@contextmanager
def echo_peer():
 listener=socket.socket();listener.bind(('127.0.0.1',46078));listener.listen(3);listener.settimeout(.1)
 stop=threading.Event();rows=[];errors=[];live=[]
 def serve():
  try:
   while not stop.is_set():
    try:conn,_=listener.accept()
    except socket.timeout:continue
    live.append(conn);conn.settimeout(30)
    if len(rows)>=3:conn.close();raise ValueError('extra startup echo connection')
    data=bytearray()
    while True:
     part=conn.recv(2048)
     if not part:break
     data.extend(part)
     if len(data)>len(ECHO):raise ValueError('oversized startup echo')
    if bytes(data)!=ECHO:raise ValueError('wrong startup echo payload')
    conn.sendall(ECHO);conn.shutdown(socket.SHUT_WR);conn.close()
    rows.append(dict(payload_hex=bytes(data).hex(),response_hex=ECHO.hex(),received=len(data),sent=len(ECHO)))
  except Exception as exc:
   if not stop.is_set():errors.append(safe_exception(exc))
 worker=threading.Thread(target=serve,daemon=True);worker.start()
 try:yield dict(rows=rows,errors=errors)
 finally:
  stop.set();listener.close()
  for conn in live:
   try:conn.shutdown(socket.SHUT_RDWR)
   except OSError:pass
   conn.close()
  worker.join(timeout=2)
  if worker.is_alive():errors.append('startup echo worker did not stop')

def verify_keys(trace,actions):
 previous=0;count=0
 for action in actions:
  if action.get('kind')!='key':continue
  start,end=action.get('start'),action.get('end');span=window(trace,start,end);key=action.get('key')
  if key not in KEYS or start<previous or action.get('transport')!='input-send-event':raise ValueError('key identity/order/transport differs')
  codes=[int(m[1],16)for m in re.finditer(r'\[sexinput.ps2.scancode\] raw=(0x[0-9a-f]+)'+END,span)]
  if [code for code in codes if code<128]!=[KEYS[key]]or codes.count(KEYS[key]|128)!=1 or codes.index(KEYS[key])>=codes.index(KEYS[key]|128):raise ValueError('missing/extra/reversed physical make/release barrier')
  previous=end;count+=1
 if not count:raise ValueError('missing actual hardware input')
 return dict(ok=True,hardware_keys=count)

def close_observations(trace,result,actions,proof,manifest):
 """Replay host observations against their exact same-run raw serial cuts.

 These intervals include QMP and polling delay. They are neither guest event
 timestamps nor a universal scheduler/cryptographic latency guarantee.
 """
 if proof.get('mode')!='allocated-close':return dict(scope='quiesced mode; no allocated-close timing claim',rows=[])
 def integer(value):return type(value)is int
 if not isinstance(actions,list)or any(not isinstance(action,dict)for action in actions):raise ValueError('malformed retained hardware observations')
 lower,upper=(manifest.get(k)for k in('host_monotonic_start_ns','host_monotonic_end_ns'))
 if manifest.get('monotonic_clock')!=MONOTONIC_CLOCK or not integer(lower)or not integer(upper)or not 0<lower<=upper:raise ValueError('invalid same-run host monotonic observation bracket')
 def cut(end,digest):
  if not integer(end)or not 0<=end<=len(trace)or digest!=sha(trace[:end].encode()):raise ValueError('changed/out-of-bounds host observation serial prefix')
 rows=[];used_keys=[];used_observations=[];previous=lower
 for step,join in zip(result['chain'],proof['joins']):
  index=step.get('close_observation_index')
  if not integer(index)or not 0<=index<len(actions):raise ValueError('missing exact Close observation action')
  observation=actions[index];key_index=observation.get('key_index')
  if not integer(key_index)or not 0<=key_index<index:raise ValueError('invalid Close hardware action reference')
  key=actions[key_index];row=step['identity'];effect=join['close_evidence'];offsets=effect['offsets'];target,task,surface=(row[k]for k in('target','task','surface'))
  if observation.get('kind')!='browser_close'or observation.get('mode')!='allocated-close'or (observation.get('target'),observation.get('task'),observation.get('surface'))!=(target,task,surface):raise ValueError('Close observation subject/mode differs')
  if key.get('kind')!='key'or key.get('key')!='f11'or any(not integer(key.get(k))for k in('start','end','receipt_offset','receipt_end','receipt_serial_end'))or not step['close_start']<=key['start']<key['end']<=step['end']:raise ValueError('Close lacks exact in-phase physical F11 action')
  verify_keys(trace,[key])
  expected=[dict(execute='input-send-event',arguments=dict(events=[dict(type='key',data=dict(down=down,key=dict(type='qcode',data='f11')))for down in(True,False)]))]
  if key.get('qmp')!=expected:raise ValueError('Close hardware command differs from actual explicit down/up packet')
  if key.get('receipt_pattern')!=rf'\[shell.app.close.request\] target={target} task={task} surface={surface}'+END:raise ValueError('Close key observed a different receipt')
  request_end=trace.index('\n',offsets['request']);pool=join['cleanup']['pool_offset'];pool_end=trace.index('\n',pool)
  if key.get('receipt_offset')!=offsets['request']or key.get('receipt_end')!=request_end or not key['start']<=key['receipt_offset']<key['receipt_end']<key['receipt_serial_end']<=key['end']:raise ValueError('F11 lacks the exact complete Close-request observation')
  cut(key['start'],key.get('start_sha256'));cut(key['receipt_serial_end'],key.get('receipt_serial_sha256'));cut(key['end'],key.get('end_sha256'))
  if (observation.get('start'),observation.get('end'),observation.get('request_offset'),observation.get('pool_offset'),observation.get('pool_end'))!=(step['close_start'],step['end'],offsets['request'],pool,pool_end):raise ValueError('Close observation raw offsets differ')
  cut(observation['end'],observation.get('serial_sha256'))
  issued,sent,request,completed,pool_ns=(key.get('issued_ns'),key.get('sent_ns'),key.get('receipt_observed_ns'),key.get('completed_ns'),observation.get('pool_observed_ns'))
  if any(not integer(v)for v in(issued,sent,request,completed,pool_ns))or not previous<=issued<=sent<=request<=completed<=pool_ns<=upper:raise ValueError('invalid/reversed/cross-run host observation times')
  if any(not integer(observation.get(k))for k in('issued_ns','request_observed_ns','key_to_request_ns','key_to_pool_ns'))or (observation.get('issued_ns'),observation.get('request_observed_ns'),observation.get('key_to_request_ns'),observation.get('key_to_pool_ns'))!=(issued,request,request-issued,pool_ns-issued):raise ValueError('recorded Close observation interval differs')
  rows.append(dict(target=target,task=task,surface=surface,key_action_index=key_index,observation_index=index,key_to_request_ns=request-issued,key_to_pool_ns=pool_ns-issued,issued_ns=issued,request_observed_ns=request,pool_observed_ns=pool_ns,request_offset=offsets['request'],pool_offset=pool))
  used_keys.append(key_index);used_observations.append(index);previous=pool_ns
 if [i for i,a in enumerate(actions)if a.get('kind')=='key'and a.get('key')=='f11']!=used_keys or [i for i,a in enumerate(actions)if a.get('kind')=='browser_close']!=used_observations:raise ValueError('missing/extra/replayed semantic Close actions')
 for code in('57','d7'):
  edges=list(re.finditer(r'\[sexinput.ps2.scancode\] raw=0x'+code+r'(?=\n|$)',trace))
  if len(edges)!=len(used_keys)or any(m.end()==len(trace)or not any(actions[i]['start']<=m.start()<actions[i]['end']for i in used_keys)for m in edges):raise ValueError('unrecorded/extra/truncated physical F11 edge')
 return dict(clock=MONOTONIC_CLOCK,rows=rows,scope='same-run observed hardware-key to serial receipt/pool; includes QMP/transport/polling, not hard realtime')

def write_gzip(source,target):
 with Path(source).open('rb')as raw,Path(target).open('xb')as file:
  with gzip.GzipFile(filename='',fileobj=file,mode='wb',mtime=0)as packed:shutil.copyfileobj(raw,packed)
def disk_hash(path):
 with gzip.open(path,'rb')as stream:
  data=stream.read(MAX_DISK+1)
 if len(data)>MAX_DISK:raise ValueError('bounded disk replay exceeded')
 return sha(data)
def frame_prefix(trace,frame):
 end=frame.get('serial_end')
 if type(end)is not int or not 0<end<=len(trace)or frame.get('serial_sha256')!=sha(trace[:end].encode()):raise ValueError('capture prefix differs')
 return trace[:end]

def capture_fence(trace,frame,row):
 """Bound qualification before screendump; reject changes during the capture."""
 before=frame.get('serial_end');after=frame.get('post_serial_end')
 frame_prefix(trace,frame)
 if not isinstance(after,int)or isinstance(after,bool)or not before<=after<=len(trace)or sha(trace[:after].encode())!=frame.get('post_serial_sha256'):raise ValueError('invalid post-capture prefix fence')
 span=trace[before:after];target,task,surface=row['target'],row['task'],row['surface']
 for tag in ('browser.viewport','browser.tls.layout','browser.tls.applied','display.browser.text.committed','shell.document.viewport'):
  if re.search(re.escape('['+tag+']')+rf' target={target}\b',span):raise ValueError('Browser frame superseded during screendump')
 if '[browser.tls.view]'in span or re.search(r'\[shell.focus.set\] id=\d+'+END,span)or re.search(rf'\[shell.window.action\] action=ZoomToggle frame=\d+ sid={surface}\b',span):raise ValueError('Browser state/focus superseded during screendump')
 return trace[:after]

def resize_evidence(trace,row,start,end,geometry,actions):
 """Caller-return log may follow callee viewport; both must be in this phase."""
 zoom=one(trace,rf'\[shell.window.action\] action=ZoomToggle frame=\d+ sid={row["surface"]} ok=1 reason=ok',start,end)
 view=one(trace,rf'\[browser.viewport\] target={row["target"]} task={row["task"]} sequence={geometry["viewport"]} w={geometry["tile"]["w"]} h={geometry["tile"]["h"]} origin=held_shell',start,end)
 if start+view.start()>=geometry['layout_offset']:raise ValueError('resize frame lacks prior accepted viewport')
 enters=[a for a in actions if a.get('kind')=='key'and a.get('key')=='ret'and start<=a.get('start',-1)<a.get('end',-1)<=end]
 if len(enters)!=1 or not enters[0]['start']<=start+zoom.start()<start+zoom.end()<=enters[0]['end']:raise ValueError('Zoom receipt lacks exact physical Enter gesture')
 return dict(zoom_offset=start+zoom.start(),viewport_offset=start+view.start(),enter_start=enters[0]['start'])

def command_profile(command,iso,out,resolution,model):
 from tls_responsiveness_gate import native_command
 if not isinstance(command,list)or any(not isinstance(v,str)for v in command):raise ValueError('malformed native command')
 if resolution not in('1024x768','1280x800')or model not in('e1000','e1000e'):raise ValueError('unreviewed display/NIC profile')
 qmp=[v for v in command if re.fullmatch(r'unix:/tmp/sex-browser-reserve-\d+\.sock,server=on,wait=off',v)]
 if len(qmp)!=1:raise ValueError('missing exact owned QMP endpoint')
 sock=Path(qmp[0].split(',',1)[0].removeprefix('unix:'))
 expected=native_command(Path(iso),Path(out)/'nvme.img',Path(out)/'serial.log',sock,Path(out)/'wire.pcap',model,resolution)
 if command!=expected:raise ValueError('native source/device/RTC/CPU/image profile differs')

def replay(root,iso,binding,out,mode='quiesced'):
 from check_disk_persistence import ppm_rgb
 from check_tcp_capture import inspect as tcp_capture
 from network_contention_gate import owner_joins
 from check_launcher_pixels import verify as launcher_pixels
 from check_desktop_pixels import font_from_source
 root,out=Path(root),Path(out);contract=source_contract(root);bound=frozen_binding(root,Path(iso),Path(binding));require_inputs(json.loads(Path(binding).read_text()),contract)
 manifest=json.loads((out/'manifest.json').read_text());result=json.loads((out/'result.json').read_text())
 evidence_mode(manifest,result,mode)
 if manifest.get('error')is not None or result.get('ok')is not True or manifest.get('guest_exited')is not True:raise ValueError('failed/nonterminal attempt remains diagnostic')
 if manifest.get('binding')!=bound or manifest.get('source_sha256')!=contract['source_sha256']or manifest.get('serial_offset_units')!=SERIAL_UNITS or manifest.get('tls_accounting')!=contract['tls_accounting']:raise ValueError('frozen source/image, accounting or prefix convention differs')
 command_profile(manifest.get('command'),iso,out,manifest.get('resolution'),manifest.get('model'))
 frames=result.get('frames',[]);names=[f'browser-{n}-{phase}'for n in(15,30,31)for phase in('prepared','expanded','restored')]+['browser-reserve-exhausted']
 if [f.get('name')for f in frames]!=names:raise ValueError('missing/reordered/duplicate immutable captures')
 expected={'serial.log','wire.pcap','endpoint.json','actions.json','seed.img.gz','nvme.img.gz','result.json'}|{f['screenshot']for f in frames}
 if set(manifest.get('raw_sha256',{}))!=expected:raise ValueError('terminal raw closure differs')
 for name,digest in manifest['raw_sha256'].items():
  path=out/name
  if path.is_symlink()or path.resolve().parent!=out.resolve()or sha(path.read_bytes())!=digest:raise ValueError('unsafe/changed raw artifact: '+name)
 if disk_hash(out/'seed.img.gz')!=manifest['input_disk_sha256']or disk_hash(out/'nvme.img.gz')!=manifest['final_disk_sha256']:raise ValueError('terminal disk closure differs')
 trace=serial_text(out/'serial.log')
 if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow|\[fault.kill\]',trace):raise ValueError('guest fault/diagnostic overflow')
 proof=verify(trace,result,root,mode);actions=json.loads((out/'actions.json').read_text());keys=verify_keys(trace,actions)
 if mode=='allocated-close'and result.get('close_evidence')!=[j['close_evidence']for j in proof['joins']]:raise ValueError('stored allocated Close evidence differs from independently replayed raw receipts')
 observations=close_observations(trace,result,actions,proof,manifest)
 peer=json.loads((out/'endpoint.json').read_text())
 if peer.get('errors')!=[]or len(peer.get('rows',[]))!=3 or any(r.get('payload_hex')!=ECHO.hex()or r.get('response_hex')!=ECHO.hex()or(r.get('received'),r.get('sent'))!=(1025,1025)for r in peer['rows']):raise ValueError('exact startup echo endpoint closure differs')
 raw=(out/'wire.pcap').read_bytes();wire=tcp_capture(raw,peer['rows'],guest_ports=range(48001,65536),initiator='guest');owners=owner_joins(trace,raw,[(r['identity']['target'],r['identity']['task'])for r in result['chain']])
 if not wire['ok']or not owners['ok']:raise ValueError('independent startup wire/owner joins failed')
 resolution=tuple(map(int,manifest['resolution'].split('x')));font=font_from_source((root/'servers/sexdisplay/src/main.rs').read_text())
 for index,frame in enumerate(frames):
  prefix=frame_prefix(trace,frame);ppm=(out/frame['screenshot']).read_bytes()
  if sha(ppm)!=frame['screenshot_sha256']or ppm_rgb(ppm,min_width=1,min_height=1)[:2]!=resolution:raise ValueError('capture hash/framebuffer differs')
  if index<9:
   from tls_prepare_gate import consumer_context,tls_contract,verify_tls_pixels
   step=result['chain'][index//3];row=step['identity'];starts=step.get('paint_starts')
   if not isinstance(starts,list)or len(starts)!=3 or starts[0]!=step['work_start']or not starts[0]<starts[1]<starts[2]<step['close_start']:raise ValueError('missing ordered independent paint windows')
   begin=starts[index%3];limit=starts[index%3+1]if index%3<2 else step['close_start']
   if frame.get('target')!=row['target']or frame.get('kind')!='browser_tls'or frame.get('pixels_qualified')is not True or frame.get('window_start')!=begin or not begin<frame['serial_end']<=limit:raise ValueError('Browser TLS capture mislabeled/stale or falsely qualified')
   current=re.findall(r'\[shell.focus.set\] id=(\d+)'+END,prefix)
   if not current or int(current[-1])!=row['surface']:raise ValueError('Browser TLS capture lacks exact current focus')
   capture_fence(trace,frame,row)
   consumer=consumer_context(trace,row['target'],task=row['task'],root=root)
   pixels=verify_tls_pixels(trace,ppm,'HelloPending',tls_contract(root),resolution,consumer=consumer,window=(begin,frame['serial_end']),root=root)
   if not pixels.get('status_qualified')or json.loads(json.dumps(pixels))!=frame.get('proof'):raise ValueError('Browser TLS pixel proof changed or incomplete')
   if index%3:
    resize_evidence(trace,row,begin,frame['serial_end'],pixels['geometry'],actions)
    before=frames[index-1]['proof']['geometry']
    if pixels['geometry']['viewport']<=before['viewport']or pixels['geometry']['paint']<=before['paint']:raise ValueError('resize reuses stale viewport/paint')
    if index%3==1 and (pixels['geometry']['tile']['w'],pixels['geometry']['tile']['h'])==(before['tile']['w'],before['tile']['h']):raise ValueError('Zoom did not change Browser extent')
   if index%3==2:
    original=frames[index-2]['proof']['geometry']['tile'];restored=pixels['geometry']['tile']
    if restored!=original:raise ValueError('restored Browser tile differs')
  else:
   if not result['exhaustion_start']<frame['serial_end']<=result['exhaustion_end']:raise ValueError('stale exhaustion capture')
   pixels=launcher_pixels(prefix,ppm,font,resolution)
   if pixels:raise ValueError('exhausted whole-launcher glyphs failed: '+str(pixels))
 return dict(proof,wire=wire,owners=owners,hardware=keys,close_observations=observations,launcher_pixels_qualified=True,browser_pixels_qualified=True,captures=10)

def native(root,iso,binding,seed,out,resolution='1280x800',model='e1000e',mode='quiesced'):
 from tls_responsiveness_gate import native_command
 from launcher_workflow_gate import pointer_park
 from check_launcher_pixels import verify as launcher_pixels
 from check_desktop_pixels import font_from_source
 mode_value(mode)
 root,iso,binding,seed,out=map(Path,(root,iso,binding,seed,out));out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise ValueError('fresh evidence directory required')
 contract=source_contract(root);bound=frozen_binding(root,iso,binding);require_inputs(json.loads(binding.read_text()),contract)
 if seed.stat().st_size>MAX_DISK:raise ValueError('bounded input disk exceeded')
 shutil.copyfile(seed,out/'nvme.img');shutil.copyfile(binding,out/'source-binding.json')
 input_hash=sha((out/'nvme.img').read_bytes())
 if input_hash!=sha(seed.read_bytes()):raise ValueError('input disk changed during copy')
 write_gzip(out/'nvme.img',out/'seed.img.gz')
 serial=out/'serial.log';sock=Path('/tmp')/f'sex-browser-reserve-{os.getpid()}.sock';pcap=out/'wire.pcap'
 command=native_command(iso,out/'nvme.img',serial,sock,pcap,model,resolution)
 manifest=dict(evidence_version=EVIDENCE_VERSION,options=dict(mode=mode),binding=bound,source_sha256=contract['source_sha256'],tls_accounting=contract['tls_accounting'],serial_offset_units=SERIAL_UNITS,command=command,resolution=resolution,model=model,input_disk_sha256=input_hash,scope=SCOPE,monotonic_clock=MONOTONIC_CLOCK,host_monotonic_start_ns=time.monotonic_ns())
 frames=[];actions=[];result={};error=None;proc=None;q=None;started=time.monotonic();dimensions=tuple(map(int,resolution.split('x')))
 with echo_peer()as peer:
  try:
   proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT)
   def text():return serial_text(serial)
   def health():
    if proc.poll()is not None:raise RuntimeError('guest exited unexpectedly')
    if time.monotonic()-started>720:raise TimeoutError('bounded720second Browser reserve workflow')
    if peer['errors']:raise ValueError(str(peer['errors']))
    if any(p.exists()and p.stat().st_size>MAX_PCAP for p in(serial,pcap)):raise ValueError('bounded raw trace exceeded')
    if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',text()):raise ValueError('guest fault')
   def wait(pattern,start=0,timeout=120):
    deadline=time.monotonic()+timeout
    while time.monotonic()<deadline:
     health();match=re.search(pattern,text()[start:])
     if match:return match
     time.sleep(.02)
    raise TimeoutError('Browser reserve receipt: '+pattern)
   def key(code,pattern):
    prefix=text();begin=len(prefix);issued_ns=time.monotonic_ns();requests=[]
    class ObservedTransport:
     def cmd(self,command,arguments):
      requests.append(dict(execute=command,arguments=json.loads(json.dumps(arguments))))
      return q.cmd(command,arguments)
    hardware_keys(ObservedTransport(),(code,));sent_ns=time.monotonic_ns();match=wait(pattern,begin)
    receipt_ns=time.monotonic_ns();receipt_prefix=text()
    wait(rf'\[sexinput.ps2.scancode\] raw={hex(KEYS[code]|128)}'+END,begin)
    final_prefix=text();completed_ns=time.monotonic_ns()
    actions.append(dict(kind='key',key=code,start=begin,end=len(final_prefix),transport='input-send-event',qmp=requests,
     start_sha256=sha(prefix.encode()),end_sha256=sha(final_prefix.encode()),receipt_pattern=pattern,
     receipt_offset=begin+match.start(),receipt_end=begin+match.end(),receipt_serial_end=len(receipt_prefix),receipt_serial_sha256=sha(receipt_prefix.encode()),
     issued_ns=issued_ns,sent_ns=sent_ns,receipt_observed_ns=receipt_ns,completed_ns=completed_ns));return match
   font=font_from_source((root/'servers/sexdisplay/src/main.rs').read_text())
   def capture(name,kind,target,expected,window_start=None):
    pointer_park(q,dimensions,text,wait,actions.append,len(frames));path=out/(name+'.ppm');candidate=out/(name+'.candidate.ppm')
    deadline=time.monotonic()+45
    while True:
     health();prefix=text();q.cmd('screendump',{'filename':str(candidate)});post=text();raw=candidate.read_bytes()
     pixels=None
     if kind=='browser_tls':
      from tls_prepare_gate import consumer_context,tls_contract,verify_tls_pixels
      try:
       consumer=consumer_context(post,target,root=root)
       fence=dict(serial_end=len(prefix),serial_sha256=sha(prefix.encode()),post_serial_end=len(post),post_serial_sha256=sha(post.encode()))
       capture_fence(post,fence,dict(target=consumer.target,task=consumer.task,surface=consumer.surface))
       pixels=verify_tls_pixels(post,raw,expected,tls_contract(root),dimensions,consumer=consumer,window=(window_start,len(prefix)),root=root)
       errors=[]if pixels.get('status_qualified')else pixels.get('errors')or['TLS status not complete']
      except (ValueError,AssertionError)as exc:errors=[str(exc)]
     else:errors=launcher_pixels(prefix,raw,font,dimensions)if kind=='launcher'else['unknown capture kind']
     if not errors:break
     if time.monotonic()>=deadline:raise ValueError(str(errors))
     time.sleep(.1)
    with path.open('xb')as file:file.write(raw)
    candidate.unlink();frame=dict(name=name,kind=kind,target=target,expected=expected,screenshot=path.name,screenshot_sha256=sha(raw),serial_end=len(prefix),serial_sha256=sha(prefix.encode()),post_serial_end=len(post),post_serial_sha256=sha(post.encode()),pixels_qualified=True)
    if kind=='browser_tls':frame.update(window_start=window_start,proof=json.loads(json.dumps(pixels)))
    frames.append(frame);return frame
   deadline=time.monotonic()+20
   while not sock.exists():
    health()
    if time.monotonic()>=deadline:raise TimeoutError('QMP unavailable')
    time.sleep(.05)
   q=QMP(sock)
   if resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=10);hardware_keys(q,('down',));hardware_keys(q,('ret',))
   wait(r'\[shell.app.registered.reply\] target=15 task=\d+ surface=300 committed=1'+END)
   wait(r'\[net.source.binding.held\] target=31 [^\n]*'+END)
   result=run(dict(root=root,text=text,wait=wait,key=key,capture=capture,actions=actions),mode);result.update(ok=True,frames=frames)
  except Exception as exc:error=safe_exception(exc)
  finally:
   if proc is not None and proc.poll()is None:
    proc.terminate()
    try:proc.wait(timeout=10)
    except subprocess.TimeoutExpired:proc.kill();proc.wait(timeout=10)
   if q is not None:q.sock.close()
   manifest['guest_exited']=proc is not None and proc.poll()is not None
 peer_data=dict(rows=peer['rows'],errors=peer['errors']);(out/'endpoint.json').write_text(json.dumps(peer_data,indent=2)+'\n')
 (out/'actions.json').write_text(json.dumps(actions,indent=2)+'\n');(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 manifest.update(error=error,final_disk_sha256=sha((out/'nvme.img').read_bytes()),host_monotonic_end_ns=time.monotonic_ns())
 write_gzip(out/'nvme.img',out/'nvme.img.gz')
 names=['serial.log','wire.pcap','endpoint.json','actions.json','seed.img.gz','nvme.img.gz','result.json']+[f['screenshot']for f in frames]
 manifest['raw_sha256']={name:sha((out/name).read_bytes())for name in names if(out/name).is_file()}
 (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
 if error:raise RuntimeError(error)
 return replay(root,iso,binding,out,mode)

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--iso',required=True,type=Path);p.add_argument('--binding',required=True,type=Path);p.add_argument('--out',required=True,type=Path)
 p.add_argument('--source-disk',type=Path);p.add_argument('--resolution',choices=('1024x768','1280x800'),default='1280x800');p.add_argument('--model',choices=('e1000','e1000e'),default='e1000e');p.add_argument('--mode',choices=MODES,default='quiesced');p.add_argument('--check',action='store_true');a=p.parse_args()
 iso,binding,out=a.iso.resolve(strict=True),a.binding.resolve(strict=True),a.out.resolve()
 if a.check:result=replay(ROOT,iso,binding,out,a.mode)
 else:
  if a.source_disk is None:p.error('native Browser reserve needs --source-disk')
  result=native(ROOT,iso,binding,a.source_disk.resolve(strict=True),out,a.resolution,a.model,a.mode)
 print(json.dumps(result,indent=2))
if __name__=='__main__':main()
