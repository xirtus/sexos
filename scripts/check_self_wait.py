#!/usr/bin/env python3
"""Join actual BOOT inventory, timer park/expiry and Ring3 return receipts."""
import argparse,hashlib,json,re
from pathlib import Path

# Independent qualification bound. A larger logged value cannot authorize a
# larger BOOT inventory; changing this profile needs an explicit checker change.
EXPECTED_BOOT_MAIN_CAPACITY=64

# Reviewed causal implementations, not hashes inferred from the trace. Two
# variants differ only in the separately reviewed keyboard-log redaction and
# SDK module plumbing. Any new causal source requires explicit review.
APPROVED_TIMED_WAIT_SOURCES = {'kernel/src/program_records.rs': ('37561f94116d2c3c44b1bf88b8f22927ebd57eeec7630cd6e40a8d38150fb2a8',),
'kernel/src/pd/create.rs': ('3fff0e0483f767e2e8509515874d0b320496cea0671c85b21c4aa368bbdf9b1e',),
'kernel/src/init.rs': ('784b18c238608b124ed2f2e2cd8d1d0eeaf6933b7bf9291fbc07b230688f0f64',),
'kernel/src/scheduler.rs': ('0b75bce2639de6891a9ed4b7e3e0f871f9a5110bc23b9002b2912316e4ef7f64',),
 'kernel/src/self_wait.rs': ('0e7b60adbce245427e67bf1bb0ba20ddb929c304efbad13ce078fa79e43e4eaf',),
 'kernel/src/task_ownership.rs': ('57e00d7269315b5810adb7d43bcc75e92a140b6fa02b1c2dd8c15533757564f6',),
 'kernel/src/smp.rs': ('bd523573ed7c9576665af35b7f0d598ddccb5b9a44f21d5d85fb21a15d961325',),
 'kernel/src/interrupts.rs': ('c686d2ff054854086e2f27e872b1c0d0eafd3c9aa88636b716775b33288afd0d',
                              'dac8806ada26bbf32518d7a835d009c96bfd4787aa724a9fd29eefccc6fac62d'),
 'kernel/src/syscalls/mod.rs': ('aa6169383696167b349be8f37819f61fb896239b3ff7aa489f4ffac1a4e11524',),
 'kernel/src/program_workers.rs': ('6180ef3fb5702e6bd63e1649b07df8ce39390c8fe69a6a9c93b2e199686dbe8e',),
 'crates/sex-pdx/src/lib.rs': ('8477d3326af90b744f72d300aa26af3b4b8ae7075527ca1b3aa622ea9ec5ccde',
                               'f76238b37b9a35de5bdf5e7e62c95e6788e30e86411ef4ec77d8638b4b82ef05'),
 'servers/sexnet/src/main.rs': ('00da2ed6f93bb473e3944a8a4bc7e38f887676083cfe8cf9195c821cba4fc725',),
 'servers/sexusb/src/main.rs': ('3fb9341fda71262a023114c8115407949ce700c604c7c3327464e79a68af3a81',
                                'adc83a02655343df1b83d99dedd252a7e9ee06164055a54420b5e9207b3dba06')}


# Mechanical source closure only, not a general Rust/unsafe-code verifier.
# Any textual writer/poller reference outside these reviewed, hash-pinned owners
# fails closed, including references in new/uncompiled modules or comments.
DEADLINE_REFERENCE_OWNERS=frozenset((
    'kernel/src/scheduler.rs','kernel/src/self_wait.rs','kernel/src/interrupts.rs',
    'kernel/src/program_workers.rs','kernel/src/program_records.rs','kernel/src/syscalls/mod.rs'))
DEADLINE_REFERENCE=re.compile(r'\b(?:wait_deadline|poll_self_wait_deadlines|take_due|wait_for_message_ticks|wake_record_if_due)\b|\bself_wait\s*::\s*Deadline\b')


def deadline_source_closure(root):
    kernel=root/'kernel';hashes={};owners={};total=0;entries=0
    if kernel.is_symlink() or not kernel.resolve(strict=True).is_relative_to(root):raise ValueError('kernel scan escapes source root')
    for path in kernel.rglob('*'):
        entries+=1
        if entries>4096:raise ValueError('kernel deadline source scan entry bound exceeded')
        if path.is_symlink():raise ValueError('kernel deadline source scan does not accept symlinks')
        if path.suffix!='.rs':continue
        if not path.is_file() or not path.resolve(strict=True).is_relative_to(root):raise ValueError('invalid kernel Rust source')
        size=path.stat().st_size
        if len(hashes)>=1024 or size>2*1024*1024 or total+size>16*1024*1024:
            raise ValueError('kernel deadline source scan size bound exceeded')
        data=path.read_bytes();total+=len(data);name=path.relative_to(root).as_posix()
        hashes[name]=hashlib.sha256(data).hexdigest()
        if DEADLINE_REFERENCE.search(data.decode()):
            if name not in DEADLINE_REFERENCE_OWNERS:raise ValueError('unreviewed kernel deadline reference owner: '+name)
            owners[name]=hashes[name]
    if set(owners)!=DEADLINE_REFERENCE_OWNERS:raise ValueError('reviewed kernel deadline reference owner missing')
    return dict(scope='bounded textual Rust deadline writer/poller references; not a general Rust verifier',
                files=len(hashes),bytes=total,owner_sha256=owners,kernel_rust_sha256=hashes)


def source_contract(root):
    """Pin the complete reviewed causal code, including cancellation and clock."""
    sources={}
    try:
        root=Path(root).resolve(strict=True);texts={}
        for name,approved in APPROVED_TIMED_WAIT_SOURCES.items():
            path=(root/name).resolve(strict=True)
            if not path.is_relative_to(root) or not path.is_file():raise ValueError('source escapes root: '+name)
            data=path.read_bytes();digest=hashlib.sha256(data).hexdigest();sources[name]=digest
            if digest not in approved:raise ValueError('unreviewed timed-wait source: '+name)
            texts[name]=re.sub(r'\s+','',data.decode())
        closure=deadline_source_closure(root)
        scheduler=texts['kernel/src/scheduler.rs']
        wait=scheduler.split('pubunsafefnwait_for_message_ticks(',1)[1].split('pubfnpoll_self_wait_deadlines(',1)[0]
        order=['interrupts::disable();','SELF_WAIT_TASKS.contains(task)','wait_deadline.arm(now,ticks)','state.park(core.core_id)']
        if [wait.index(x)for x in order]!=sorted(wait.index(x)for x in order):raise ValueError('changed interrupt/arm/park order')
        for token in ('ParkResult::WakeConsumed=>{(*task).wait_deadline.cancel();true}',
                      'ParkResult::Exited=>{(*task).wait_deadline.cancel();yield_and_switch(regs_ptr)}',
                      'ParkResult::NotOwner=>{(*task).wait_deadline.cancel();panic!("TIMED_WAIT_WITHOUT_ATTACHED_OWNER")}'):
            if token not in wait:raise ValueError('changed nonblocked cancellation')
        usb=texts['servers/sexusb/src/main.rs']
        if 'letquiet=irq&&!queued&&!usb.changed&&!slept&&!usb.ether_active();' not in usb or 'sys_wait_self_ticks(ifquiet{32}else{1})' not in usb:
            raise ValueError('changed USB quiet predicate')
        net=texts['servers/sexnet/src/main.rs']
        envelope='letbefore=sex_pdx::get_ticks();sex_pdx::sys_wait_self_ticks(2).expect("BOOT_SELF_WAIT_AUTHORITY");letafter=sex_pdx::get_ticks();'
        if net.count(envelope)!=1:raise ValueError('changed Net two-tick clock envelope')
        return dict(ok=True,contract='reviewed-up-self-wait-v1',source_sha256=sources,deadline_reference_closure=closure,
                    shapes=['interrupts-disabled-before-arm-and-park','all-nonblocked-paths-cancel',
                            'deadline-take-due-before-timer-wake','IPC-and-exit-cancel',
                            'same-kernel-tick-clock-before-and-after-exact-Net2call','exact-USB-quiet-predicate','BOOT-Net-pd13-main-task-identity'])
    except (OSError,ValueError,KeyError,IndexError,UnicodeError,TypeError,RuntimeError)as error:
        return dict(ok=False,contract='reviewed-up-self-wait-v1',source_sha256=sources,errors=[str(error)])


def _up_topology(text):
    markers=list(re.finditer(re.escape('[smp.online'),text))
    matches=list(re.finditer(r'^\[smp.online\] cores=1 present=1\r?\n',text,re.M))
    first=list(re.finditer(r'^\[smp.core.first_task\] core=([0-9]+) pd=[0-9]+ tsc=[0-9]+\r?\n',text,re.M))
    first_markers=list(re.finditer(re.escape('[smp.core.first_task'),text))
    valid=len(markers)==len(matches)==1 and len(first)==len(first_markers) and all(m[1]=='0'for m in first)
    return dict(ok=valid,cores=1 if valid else None,present=1 if valid else None,
                receipt_offset=matches[0].start()if valid else None)


def _net_identity(text):
    markers=list(re.finditer(re.escape('[kernel.spawn.sexnet'),text))
    matches=list(re.finditer(r'^\[kernel.spawn.sexnet\] id=13 path=/servers/sexnet\r?\n',text,re.M))
    valid=len(markers)==len(matches)==1
    return dict(ok=valid,task=13 if valid else None,path='/servers/sexnet'if valid else None,
                receipt_offset=matches[0].start()if valid else None)


def _complete(text,match,suffix):
    return match.start()==text.rfind('\n',0,match.start())+1 and re.match(suffix,text[match.end():])is not None


def _sampled(count,initial):
    return count>0 and(count<=initial or count&(count-1)==0)

def inspect(text,active=True,*,browser_tasks=(15,),source_root=None):
 errors=[]
 source=source_contract(source_root)if source_root is not None else None
 source_ok=source is not None and source['ok']
 topology=_up_topology(text)
 net_identity=_net_identity(text)
 # IDs are observed task identities, not catalog PD numbers or indices into
 # the 64-entry inventory. A caller must join its held source records first.
 valid_browser_tasks=isinstance(browser_tasks,(tuple,list))and 1<=len(browser_tasks)<=EXPECTED_BOOT_MAIN_CAPACITY
 valid_browser_tasks=valid_browser_tasks and all(type(task)is int and 0<task<=0xffff_ffff for task in browser_tasks)
 valid_browser_tasks=valid_browser_tasks and len(set(browser_tasks))==len(browser_tasks)
 selected=tuple(browser_tasks)if valid_browser_tasks else()
 if not valid_browser_tasks:errors.append('invalid explicit Browser task IDs: require 1..64 unique positive u32 identities')
 markers=list(re.finditer(re.escape('[boot.self_wait.sealed'),text))
 seals=list(re.finditer(r'^\[boot.self_wait.sealed\] tasks=([1-9][0-9]?) capacity='+str(EXPECTED_BOOT_MAIN_CAPACITY)+r' max_ticks=64 runtime_registration=0\r?\n',text,re.M))
 authority=list(re.finditer(r'\[sexnet.wait.authority\] invalid_intervals=3 spare_arguments=4 ok=1',text))
 parks=list(re.finditer(r'\[scheduler.self_wait.park\] task=(\d+) ticks=(\d+) deadline=(\d+) count=(\d+)',text))
 expiries=list(re.finditer(r'\[scheduler.self_wait.expired\] task=(\d+) deadline=(\d+) now=(\d+) count=(\d+)',text))
 returns=list(re.finditer(r'\[sexnet.receive.wait\] before=(\d+) after=(\d+) ticks=2 count=(\d+) kind=timer_assisted_poll irq=0',text))
 valid_seal=len(markers)==len(seals)==1 and 1<=int(seals[0][1])<=EXPECTED_BOOT_MAIN_CAPACITY
 if not valid_seal:errors.append('missing/invalid unique complete BOOT timer inventory (capacity must be 64)')
 if len(authority)!=1:errors.append('missing unique real Ring3 invalid-argument probes')
 if seals and any(m.start()<seals[0].start()for m in parks+expiries+authority):errors.append('timed wait precedes BOOT seal')
 for p in parks:
  # SexNet (13) and explicitly selected Browser tasks poll every 2 ticks;
  # since round 105 the
  # sexdrive main task (2) waits 1 tick for NVMe completions. Park records
  # are sampled (first four, then powers of two), so either may appear.
  # Idle Display (1) and SilkBar (6) sleep 8 ticks; polled sexinput (4) and sexusb (5) sleep 1.
  if (int(p[1]),int(p[2]))not in((13,2),(2,1),(1,8),(6,8),(4,1),(5,1))+(((5,32),)if source_ok else())+tuple((task,2)for task in selected) or int(p[3])==0:errors.append('unexpected timed-poll task/interval/deadline')
 for e in expiries:
  if int(e[1])not in(13,2,1,6,4,5)+selected or int(e[3])<int(e[2]):errors.append('foreign task or early timer expiry')
 for r in returns:
  if int(r[2])<int(r[1]):errors.append('backward Ring3 clock')
 for group,index in [(parks,4),(expiries,4),(returns,3)]:
  counts=[int(x[index])for x in group]
  if any(n<=0 for n in counts)or any(a>=b for a,b in zip(counts,counts[1:])):errors.append('repeated/backward diagnostic ordinal')
 joined=[]
 for e in expiries:
  if int(e[1])!=13:continue
  matching=next((p for p in parks if p.start()<e.start()and p[1]==e[1]and p[3]==e[2]),None)
  returned=next((r for r in returns if r.start()>e.start()and int(r[1])<=int(e[2])-2 and int(r[2])>=int(e[3])),None)
  if matching and returned:joined.append(dict(task=int(e[1]),deadline=int(e[2]),expired_at=int(e[3]),returned_at=int(returned[2])))
 observed=[dict(item,evidence_type='observed_park_expiry_return')for item in joined]
 inferred=[]
 # An expiry is not generally a Blocked-state receipt: on SMP a remote timer
 # may claim an armed deadline before park. On the reviewed UP implementation,
 # interrupts stay disabled through arm/park and every nonblocked route cancels.
 # Thus a due deadline inside this one Net call proves its unsampled Blocked path.
 if active and not observed and source_ok and topology['ok'] and net_identity['ok'] and valid_seal and net_identity['receipt_offset']<seals[0].start():
  candidates=[]
  for e in expiries:
   if int(e[1])!=13 or not _complete(text,e,r'\r?\n') or not _sampled(int(e[4]),4):continue
   matches=[r for r in returns if r.start()>e.start() and topology['receipt_offset']<e.start()
            and _complete(text,r,r' device=[01] reply_pending=[01]\r?\n')
            and _sampled(int(r[3]),8) and int(r[1])<=int(e[2])-2<=int(e[3])<=int(r[2])]
   if len(matches)>1:errors.append('ambiguous source-bound Net return envelopes')
   if len(matches)==1:candidates.append((e,matches[0]))
  for e,r in candidates:
   if sum(other.start()==r.start()for _,other in candidates)!=1:
    errors.append('multiple expiries in one source-bound Net return envelope');continue
   inferred.append(dict(task=13,deadline=int(e[2]),expired_at=int(e[3]),returned_at=int(r[2]),
                        before=int(r[1]),expiry_count=int(e[4]),return_count=int(r[3]),
                        evidence_type='source_bound_up_expiry_return_inferred_park'))
 joined=observed+inferred
 if active and(not parks or not expiries or not returns or not joined):errors.append('missing actual park→timer expiry→user return join or reviewed UP equivalent')
 if not active and(any(int(p[1])==13 for p in parks)or any(int(e[1])==13 for e in expiries)or returns):
  if not returns or any(not text[r.end():].startswith(' device=0 reply_pending=1')for r in returns):errors.append('unsupported NIC unexpectedly entered device polling without retained reply work')
 if re.search(r'TIMED_WAIT_WITHOUT_ATTACHED_OWNER|BOOT_SELF_WAIT_AUTHORITY|SELF_WAIT_FRONTEND_PROBE|KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|panicked at',text):errors.append('guest self-wait failure')
 return dict(ok=not errors,errors=errors,browser_tasks=list(selected),boot_tasks=int(seals[0][1])if valid_seal else None,parks_observed=len(parks),expiries_observed=len(expiries),returns_observed=len(returns),timer_user_joins=joined,observed_timer_user_joins=observed,inferred_timer_user_joins=inferred,source_contract=source,up_topology=topology,net_boot_identity=net_identity,scope='Sampled real BOOT/main-task/park/timer/Ring3 receipts with explicitly caller-selected observed Browser task IDs; no identity autodiscovery or authority grant. Native peer and desktop gates separately establish useful progress. Nominal timer-assisted polling, not NIC IRQ, CPU/power bound, physical or SMP proof. Inferred park requires reviewed unchanged causal source, exact observed UP topology and one-to-one expiry/Net-return envelopes; caller separately verifies frozen source/image binding.')

def main():
 p=argparse.ArgumentParser();p.add_argument('log',type=Path);p.add_argument('--inactive',action='store_true');p.add_argument('--source-root',type=Path,help='reviewed causal source for sampled UP equivalence and quiet USB32');a=p.parse_args();r=inspect(a.log.read_text(errors='replace'),not a.inactive,source_root=a.source_root);print(json.dumps(r,indent=2));raise SystemExit(0 if r['ok']else 1)
if __name__=='__main__':main()
