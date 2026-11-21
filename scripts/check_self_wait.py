#!/usr/bin/env python3
"""Join actual BOOT inventory, timer park/expiry and Ring3 return receipts."""
import argparse,hashlib,json,re
from pathlib import Path

# Independent qualification bound. A larger logged value cannot authorize a
# larger BOOT inventory; changing this profile needs an explicit checker change.
EXPECTED_BOOT_MAIN_CAPACITY=64

# Reviewed causal implementations, not hashes inferred from the trace. Known
# variants include reviewed keyboard-log/SDK plumbing and c20934ae's BOOT
# SerialIO/power setup. That review leaves every deadline owner unchanged.
# Any new causal source requires explicit review. 2026-11-15 master reconcile:
# Denim (syscall 88), Cufflinks/Bluetooth, touchpad route and SYS_VM (110) add no
# deadline reference; USB quiet only narrows (also requires no Bluetooth HID traffic).
# Reviewed causal implementations, one pin per line in self_wait_pins.py (merged
# with merge=union so parallel re-pins do not conflict).
def _pins(kind):
    try:
        import self_wait_pins as module  # a plain import, so source freezers see it
        if Path(module.__file__).resolve()!=Path(__file__).with_name('self_wait_pins.py').resolve():raise ImportError
    except ImportError:
        import importlib.util
        spec=importlib.util.spec_from_file_location('self_wait_pins',Path(__file__).with_name('self_wait_pins.py'))
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    out={}
    for k,path,digest in module.PINS:
        if k==kind and digest not in out.setdefault(path,()):out[path]+=(digest,)
    return out
APPROVED_TIMED_WAIT_SOURCES=_pins('timed')


# Mechanical source closure only, not a general Rust/unsafe-code verifier.
# Any textual writer/poller reference outside these reviewed, hash-pinned owners
# fails closed, including references in new/uncompiled modules or comments.
DEADLINE_REFERENCE_OWNERS=frozenset((
    'kernel/src/scheduler.rs','kernel/src/self_wait.rs','kernel/src/interrupts.rs',
    'kernel/src/program_workers.rs','kernel/src/program_records.rs','kernel/src/syscalls/mod.rs'))
# Reviewed whole-tree digests, kept as review receipts; deadline_source_closure no
# longer requires the current tree to be one of them.
APPROVED_KERNEL_RUST_TREES=_pins('tree').get('',())  # review receipts, one per line in self_wait_pins.py
DEADLINE_REFERENCE=re.compile(r'\b(?:wait_deadline|poll_self_wait_deadlines|take_due|wait_for_message_ticks|wake_record_if_due)\b|\bself_wait\s*::\s*Deadline\b')


# Files that change for reasons unrelated to waiting are pinned by a wait
# fingerprint instead of their whole bytes, so unrelated edits need no re-pin:
# - sex-pdx lib.rs: everything except bare `pub mod x;` module declarations and blank lines.
# - silk-shell main.rs: exactly one `mod wait_observer;` and one
#   `wait_observer::poll();`, plus every line that waits, yields, listens or
#   reaches wait_observer, in order (whitespace removed).
SHELL_WAIT_LINE=re.compile(r'wait_observer|sys_wait_self_ticks|pdx_listen|pdx_try_listen|sys_yield|sched_yield|wait_for_message')
# Kernel causal files are pinned by their wait functions only (Andreas, 2026-11-18):
# every `fn` whose body names a wait/park/wake/timer item. A short one is pinned
# whole; a very long one (dispatch, init) by its signature and those lines. Other
# functions change freely; a new function that waits changes the fingerprint.
# smp.rs stays whole; scheduler.rs, self_wait.rs and task_ownership.rs are modelled (below).
KERNEL_WAIT_TOKEN=re.compile(r'wait_deadline|self_wait|SELF_WAIT|take_due|wait_for_message|wake_record|poll_self_wait|ParkResult|\.park\(|\.wake\(|unpark|sexnet_id|resume_attached|yield_and_switch')
KERNEL_FN_SCOPED=frozenset(('kernel/src/program_records.rs','kernel/src/pd/create.rs','kernel/src/init.rs',
    'kernel/src/interrupts.rs','kernel/src/syscalls/mod.rs','kernel/src/program_workers.rs'))
KERNEL_FN_WHOLE_LINES=200
def _functions(text):
    """Each `fn` with a body: (name, text from `fn` to its closing brace)."""
    for m in re.finditer(r'\bfn\s+(\w+)\s*(?:<[^>{;]*>)?\s*\(',text):
        start=text.find('{',m.end());semi=text.find(';',m.end())
        if start<0 or 0<=semi<start:continue
        depth=0;end=start
        while end<len(text):
            depth+=(text[end]=='{')-(text[end]=='}')
            if depth==0:break
            end+=1
        yield m.group(1),text[m.start():end+1],start-m.start()
def _kernel_wait_functions(text):
    out=[]
    for name,body,_ in _functions(text):
        if not KERNEL_WAIT_TOKEN.search(body):continue
        lines=body.splitlines()
        kept=lines if len(lines)<=KERNEL_FN_WHOLE_LINES else [lines[0]]+[l for l in lines[1:] if KERNEL_WAIT_TOKEN.search(l)]
        out.append(name+':'+''.join(re.sub(r'\s+','',l) for l in kept))
    return out
# Kernel pass 2, K6: proof over pins. The park/wake custody word (task_ownership.rs)
# and the timed-wait deadlines and BOOT inventory (self_wait.rs) are checked by the
# model in tests/model_scheduler_wait.rs, run on the real files by
# scripts/scheduler_model_gate.sh (spec check scheduler_model, with compiled mutants),
# by loom (tests/model_loom_scheduler.rs) and by Kani, so they carry no pin. Their use
# in scheduler.rs is checked against the model too: every scheduler.rs function that
# waits or touches custody must make exactly the custody, deadline and wait calls,
# in order with their match arms, that the model's SCHEDULER_PROTOCOL table lists.
# An edit that keeps those calls needs no pin; one that changes them changes the
# table, next to the models it must pass.
SCHEDULER_MODEL='tests/model_scheduler_wait.rs'
MODELLED_SOURCES=frozenset(('kernel/src/scheduler.rs','kernel/src/self_wait.rs','kernel/src/task_ownership.rs'))
# Snapshots only read the word, so they are left out.
SCHEDULER_PROTOCOL_CALL=re.compile(r'interrupts::disable\(\)|\.state\.(?!snapshot\()\w+\(|\.wait_deadline\.\w+\(|SELF_WAIT_TASKS\.\w+\(|'
    r'\b(?:ParkResult|WakeResult|Admission)::\w+|\.(?:admit_task|admit)\(|\b(?:wake_owned_thread|yield_and_switch|wait_for_local_resume_or_switch|'
    r'unpark_thread|park_current_thread|wake_record|wake_record_if_due|wait_for_message|wait_for_message_ticks|poll_self_wait_deadlines)\(')
def scheduler_protocol(text):
    """{function: calls} for each scheduler.rs function that waits or touches custody."""
    out={}
    for name,body,head in _functions(text):
        inner=body[head:]
        if not KERNEL_WAIT_TOKEN.search(body) and not re.search(r'\.state\.(?!snapshot\()\w+\(',inner):continue
        calls=[re.sub(r'^\.(?=state|wait_deadline)','',c.group(0)).rstrip('(').replace('interrupts::disable()','interrupts::disable')
               for c in SCHEDULER_PROTOCOL_CALL.finditer(re.sub(r'//[^\n]*','',inner))]
        if not calls:continue
        if name in out:raise ValueError('scheduler protocol function named twice: '+name)
        out[name]=' '.join(calls)
    return out
def model_protocol(text):
    table=text.split('pub const SCHEDULER_PROTOCOL: &str = "',1)[1].split('";',1)[0]
    out={}
    for line in table.strip().splitlines():
        name,calls=line.split(':',1)
        out[name.strip()]=' '.join(calls.split())
    return out
def model_fingerprint(root,name,data):
    model=(root/SCHEDULER_MODEL).read_text()
    if name=='kernel/src/scheduler.rs':
        actual,modelled=scheduler_protocol(data.decode()),model_protocol(model)
        for fn in sorted(set(actual)|set(modelled)):
            if actual.get(fn)!=modelled.get(fn):
                raise ValueError('scheduler.rs custody protocol differs from '+SCHEDULER_MODEL+' in '+fn+': kernel '+repr(actual.get(fn))+' model '+repr(modelled.get(fn)))
        return 'model-v1:'+hashlib.sha256(json.dumps(actual,sort_keys=True).encode()).hexdigest()
    if '#[path = "../'+name+'"]' not in model:raise ValueError(SCHEDULER_MODEL+' no longer models '+name)
    return 'model-v1:'+SCHEDULER_MODEL
def wait_fingerprint(name,data):
    text=data.decode()
    if name in KERNEL_FN_SCOPED:
        return 'fn-v1:'+hashlib.sha256('\n'.join(_kernel_wait_functions(text)).encode()).hexdigest()
    if name=='crates/sex-pdx/src/lib.rs':
        kept=[l.rstrip() for l in text.splitlines() if l.strip() and not re.fullmatch(r'\s*pub\s+mod\s+\w+\s*;\s*',l)]
        return 'wait-v1:'+hashlib.sha256('\n'.join(kept).encode()).hexdigest()
    if name=='servers/silk-shell/src/main.rs':
        flat=re.sub(r'\s+','',text)
        if flat.count('modwait_observer;')!=1 or flat.count('wait_observer::poll();')!=1 or flat.count('wait_observer::')!=1:
            raise ValueError('changed Shell wait_observer module/call')
        kept=[re.sub(r'\s+','',l) for l in text.splitlines() if SHELL_WAIT_LINE.search(l)]
        return 'wait-v1:'+hashlib.sha256('\n'.join(kept).encode()).hexdigest()
    return hashlib.sha256(data).hexdigest()


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
    tree=hashlib.sha256(json.dumps(hashes,sort_keys=True,separators=(',',':')).encode()).hexdigest()
    # The tree digest is reported, not required (Andreas, 2026-11-18): a whole-tree pin
    # turned master red whenever two separately pinned kernel PRs merged. Every causal
    # file is still pinned above, and a deadline reference outside the owners fails closed.
    return dict(scope='bounded named-reference scan of the complete kernel Rust tree, with hash-pinned causal files; not macro expansion or general Rust verification',
                files=len(hashes),bytes=total,tree_sha256=tree,owner_sha256=owners,kernel_rust_sha256=hashes)


def source_contract(root):
    """Pin the complete reviewed causal code, including cancellation and clock."""
    sources={}
    try:
        root=Path(root).resolve(strict=True);texts={}
        for name in sorted(set(APPROVED_TIMED_WAIT_SOURCES)|MODELLED_SOURCES):
            path=(root/name).resolve(strict=True)
            if not path.is_relative_to(root) or not path.is_file():raise ValueError('source escapes root: '+name)
            data=path.read_bytes()
            if name in MODELLED_SOURCES:sources[name]=model_fingerprint(root,name,data)
            else:
                digest=wait_fingerprint(name,data);sources[name]=digest
                if digest not in APPROVED_TIMED_WAIT_SOURCES[name]:raise ValueError('unreviewed timed-wait source: '+name)
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
        if 'letquiet=irq&&!queued&&!usb.changed&&!slept&&!usb.ether_active()&&!usb.bt_hid_active();' not in usb or 'sys_wait_self_ticks(ifquiet{32}else{1})' not in usb:
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


# Optional auxiliary role profile; it never establishes the Net timer join.
ACPI_AUX_SOURCES=('servers/sexacpi/src/main.rs','crates/sex-pdx/src/power.rs')
ACPI_AUX_COHORTS=(
 ('822bebd1f4048d35105a0c7fae00b3112b4c11a6013501cf8de528dc5e42dbf6','b13e5f3e739b0000837a81c9ae7212f0356c5d507d04d0f100a421f2ea05ddd6'),
 ('24446427547d1f80eeb557e928b5854c27edc8b0caa45d5aebd7a5e6b460e531','227ca3f652dd675bca14b52f9f287c70139dd1b999d1db3ffe462f0183682616'),
 # FADT PM1 blocks outside IO space refused before FixedRegisters::new; wait loop unchanged.
 ('2456130585cec58be8f2fe41e758175ce79e83faefa3c09bf6acf73b3fc917af','227ca3f652dd675bca14b52f9f287c70139dd1b999d1db3ffe462f0183682616'),
 # power.rs gains OP_DEVICE_SLEEP; sexacpi reports every MWAIT _CST state (wait loop unchanged), 2026-11-20
 ('8e6bb20e7be6ff97c756986704690f9a5ad3791d77106b69de16bfcc5618590a','231583a434ade78732de80b01c85ec47612099338adcc832e0c52e6b0c4e9142'))

def _acpi_wait_profile(text,root,source,seals):
    result=dict(ok=False,task=50,ticks=64,source_sha256={},scope='optional sampled ACPI wait-role admission; no inferred park or user-return proof')
    if source is None or not source['ok'] or len(seals)!=1:return result
    try:
        root=Path(root).resolve(strict=True);texts=[]
        for name in ACPI_AUX_SOURCES:
            path=(root/name).resolve(strict=True)
            if not path.is_relative_to(root)or not path.is_file()or path.stat().st_size>2*1024*1024:raise ValueError('invalid auxiliary source')
            data=path.read_bytes();result['source_sha256'][name]=hashlib.sha256(data).hexdigest();texts.append(re.sub(r'\s+','',data.decode()))
        if tuple(result['source_sha256'].values())not in ACPI_AUX_COHORTS:raise ValueError('unreviewed auxiliary ACPI source cohort')
        if 'let_=sex_pdx::sys_wait_self_ticks(64);'not in texts[0]or 'pubconstACPI_SERVICE_PD:u32=50;'not in texts[1]or 'pubconstACPI_SERVICE_KEY:u8=4;'not in texts[1]:raise ValueError('changed ACPI fixed wait/identity')
        markers=list(re.finditer(re.escape('[kernel.spawn.sexacpi'),text))
        roles=list(re.finditer(r'^\[kernel.spawn.sexacpi\] id=50 key=4 organ=senses\r?\n',text,re.M))
        if len(markers)!=len(roles)or len(roles)!=1 or roles[0].start()>=seals[0].start():raise ValueError('missing unique prior BOOT ACPI role')
        result.update(ok=True,receipt_offset=roles[0].start())
    except (OSError,ValueError,UnicodeError,RuntimeError)as error:result['errors']=[str(error)]
    return result


# Idle sexinput sleeps up to 32 ticks (touchpad::wait_bound(now, IDLE_WAIT_TICKS)), else 1.
INPUT_IDLE_TOKENS=('constIDLE_WAIT_TICKS:u64=32;',
 'sex_pdx::sys_wait_self_ticks(ifidle{touchpad::wait_bound(sex_pdx::get_ticks(),IDLE_WAIT_TICKS)}else{1});',
 'Some(p)=>p.next_poll.saturating_sub(now).clamp(1,bound.max(1)),')
# Denim (BOOT pd 51, key 14) scans every 64 ticks, every 8 while a program waits on a page.
DENIM_TOKENS=('sex_pdx::sys_wait_self_ticks(ifdenim.waiting.is_empty(){64}else{8});','pubconstDENIM_PD:u32=51;','pubconstDENIM_KEY:u8=14;')


def _source_tokens(root,names,tokens):
    root=Path(root).resolve(strict=True);text=''
    for name in names:
        path=(root/name).resolve(strict=True)
        if not path.is_relative_to(root)or not path.is_file()or path.stat().st_size>2*1024*1024:raise ValueError('invalid auxiliary source')
        text+=re.sub(r'\s+','',path.read_bytes().decode())
    missing=[t for t in tokens if t not in text]
    if missing:raise ValueError('changed reviewed wait: '+missing[0])


def _input_idle_profile(root,source):
    result=dict(ok=False,task=4,ticks=32,scope='optional idle sexinput bounded sleep (1..32 ticks)')
    if source is None or not source['ok']:return result
    try:
        _source_tokens(root,('servers/sexinput/src/main.rs','servers/sexinput/src/touchpad.rs'),INPUT_IDLE_TOKENS);result['ok']=True
    except (OSError,ValueError,UnicodeError,RuntimeError)as error:result['errors']=[str(error)]
    return result


# Quil (BOOT domain 9) waits 64 ticks between autosave countdown checks while a draft is armed.
QUIL_TOKENS=('}elseifunsafe{QUIL_AUTOSAVE.armed()}{//Idledraftcountdown:boundedtimedwait,wokenearlybyinput.matchpdx_try_listen_raw(0){Some(msg)=>msg,None=>{let_=sex_pdx::sys_wait_self_ticks(64);continue}}',)


def _quil_wait_profile(text,root,source,seals):
    result=dict(ok=False,task=QUIL_TASK,ticks=64,scope='optional Quil autosave countdown wait-role admission')
    if source is None or not source['ok'] or len(seals)!=1:return result
    try:
        _source_tokens(root,('servers/quil/src/main.rs',),QUIL_TOKENS)
        markers=list(re.finditer(re.escape('[kernel.spawn.quil'),text))
        roles=list(re.finditer(r'^\[kernel.spawn.quil\] id=9 path=\S+\r?\n',text,re.M))
        if len(markers)!=len(roles)or len(roles)!=1 or roles[0].start()>=seals[0].start():raise ValueError('missing unique prior BOOT Quil role')
        result.update(ok=True,receipt_offset=roles[0].start())
    except (OSError,ValueError,UnicodeError,RuntimeError)as error:result['errors']=[str(error)]
    return result


def _denim_wait_profile(text,root,source,seals):
    result=dict(ok=False,task=DENIM_TASK,ticks=(64,8),scope='optional Denim scan-loop wait-role admission')
    if source is None or not source['ok'] or len(seals)!=1:return result
    try:
        _source_tokens(root,('servers/denim/src/main.rs','crates/sex-pdx/src/denim.rs'),DENIM_TOKENS)
        markers=list(re.finditer(re.escape('[kernel.spawn.denim'),text))
        roles=list(re.finditer(r'^\[kernel.spawn.denim\] id=51 key=14 organ=memory\r?\n',text,re.M))
        if len(markers)!=len(roles)or len(roles)!=1 or roles[0].start()>=seals[0].start():raise ValueError('missing unique prior BOOT Denim role')
        result.update(ok=True,receipt_offset=roles[0].start())
    except (OSError,ValueError,UnicodeError,RuntimeError)as error:result['errors']=[str(error)]
    return result


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


# The kernel logs a park/expiry when its global ordinal is sampled (first four,
# then powers of two) or, for BOOT task ids below 64, when the task's own ordinal
# (task_count) is sampled (first SELF_WAIT_TASK_FIRST, then powers of two).
# Net logs its first NET_RETURN_FIRST timed returns, then powers of two. Since
# every Net expiry ends one Net timed wait, an early Net expiry and its return
# are both logged however busy the other tasks are.
SELF_WAIT_TASK_FIRST=32
NET_RETURN_FIRST=32

def _record_sampled(record):
    own=record[5]
    return _sampled(int(record[4]),4)or(own is not None and int(record[1])<64 and _sampled(int(own),SELF_WAIT_TASK_FIRST))


def _task_ordinals_ok(records):
    last={}
    for record in records:
        if record[5] is None:continue
        task,own=int(record[1]),int(record[5])
        if(own==0)!=(task>=64)or not _record_sampled(record):return False
        if own and own<=last.get(task,0):return False
        if own:last[task]=own
    return True

# The optional passive diagnostic observer never grants authority or causes a wake.
# Only this actual reviewed Shell/API/kernel cohort is admitted; future merge
# combinations need independent review and an exact new fingerprint.
PASSIVE_OBSERVER_SOURCES = {'servers/silk-shell/src/wait_observer.rs': '99900d657e10aa0aca89e34bdd2b632207aebe2e1655eeb5bc0a16a03b73c5fc',
 'servers/silk-shell/src/main.rs': 'wait-v1:8f387c104199b88d2ea09b2248a64cde8d22b6ca7655b967b87b072e88ce687b',
 'crates/sex-pdx/src/system_stats.rs': '408fb8ea07d12eb2fc53f38797b8ddb4fd61c32140ecc516e63b69fd5689f97d',
 'crates/sex-pdx/src/crash_log.rs': '4930c8a6ca16756e9421cc1b1c8dca8a0301413cb0f3694189005fa90579ae22',
 'crates/sex-pdx/src/web.rs': '2cd0cedd8bf837aca965d45a147d5251006d0d14521179b1d6c1496a8cffeccd',
 # core_view: Relaxed reads only (reviewed by the microkernel thread, 2026-11-20).
 'kernel/src/cpu_account.rs': '422f8b140abc987fd2758508927e687eeaab2fb6f5fda4e9997addd719d16d6b',
 'kernel/src/core_local.rs': '5abe787a39e72745c3087029be159b43a7ca38e4c95edc57bbd6892abdfaf6cb',
 'kernel/src/app_resources.rs': '3ae927a6b4abc6651d556fae749f32a96250b0a72102cc378c637afdee02c119',
 'kernel/src/service_restart.rs': 'c8899b8f71ab65b431a7c2f05380ec7562d398857ebbbde97787b11fedf63b5e',
 'kernel/src/crash_log.rs': '1a0619db6d5e2e9a18f60f9da923df062d90206d7378009a5751c2802dc145d7',  # SMAP: user copies through smap::user (no observer change), 2026-11-20
 'crates/sex-pdx/src/app_catalog.rs': '07b846017c83b1662fc66dbdffd14dd09374b7a58454ebf0cbbc06954757e0f3'}


def passive_observer_source(root,core):
    result=dict(ok=False,source_sha256={},scope='reviewed passive public-counter/peer-epoch observer; not a general Rust verifier')
    if core is None or not core['ok']:return result
    try:
        root=Path(root).resolve(strict=True)
        for name,expected in PASSIVE_OBSERVER_SOURCES.items():
            path=(root/name).resolve(strict=True)
            if not path.is_relative_to(root)or not path.is_file()or path.stat().st_size>2*1024*1024:raise ValueError('invalid passive observer source')
            digest=wait_fingerprint(name,path.read_bytes());result['source_sha256'][name]=digest
            if digest!=expected:raise ValueError('unreviewed passive observer source: '+name)
        result['core_source_sha256']=core['source_sha256']
        result.update(ok=True,conditions=['diagnostic-only-Shell-main-loop-call','public-counter-indexes-current-pd-not-query-target',
          'read-only-domain13-counter-and-held-route262-epoch/life','BOOT-Net13-single-main-no-program-workers',
          'UP-tick-read-before-peer/counter/peer-read','monotonic-first-boot-epoch-and-counter'])
    except (OSError,ValueError,RuntimeError)as error:result['errors']=[str(error)]
    return result


def passive_observer_joins(text,root,core,topology,identity,seals,expiries):
    proof=dict(ok=False,source_contract=passive_observer_source(root,core),samples=0,scope='sampled UP expiry then later Ring3 syscall continuation; not an exact two-tick return envelope')
    if not proof['source_contract']['ok']or not topology['ok']or not identity['ok']or len(seals)!=1:return [],proof
    def fail(reason):proof['errors']=[reason];return [],proof
    # The diagnostic cfg must be observed, never inferred from an image name.
    mode_markers=list(re.finditer(r'\[input.payload.mode\] component=shell\b',text))
    modes=list(re.finditer(r'^\[input.payload.mode\] component=shell mode=proof\r?\n',text,re.M))
    ready_markers=list(re.finditer(re.escape('[shell.wait_observer.ready]'),text))
    ready=list(re.finditer(r'^\[shell.wait_observer.ready\] mode=proof target=13 sys=87 period=250 limit=512\r?\n',text,re.M))
    if len(mode_markers)!=len(modes)or len(modes)!=1 or len(ready_markers)!=len(ready)or len(ready)!=1:return fail('missing unique diagnostic observer startup/mode')
    handoff_markers=list(re.finditer(r'^\[bootgraph.boot.handoff\] target=sexnet\b',text,re.M))
    handoff=list(re.finditer(r'^\[bootgraph.boot.handoff\] target=sexnet id=13 entry=0x[0-9a-f]+\r?\n',text,re.M))
    if len(handoff_markers)!=len(handoff)or len(handoff)!=1 or not(identity['receipt_offset']<seals[0].start()<handoff[0].start()):return fail('missing unique BOOT Net13 spawn/seal/handoff')
    registrations=list(re.finditer(r'^\[boot.service.restartable\] pd=13 task=13 pages=[1-9][0-9]* entry=0x[0-9a-f]+\r?\n',text,re.M))
    registration_markers=list(re.finditer(r'\[boot.service.restartable\] pd=13\b',text))
    if len(registrations)!=1 or len(registration_markers)!=1 or not(identity['receipt_offset']<registrations[0].start()<seals[0].start()):return fail('missing unique initial Net restart-registration identity')
    # A fixed-domain counter must never be attributed to another task/incarnation.
    for line in text.splitlines():
        if line==registrations[0].group().rstrip('\r\n'):continue
        if re.search(r'fault|panic|restart|retir|rebirth|worker|thread.start',line,re.I)and re.search(r'\b(?:target|pd|caller_pd|owner|logical|task)=13\b',line):return fail('Net fault/lifecycle/worker ambiguity')
    number=r'(?:0|[1-9][0-9]{0,19})'
    records=list(re.finditer(re.escape('[shell.wait_observer]'),text))
    samples=list(re.finditer(r'^\[shell.wait_observer\] seq=('+number+r') target=13 before=('+number+r') after=('+number+r') syscalls=('+number+r') task=0 slot=262 epoch_before=1 life_before=1 epoch_after=1 life_after=1 ok=1\r?\n',text,re.M))
    if len(records)!=len(samples)or not 2<=len(samples)<=512:return fail('missing/malformed/budget-exceeded observer samples')
    proof['samples']=len(samples)
    if not(modes[0].start()<ready[0].start()<samples[0].start()and handoff[0].start()<samples[0].start()):return fail('observer samples precede diagnostic/BOOT identity')
    previous=None
    for i,sample in enumerate(samples,1):
        seq,before,after,counter=map(int,sample.groups())
        if seq!=i or max(before,after,counter)>0xffff_ffff_ffff_ffff or after<before:return fail('invalid observer sequence/clock/counter')
        if previous is not None:
            _,old_before,old_after,old_counter=previous
            if before<old_before+250 or before<old_after or counter<old_counter:return fail('observer clock/counter reset or throttle violation')
        previous=(seq,before,after,counter)
    for expiry in expiries:
        if int(expiry[1])!=13 or not _complete(text,expiry,r'\r?\n')or not _record_sampled(expiry):continue
        if not(handoff[0].start()<expiry.start()and topology['receipt_offset']<expiry.start()and 2<=int(expiry[2])<=int(expiry[3])):continue
        for first,second in zip(samples,samples[1:]):
            aseq,abefore,aafter,acount=map(int,first.groups());bseq,bbefore,bafter,bcount=map(int,second.groups())
            # BOTH reads are causally post-expiry: serial order alone is not a
            # clock fence, nor may an after-only clock straddle the actual read.
            if first.start()<=expiry.start()or second.start()<=expiry.start()or min(abefore,bbefore)<int(expiry[3])or bcount<=acount:continue
            proof.update(ok=True,expiry_offset=expiry.start(),baseline_offset=first.start(),continuation_offset=second.start())
            return [dict(task=13,deadline=int(expiry[2]),expired_at=int(expiry[3]),expiry_count=int(expiry[4]),
              baseline=dict(sequence=aseq,before=abefore,after=aafter,syscalls=acount,peer_epoch=1),
              continuation=dict(sequence=bseq,before=bbefore,after=bafter,syscalls=bcount,peer_epoch=1),
              evidence_type='source_bound_up_expiry_later_ring3_syscall_continuation',exact_two_tick_envelope=False)],proof
    return fail('no strictly increasing post-expiry Net counter pair')


SHELL_TASK=3
QUIL_TASK=9
DENIM_TASK=51
SHELL_AUTOFILL_MAX_TICKS=1000


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
 parks=list(re.finditer(r'\[scheduler.self_wait.park\] task=(\d+) ticks=(\d+) deadline=(\d+) count=(\d+)(?: task_count=(\d+))?',text))
 expiries=list(re.finditer(r'\[scheduler.self_wait.expired\] task=(\d+) deadline=(\d+) now=(\d+) count=(\d+)(?: task_count=(\d+))?',text))
 returns=list(re.finditer(r'\[sexnet.receive.wait\] before=(\d+) after=(\d+) ticks=2 count=(\d+) kind=timer_assisted_poll irq=0',text))
 valid_seal=len(markers)==len(seals)==1 and 1<=int(seals[0][1])<=EXPECTED_BOOT_MAIN_CAPACITY
 acpi=_acpi_wait_profile(text,source_root,source,seals)
 auxiliary=((50,64),)if acpi['ok']else()
 inputs=_input_idle_profile(source_root,source)if source_root is not None else dict(ok=False)
 input_idle=tuple((4,t)for t in range(2,33))if inputs['ok']else()
 denim=_denim_wait_profile(text,source_root,source,seals)
 denim_waits=((DENIM_TASK,64),(DENIM_TASK,8))if denim['ok']else()
 quil=_quil_wait_profile(text,source_root,source,seals)
 quil_waits=((QUIL_TASK,64),)if quil['ok']else()
 if not valid_seal:errors.append('missing/invalid unique complete BOOT timer inventory (capacity must be 64)')
 if len(authority)!=1:errors.append('missing unique real Ring3 invalid-argument probes')
 if seals and any(m.start()<seals[0].start()for m in parks+expiries+authority):errors.append('timed wait precedes BOOT seal')
 for p in parks:
  # SexNet (13) and explicitly selected Browser tasks poll every 2 ticks;
  # since round 105 the
  # sexdrive main task (2) waits 1 tick for NVMe completions. Park records
  # are sampled (first four, then powers of two), so either may appear.
  # With the wired NIC's interrupt armed, an idle SexNet (13) sleeps 10 ticks (irq_wait::QUIET_TICKS).
  # Idle Display (1) and SilkBar (6) sleep 8 ticks; polled sexinput (4) and sexusb (5) sleep 1.
  # Shell (3) wakes itself for password autofill (silk-shell autofill_routes::wake):
  # 100 ticks while a page identity is held, at most 1000 toward a transaction deadline;
  # and for the tray (tray_wake), at most 1000 while a Wi-Fi answer is owed or due.
  shell=source_ok and int(p[1])==SHELL_TASK and 1<=int(p[2])<=SHELL_AUTOFILL_MAX_TICKS
  if not shell and((int(p[1]),int(p[2]))not in((13,2),(2,1),(1,8),(6,8),(4,1),(5,1))+(((5,32),(13,10))if source_ok else())+input_idle+denim_waits+quil_waits+tuple((task,2)for task in selected)+auxiliary)or int(p[3])==0:errors.append('unexpected timed-poll task/interval/deadline: task=%s ticks=%s'%(p[1],p[2]))
 for e in expiries:
  if int(e[1])not in(13,2,1,6,4,5)+((SHELL_TASK,)if source_ok else())+((DENIM_TASK,)if denim['ok']else())+((QUIL_TASK,)if quil['ok']else())+selected+((50,)if acpi['ok']else()) or int(e[3])<int(e[2]):errors.append('foreign task or early timer expiry: task=%s deadline=%s now=%s'%(e[1],e[2],e[3]))
 for r in returns:
  if int(r[2])<int(r[1]):errors.append('backward Ring3 clock')
 for group,index in [(parks,4),(expiries,4),(returns,3)]:
  counts=[int(x[index])for x in group]
  if any(n<=0 for n in counts)or any(a>=b for a,b in zip(counts,counts[1:])):errors.append('repeated/backward diagnostic ordinal')
 for group in (parks,expiries):
  if not _task_ordinals_ok(group):errors.append('repeated/backward/unsampled per-task diagnostic ordinal')
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
   if int(e[1])!=13 or not _complete(text,e,r'\r?\n') or not _record_sampled(e):continue
   matches=[r for r in returns if r.start()>e.start() and topology['receipt_offset']<e.start()
            and _complete(text,r,r' device=[01] reply_pending=[01]\r?\n')
            and _sampled(int(r[3]),NET_RETURN_FIRST) and int(r[1])<=int(e[2])-2<=int(e[3])<=int(r[2])]
   if len(matches)>1:errors.append('ambiguous source-bound Net return envelopes')
   if len(matches)==1:candidates.append((e,matches[0]))
  for e,r in candidates:
   if sum(other.start()==r.start()for _,other in candidates)!=1:
    errors.append('multiple expiries in one source-bound Net return envelope');continue
   inferred.append(dict(task=13,deadline=int(e[2]),expired_at=int(e[3]),returned_at=int(r[2]),
                        before=int(r[1]),expiry_count=int(e[4]),return_count=int(r[3]),
                        evidence_type='source_bound_up_expiry_return_inferred_park'))
 continued=[];observer=dict(ok=False,scope='not needed by existing observed/return-envelope proof')
 if active and not observed and not inferred and source_ok and topology['ok'] and net_identity['ok'] and valid_seal:
  continued,observer=passive_observer_joins(text,source_root,source,topology,net_identity,seals,expiries)
 joined=observed+inferred
 if active and(not parks or not expiries or not returns or(not joined and not continued)):
  net_expiries=[(int(e[2]),int(e[3]),int(e[4]))for e in expiries if int(e[1])==13][:6]
  errors.append('missing actual park→timer expiry→user return join or reviewed UP equivalent'
                +' (parks=%d expiries=%d net_expiries=%s returns=%d first_returns=%s source_ok=%s topology=%s net_identity=%s seal=%s observer=%s)'%(
                len(parks),len(expiries),net_expiries,len(returns),[(int(r[1]),int(r[2]),int(r[3]))for r in returns][:6],
                source_ok,topology['ok'],net_identity['ok'],valid_seal,observer.get('errors',observer.get('source_contract',{}).get('errors'))))
 if not active and(any(int(p[1])==13 for p in parks)or any(int(e[1])==13 for e in expiries)or returns):
  if not returns or any(not text[r.end():].startswith(' device=0 reply_pending=1')for r in returns):errors.append('unsupported NIC unexpectedly entered device polling without retained reply work')
 if re.search(r'TIMED_WAIT_WITHOUT_ATTACHED_OWNER|BOOT_SELF_WAIT_AUTHORITY|SELF_WAIT_FRONTEND_PROBE|KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|panicked at',text):errors.append('guest self-wait failure')
 return dict(ok=not errors,errors=errors,browser_tasks=list(selected),boot_tasks=int(seals[0][1])if valid_seal else None,parks_observed=len(parks),expiries_observed=len(expiries),returns_observed=len(returns),timer_user_joins=joined,observed_timer_user_joins=observed,inferred_timer_user_joins=inferred,passive_continuation_joins=continued,passive_observer_contract=observer,source_contract=source,auxiliary_wait_profiles={'acpi':acpi,'input_idle':inputs,'denim':denim,'quil':quil},up_topology=topology,net_boot_identity=net_identity,scope='Sampled real BOOT/main-task/park/timer/Ring3 receipts with explicitly caller-selected observed Browser task IDs; no identity autodiscovery or authority grant. Native peer and desktop gates separately establish useful progress. Nominal timer-assisted polling, not NIC IRQ, CPU/power bound, physical or SMP proof. Inferred park requires reviewed unchanged causal source, exact observed UP topology and one-to-one expiry/Net-return envelopes; caller separately verifies frozen source/image binding. Passive diagnostic continuation is reported separately: sampled expiry then later first-epoch Ring3 syscall entry, not an exact two-tick envelope or IPC-exclusive wake reason.')

def main():
 p=argparse.ArgumentParser();p.add_argument('log',type=Path);p.add_argument('--inactive',action='store_true');p.add_argument('--source-root',type=Path,help='reviewed causal source for sampled UP equivalence and quiet USB32');a=p.parse_args();r=inspect(a.log.read_text(errors='replace'),not a.inactive,source_root=a.source_root);print(json.dumps(r,indent=2));raise SystemExit(0 if r['ok']else 1)
if __name__=='__main__':main()
