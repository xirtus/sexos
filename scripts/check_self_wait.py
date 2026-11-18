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
APPROVED_TIMED_WAIT_SOURCES = {'kernel/src/program_records.rs': ('37561f94116d2c3c44b1bf88b8f22927ebd57eeec7630cd6e40a8d38150fb2a8',),
'kernel/src/pd/create.rs': ('3fff0e0483f767e2e8509515874d0b320496cea0671c85b21c4aa368bbdf9b1e',),
'kernel/src/init.rs': ('784b18c238608b124ed2f2e2cd8d1d0eeaf6933b7bf9291fbc07b230688f0f64', 'a977d60363a05adb60083b90bc6d898913c4efcd81a80209d0b9a4946340e334', '04012df3ba205083c64b904130b8cef0a43814099ad9109008c1d9957acd8550', '26a66c2f676742ab23625962ad5d8d380be6a670e56d0a659d1c614305bbdd8e', '905080c14b576c29a9f8dc0c90400bbd93d0b46723d4397fb851af5d317bd0e8', '3ed435db8f36db8c12c6fbc2b39a041926891e0135920b0c7426cf2e7375dc0d',),
'kernel/src/scheduler.rs': ('8956a2cebc2a3acd3be1b4f8fbab9bb967fcfd7304d66220854be44f61e64654',  # IPC fast path (WS4), 2026-11-17
    '0b75bce2639de6891a9ed4b7e3e0f871f9a5110bc23b9002b2912316e4ef7f64',
                             # per-task self-wait sampling (task_count), 2026-11-17
                             '25c0bc835f30039f947f28bc79ed567e1c742e17ea3c9a8e0663b5f502c9d642',),
 'kernel/src/self_wait.rs': ('0e7b60adbce245427e67bf1bb0ba20ddb929c304efbad13ce078fa79e43e4eaf',),
 'kernel/src/task_ownership.rs': ('57e00d7269315b5810adb7d43bcc75e92a140b6fa02b1c2dd8c15533757564f6',),
 'kernel/src/smp.rs': ('bd523573ed7c9576665af35b7f0d598ddccb5b9a44f21d5d85fb21a15d961325',),
 'kernel/src/interrupts.rs': ('c686d2ff054854086e2f27e872b1c0d0eafd3c9aa88636b716775b33288afd0d',
                              'dac8806ada26bbf32518d7a835d009c96bfd4787aa724a9fd29eefccc6fac62d'),
 'kernel/src/syscalls/mod.rs': ('055312eb92a25b451b946caef4f00d1999391840bb782a1697db72e1903b1c41', '318f8602a3e053470c4f6f984fa818f928cae16253dcf9a9946edf1b114995ba',  # IPC fast path (WS4), 2026-11-17
    'aa6169383696167b349be8f37819f61fb896239b3ff7aa489f4ffac1a4e11524', '425d2dc341b9dd2b5d03200a38a1ef85f0ed96de103e0524a5d51ba947500fa0', 'd5b9e12bf9f40d9170ee7557343187559a2fe73909d90a251c3982c04a3f078f',),
 'kernel/src/program_workers.rs': ('6180ef3fb5702e6bd63e1649b07df8ce39390c8fe69a6a9c93b2e199686dbe8e', '82050a7bd043822bd1593d798dbdf14381d5efba4122a71a63a0948287251f6a',),
 'crates/sex-pdx/src/lib.rs': ('8477d3326af90b744f72d300aa26af3b4b8ae7075527ca1b3aa622ea9ec5ccde', '9eb262eb2108d51f57ae9dc821e6e40a12a9e1003fbfac711422b246ec17fd44',
                               'f76238b37b9a35de5bdf5e7e62c95e6788e30e86411ef4ec77d8638b4b82ef05', 'f9dd629e5cdda6ba58fb130e988d33f0d3a88948278d2f414bf612dca02fc166',
                               'c2895d4991c2d48d099daee3e6090063b3288f77c0ad5e9a75201c903f95c984', 'ef6e695c7535ac077e8c4adef7ded6d79425210a1fd77005940be7f823cab40a', 'cee8e1525fa104c2c14f398b4eee80f4f63cd0f3f06602310cdf421dc15009c1', '1b7fe3f1752da8d10ea2f2ea6df5c387ff9ec9969a4287be64fb6e878f4a4d75', '562ccc2f5cf8cf755e68925f0a65a699fd9e968ac78a44bfe4c7dc84f464e7ab', 'f00f44669af45f707a4a24ec15f70321ed9ed04565007c9e1b093b42fdd39e96', '3766c55f455e02199154d9cdbe28c5fc93a454f91a2459fe04e2eafaf8abd24b', '74a5f4ccbd9aeeb574ff5c9d8f7008b486d89e214ef405ef589711dfda71904b',
                               # wait fingerprint (wait_fingerprint), 2026-11-16
                               'wait-v1:0f64f51bb2e99d980f283c55f402b56e3f5da5a1d22b814ac98b4b1867b8a3d2',
                               # BLOCK_MAX_RUN 512 KiB + BLOCK_CMD_RUN (no wait change), 2026-11-17
                               'wait-v1:aa1083137f9cf350f5a1fe54038943fa82596df2b4c8737d6e4f4fe2e4598946',),
 # Reviewed restricted-provider intake/polling; the exact Net2 envelope and
 # deadline owners are unchanged. See RESTRICTED_TIMER_SOURCE_REVIEW_2026_11_15.
 'servers/sexnet/src/main.rs': ('00da2ed6f93bb473e3944a8a4bc7e38f887676083cfe8cf9195c821cba4fc725',
                              'd7526adf2002e45afd5ad986808e65e7eee3d5f793f718e62f8893fef51ae10d', 'fcf8299b6ce8b8acdd0bc005507036087d5569664c08d63da001dc9e694c7253',
                              # first 32 timed returns logged, 2026-11-17
                              '10fd38acaa45e76df8e7bd579e382b66365f46b944d2fc958104b8631476cb5b',),
 'servers/sexusb/src/main.rs': ('3fb9341fda71262a023114c8115407949ce700c604c7c3327464e79a68af3a81',
                                'adc83a02655343df1b83d99dedd252a7e9ee06164055a54420b5e9207b3dba06', '129171cc0512a8097fe5535c36f8e76a1c358355432f9105f556fe2c454335ec')}


# Mechanical source closure only, not a general Rust/unsafe-code verifier.
# Any textual writer/poller reference outside these reviewed, hash-pinned owners
# fails closed, including references in new/uncompiled modules or comments.
DEADLINE_REFERENCE_OWNERS=frozenset((
    'kernel/src/scheduler.rs','kernel/src/self_wait.rs','kernel/src/interrupts.rs',
    'kernel/src/program_workers.rs','kernel/src/program_records.rs','kernel/src/syscalls/mod.rs'))
# c209 raw-master only; no speculative combined privacy/kernel cohort.
APPROVED_KERNEL_RUST_TREES=(
    '417835283472c60cfd1455bccc223876d2936126736b182b47c1e75f6ed56260',  # Phase 1a (#406) + IPC v2 objects (#410) combined on master, 2026-11-18
    # Phase 1a dead kernel modules removed, 2026-11-18
    '620ffd8bab9d6528e4842df3930d060d4ceacc071b7587db37604bd3ddaf7c6d','ad54b57094dec16de9007a317b15506afc52747967aa9d4fedd100a32b1d95dd',  # IPC fast path (WS4), 2026-11-17
    '91e41656109be5dd659d70a70718e0640f68372f35762c34d95a28260b807e92', 'dd69100da62b94d190510e266327094b2990443442b2a59326b4e60afd2792dc', '1e5f9a4228d4f0342d7d54c4cbc2cba37b9c59ba1923e0d82b1afa50a687dbcb', '42f85248058717dfcb4983c4209337b544347a2167abfe769fe30a86330c94b4', 'b84ac4b152e0920d553a0a5121b31ea516522801e09aac03a499a45637d83707', 'd39d8695743904116ed5e7bc98b47524ffd018d2b28c17ca16d5a1eddc56307c', 'fa1efe78d28850a66b795eb44f7ff544f85ac9033673335a02e8ae5fc54056eb', 'e93d67e07151c4c0f5eed240e0f3a27f9abb58074e26f52934eb8c36963278f7',
    # per-task self-wait sampling, 2026-11-17
    'a897c8b8c617cfef1ec3a7fad038f32f8530b232cf698f9ea896a89950d5a0ec',
    # WS2 kernel heap counters (SYS_STATS OP_KERNEL), unused slab.rs removed, 2026-11-17
    '95ef2223d462a85888843a0700105ff4538f044360df25138dde638253e06fb5',
    # Kernel plan phase 2: kernel/src/ipc_v2 (endpoint, reply, notification), not wired yet, 2026-11-17
    'c620598cd6151890bb134924fcbb76dbe7ab8396503971b66f7e52b1c8251cbd')
DEADLINE_REFERENCE=re.compile(r'\b(?:wait_deadline|poll_self_wait_deadlines|take_due|wait_for_message_ticks|wake_record_if_due)\b|\bself_wait\s*::\s*Deadline\b')


# Files that change for reasons unrelated to waiting are pinned by a wait
# fingerprint instead of their whole bytes, so unrelated edits need no re-pin:
# - sex-pdx lib.rs: everything except bare `pub mod x;` module declarations and blank lines.
# - silk-shell main.rs: exactly one `mod wait_observer;` and one
#   `wait_observer::poll();`, plus every line that waits, yields, listens or
#   reaches wait_observer, in order (whitespace removed).
SHELL_WAIT_LINE=re.compile(r'wait_observer|sys_wait_self_ticks|pdx_listen|pdx_try_listen|sys_yield|sched_yield|wait_for_message')
def wait_fingerprint(name,data):
    text=data.decode()
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
    if tree not in APPROVED_KERNEL_RUST_TREES:raise ValueError('unreviewed kernel Rust source tree')
    return dict(scope='reviewed complete kernel Rust tree plus bounded named-reference scan; not macro expansion or general Rust verification',
                files=len(hashes),bytes=total,tree_sha256=tree,owner_sha256=owners,kernel_rust_sha256=hashes)


def source_contract(root):
    """Pin the complete reviewed causal code, including cancellation and clock."""
    sources={}
    try:
        root=Path(root).resolve(strict=True);texts={}
        for name,approved in APPROVED_TIMED_WAIT_SOURCES.items():
            path=(root/name).resolve(strict=True)
            if not path.is_relative_to(root) or not path.is_file():raise ValueError('source escapes root: '+name)
            data=path.read_bytes();digest=wait_fingerprint(name,data);sources[name]=digest
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
 ('24446427547d1f80eeb557e928b5854c27edc8b0caa45d5aebd7a5e6b460e531','227ca3f652dd675bca14b52f9f287c70139dd1b999d1db3ffe462f0183682616'))

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
 'crates/sex-pdx/src/crash_log.rs': '98357d79e617b3cf3a9895e8e4eaa38ca8d2510ce02a432229aff6698b87c621',
 'crates/sex-pdx/src/web.rs': '2cd0cedd8bf837aca965d45a147d5251006d0d14521179b1d6c1496a8cffeccd',
 'kernel/src/cpu_account.rs': '281b10b4f856a7f665c7ba59951b73b926decefa5c3bfae968af5714ba202f45',
 'kernel/src/core_local.rs': '5abe787a39e72745c3087029be159b43a7ca38e4c95edc57bbd6892abdfaf6cb',
 'kernel/src/app_resources.rs': '1c4de7fca3f3fc4cc6ade73eecec83c269f800a0d520069b2201466d15c761a0',
 'kernel/src/service_restart.rs': 'c8899b8f71ab65b431a7c2f05380ec7562d398857ebbbde97787b11fedf63b5e',
 'kernel/src/crash_log.rs': '4a6a0f5312fdbec23efa0d5497b820105ba244c35cc878f27db04ba98cedc3ae',
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
  # Idle Display (1) and SilkBar (6) sleep 8 ticks; polled sexinput (4) and sexusb (5) sleep 1.
  # Shell (3) wakes itself for password autofill (silk-shell autofill_routes::wake):
  # 100 ticks while a page identity is held, at most 1000 toward a transaction deadline;
  # and for the tray (tray_wake), at most 1000 while a Wi-Fi answer is owed or due.
  shell=source_ok and int(p[1])==SHELL_TASK and 1<=int(p[2])<=SHELL_AUTOFILL_MAX_TICKS
  if not shell and((int(p[1]),int(p[2]))not in((13,2),(2,1),(1,8),(6,8),(4,1),(5,1))+(((5,32),)if source_ok else())+input_idle+denim_waits+quil_waits+tuple((task,2)for task in selected)+auxiliary)or int(p[3])==0:errors.append('unexpected timed-poll task/interval/deadline: task=%s ticks=%s'%(p[1],p[2]))
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
