#!/usr/bin/env python3
"""Join sustained native alpha evidence; a short rehearsal cannot pass."""
import argparse
import hashlib
import json
import math
import pathlib
import re
from check_desktop_close_log import check as cleanup_errors
from check_document_disk import recover


def finite(value):
    return not isinstance(value,bool) and isinstance(value,(int,float)) and math.isfinite(value)


def integer(value):
    return isinstance(value,int) and not isinstance(value,bool) and value>=0


def terminal_pending_errors(serial, include_quil=True):
    """Legacy pending teardown means pending at the terminal Close itself."""
    errors=[]
    for target in ([9,12] if include_quil else [12]):
        requests=list(re.finditer(rf'\[shell.app.close.request\] target={target} task={target}(?:\s|$)',serial))
        finishes=[]
        for m in re.finditer(r'\[task.finish\]([^\r\n]*)',serial):
            fields=dict(re.findall(r'(\w+)=([^\s]+)',m[1]))
            if fields.get('pd')==str(target) and fields.get('task')==str(target) and fields.get('terminal')=='1':finishes.append(m)
        if not requests or len(finishes)!=1 or requests[-1].start()>=finishes[0].start():
            errors.append(f'target{target}: pending teardown needs fresh exact-task Close/finish interval');continue
        interval=serial[requests[-1].start():finishes[0].start()]
        pattern=r'\[quil.document.close\] pending=1(?:\s|$)' if target==9 else r'\[spindle.probe.closed\] target=12 pending=1 terminal=1(?:\s|$)'
        if len(re.findall(pattern,interval))!=1:errors.append(f'target{target}: pending-at-terminal-Close receipt missing/duplicate')
    return errors


def quil_close_errors(soak, serial=None):
    """Version2 facts distinguish the pending attempt from terminal cleanup.

    Passing serial replays exact normalized-character offsets. Pending0 at
    termination is a completed-save Close, never proof of pending teardown.
    """
    errors=[];f=soak.get('quil_close') or {}
    keys=['attempt_start','save_begin','dirty_edit','declined','shell_declined',
          'confirm_start','confirmed','document_close']
    positions=[f.get(k)for k in keys]
    if not all(integer(p)for p in positions) or any(a>=b for a,b in zip(positions[1:5],positions[2:6])) or not (
        positions[0]<=positions[1] and positions[5]<=positions[6]<positions[7]):
        return ['Quil Close requires ordered fresh exact receipt offsets']
    for k in ['save_revision','dirty_revision','document','snapshot_bytes']:
        if not integer(f.get(k)):errors.append('invalid Quil Close '+k)
    if errors:return errors
    if f['dirty_revision']!=f['save_revision']+1 or not 2<=f['document']<15:
        errors.append('Quil Close snapshot/edit identity or revision differs')
    if not re.fullmatch(r'0x[0-9a-f]{1,16}',str(f.get('snapshot_hash',''))):
        errors.append('Quil Close exact immutable snapshot hash required')
    terminal=f.get('terminal_pending');confirmed=f.get('confirmed_pending')
    if type(f.get('declined_pending'))is not int or f.get('declined_pending')!=1:
        errors.append('ordinary unsaved Close did not observe a pending save')
    if type(terminal)is not int or terminal not in (0,1) or type(confirmed)is not int or confirmed!=terminal:
        errors.append('actual confirmed/document Close pending state differs')
    if soak.get('pending_save_close_guard')is not True:
        errors.append('pending-save ordinary Close guard missing')
    if soak.get('pending_quil_close')is not bool(terminal):
        errors.append('pending Quil teardown claim differs from actual terminal state')
    scope='pending_io_teardown' if terminal else 'safe_completed_save_close'
    remaining=[] if terminal else ['Quil pending-I/O terminal teardown']
    if f.get('terminal_scope')!=scope or soak.get('remaining_obligations')!=remaining:
        errors.append('Quil terminal scope/remaining obligation mislabeled')
    complete=f.get('save_complete')
    if terminal==0:
        if not isinstance(complete,dict):errors.append('completed-save Close needs positive exact save completion')
        else:
            expected=dict(begin=f['save_begin'],revision=f['save_revision'],current_revision=f['dirty_revision'],
                          dirty=1,bytes=f['snapshot_bytes'],hash=f['snapshot_hash'])
            if any(complete.get(k)!=v for k,v in expected.items()) or not all(integer(complete.get(k))for k in ['saved','completed']) or not (
                f['declined']<complete.get('saved',-1)<complete.get('completed',-1)<f['document_close']):
                errors.append('completed-save Close snapshot/revision/order differs')
    elif terminal==1 and complete is not None:
        errors.append('pending teardown cannot claim prior completed save')
    if serial is None:return errors
    def at(position,pattern):
        return integer(position) and position<len(serial) and re.match(pattern,serial[position:])is not None
    receipts={
        'save_begin':rf'\[quil.document.begin\] kind=save object={f["document"]} revision={f["save_revision"]} bytes={f["snapshot_bytes"]}(?:\s|$)',
        'dirty_edit':rf'\[quil.document.edit\] revision={f["dirty_revision"]} pending=1 bytes={f["snapshot_bytes"]+1}(?:\s|$)',
        'declined':rf'\[quil.close.declined\] dirty=1 pending=1 revision={f["dirty_revision"]}(?:\s|$)',
        'shell_declined':r'\[shell.app.close.declined\] target=9 task=9 reason=unsaved next_close=confirmed(?:\s|$)',
        'confirmed':rf'\[quil.close.confirmed\] dirty=1 pending={confirmed}(?:\s|$)',
        'document_close':rf'\[quil.document.close\] pending={terminal}(?:\s|$)',
    }
    for k,pattern in receipts.items():
        if not at(f[k],pattern):errors.append('Quil Close exact serial receipt differs: '+k)
    if re.search(r'\[quil.document.close\]',serial[f['attempt_start']:f['confirm_start']]):
        errors.append('ordinary declined Close already performed teardown')
    for lower,upper in [('attempt_start','declined'),('confirm_start','confirmed')]:
        requests=re.findall(r'\[shell.app.close.request\] target=9 task=9(?:\s|$)',serial[f[lower]:f[upper]])
        if len(requests)!=1:errors.append('fresh exact-task Quil Close request missing/duplicate')
    if terminal==0 and isinstance(complete,dict):
        for key,pattern in [
            ('saved',rf'\[quil.persist.save.ok\] bytes={f["snapshot_bytes"]} hash={f["snapshot_hash"]} doc={f["document"]}(?:\s|$)'),
            ('completed',rf'\[quil.document.saved\] saved_revision={f["save_revision"]} revision={f["dirty_revision"]} dirty=1(?:\s|$)')]:
            if not at(complete.get(key),pattern):errors.append('Quil Close exact completion differs: '+key)
    return errors


def cycle_errors(soak):
    """Require distinct ordered work windows, not a sample count plus idle."""
    errors=[];windows=soak.get('cycle_windows');cycles=soak.get('cycles');elapsed=soak.get('elapsed_seconds')
    if not integer(cycles) or not finite(elapsed) or not isinstance(windows,list) or len(windows)!=cycles:
        return ['distinct timed work windows required for every active cycle']
    active=0;prior_end=0;prior_serial=0
    for i,w in enumerate(windows,1):
        if not isinstance(w,dict):return ['malformed active cycle window']
        a,b=w.get('start_seconds'),w.get('end_seconds');s,e=w.get('serial_start'),w.get('serial_end')
        if not integer(w.get('cycle')) or w['cycle']!=i or not finite(a) or not finite(b) or not prior_end<=a<b<=elapsed or not integer(s) or not integer(e) or not prior_serial<=s<e:
            return ['active cycle windows overlap, repeat, or exceed session bounds']
        active+=b-a;prior_end=b;prior_serial=e
    if not finite(soak.get('active_elapsed_seconds')) or abs(soak['active_elapsed_seconds']-active)>1e-5 or not finite(soak.get('idle_elapsed_seconds')) or abs(soak['idle_elapsed_seconds']-(elapsed-active))>1e-5:
        errors.append('active/idle elapsed accounting differs from exact work windows')
    # The source runner deliberately idles <=15s per cycle. One second total
    # permits host scheduling/record-writing overhead, not an idle soak.
    if elapsed-active>15*cycles+1:errors.append('sustained duration contains excessive unaccounted idle time')
    samples=soak.get('samples',[])
    if not isinstance(samples,list) or any(not isinstance(s,dict)for s in samples):return errors+['malformed cycle samples']
    if any(not integer(s.get('sample_id'))for s in samples) or [s.get('sample_id')for s in samples]!=list(range(1,len(samples)+1)):
        errors.append('sample identifiers duplicate or lack ordered custody')
    for sample in samples:
        cycle=sample.get('cycle')
        if not integer(cycle) or cycle>cycles:
            errors.append('sample has invalid cycle');continue
        if cycle==0:continue  # Explicit post-soak cleanup, outside elapsed.
        w=windows[cycle-1];a=sample.get('action_seconds');b=sample.get('session_seconds');r=sample.get('recorded_seconds');n=sample.get('seconds')
        start,end=sample.get('serial_start'),sample.get('serial_end')
        if not all(finite(v)for v in [a,b,r,n]) or not w['start_seconds']<=a<=b<=r<=w['end_seconds'] or abs((b-a)-n)>1e-5 or not integer(start) or not integer(end) or not w['serial_start']<=start<end<=w['serial_end']:
            errors.append('sample timing/receipt prefix is outside its active cycle')
    required=['quil_input_receipt','quil_input_pixels','warm_save_receipt','warm_save_pixels','spindle_stat']
    for cycle in range(1,cycles+1):
        for kind in required:
            if len([s for s in samples if s.get('cycle')==cycle and s.get('kind')==kind])!=1:
                errors.append(f'cycle{cycle}: exactly one {kind} required')
        for phase in ['save_admission','cycle_end']:
            for kind in ['linen_input_receipt','linen_refresh']:
                if len([s for s in samples if s.get('cycle')==cycle and s.get('kind')==kind and s.get('phase')==phase])!=1:
                    errors.append(f'cycle{cycle}: exact Linen {phase}/{kind} required')
    return errors


def boot_lifecycle_errors(result):
    """Lifecycle joins also serve same-image reboot checks of short rehearsals."""
    errors=[];soak=result.get('soak') or {}
    third=soak.get('third_quil') or {}
    expected_successors=([16,17]+[step.get('target')for step in third['chain']])if 'chain'in third else ([16,17,18]if third else [16,17])
    if soak.get('boot_successors')!=expected_successors or soak.get('cleanup_errors')!=[]:
        errors.append('fresh BOOT successors and complete cleanup required')
    if third:
        third=soak.get('third_quil') or {}
        expected=dict(target=18,surface=307,logical=9,incarnation=3,edited_and_saved=True,
                      pixel_errors=[],terminal=True,reserve_exhausted=True)
        if any(third.get(k)!=v for k,v in expected.items()) or third.get('documents')!=[d.get('object')for d in result.get('documents',[])]:
            errors.append('third BOOT incarnation needs exact recovery/input/terminal proof')
    return errors


def session_errors(result):
    errors=[]
    if result.get('ok') is not True or result.get('acceptance') is not True:
        errors.append('not an accepted session (rehearsals are excluded)')
    soak=result.get('soak') or {}
    if soak.get('acceptance') is not True:
        errors.append('sustained acceptance missing')
    elapsed=soak.get('elapsed_seconds',0)
    if isinstance(elapsed,bool) or not isinstance(elapsed,(int,float)) or not math.isfinite(elapsed) or elapsed<1800:
        errors.append('less than 30 minutes of sustained work')
    cycles=soak.get('cycles',0)
    if not isinstance(cycles,int) or isinstance(cycles,bool) or cycles<12:
        errors.append('insufficient active mixed-app cycles')
    samples=soak.get('samples',[])
    if not isinstance(samples,list) or any(not isinstance(s,dict)for s in samples):
        return errors+['malformed observed latency samples']
    count=cycles if integer(cycles) else 0
    for kind in ['quil_input_receipt','quil_input_pixels','warm_save_receipt','warm_save_pixels','spindle_stat','linen_refresh']:
        values=[s for s in samples if s.get('kind')==kind]
        if len(values)<count:errors.append('missing per-cycle latency: '+kind)
        for value in values:
            n=value.get('seconds')
            if isinstance(n,bool) or not isinstance(n,(int,float)) or not math.isfinite(n) or n<0:
                errors.append('invalid observed latency: '+kind)
    version=soak.get('evidence_version',1)
    if type(version)is not int or version not in (1,2):errors.append('unsupported alpha evidence version')
    elif version==2:
        errors+=cycle_errors(soak)+quil_close_errors(soak)
        if soak.get('pending_spindle_close')is not True:errors.append('pending Spindle teardown required')
        windows=soak.get('cycle_windows')
        close=soak.get('quil_close')or{}
        if isinstance(windows,list) and windows and isinstance(windows[-1],dict) and integer(windows[-1].get('serial_end')) and integer(close.get('attempt_start')) and windows[-1]['serial_end']>close['attempt_start']:
            errors.append('post-soak Close precedes the final active cycle')
    elif not all(soak.get(k) is True for k in ['pending_quil_close','pending_spindle_close']):
        errors.append('legacy alpha requires both pending-I/O terminal Close cases')
    errors+=boot_lifecycle_errors(result)
    docs=result.get('documents',[])
    if len(docs)!=2 or len({d.get('object') for d in docs})!=2:
        errors.append('two distinct saved documents required')
    if not result.get('keyboard') or not result.get('pointer') or not result.get('edit_during_save_dirty'):
        errors.append('actual input and immutable save revision evidence missing')
    return errors


def inspect(path):
    path=pathlib.Path(path)
    result=json.loads((path/'result.json').read_text())
    errors=session_errors(result)
    text=(path/'serial.log').read_text(errors='replace')
    from quil_reserve_catalog import quil_catalog
    third=(result.get('soak')or{}).get('third_quil');rows=quil_catalog(pathlib.Path(__file__).resolve().parents[1])if third and 'chain'in third else []
    errors+=cleanup_errors(text,include_successor=True,include_spindle_successor=True,include_third_quil=bool(third),additional_quil=rows[3:])
    if (result.get('soak')or{}).get('evidence_version')==2:
        errors+=quil_close_errors(result['soak'],text)
    errors+=terminal_pending_errors(text,include_quil=(result.get('soak')or{}).get('evidence_version',1)==1 or (result.get('soak')or{}).get('pending_quil_close')is True)
    if re.search(r'\[sexfiles.defer.drop\]|serial\.record\.overflow|KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at',text):
        errors.append('fault, dropped service request or diagnostic overflow')
    manifest=json.loads((path/'manifest.json').read_text())
    binding=json.loads((path/'source-binding.json').read_text())
    digest=result.get('iso_sha256')
    if not digest or digest!=manifest.get('iso_sha256') or digest!=binding.get('iso_sha256'):
        errors.append('session/source/image bindings differ')
    disk=(path/'nvme.img').read_bytes()
    documents=[]
    for expected in result.get('documents',[]):
        found=recover(disk,expected['object'])
        payload=found.pop('payload')
        if payload!=expected['content'].encode('ascii') or len(payload)!=expected['bytes']:
            errors.append('saved document bytes differ: '+expected['name'])
        documents.append(found)
    retirements=[dict(re.findall(r'([a-z_]+)=([^\s]+)',m)) for m in re.findall(r'\[app.private.retired\] ([^\r\n]+)',text)]
    targets={9,12,15,16,17}|({18}if third else set())|{r['target']for r in rows[3:]}
    pages=sum(int(r['pages']) for r in retirements if r.get('target')in map(str,targets) and r.get('task')==r.get('target'))
    return dict(ok=not errors,errors=errors,iso_sha256=digest,
                elapsed_seconds=(result.get('soak')or{}).get('elapsed_seconds'),
                cycles=(result.get('soak')or{}).get('cycles'),private_pages_zeroed_and_reused=pages,
                documents=documents,disk_sha256=hashlib.sha256(disk).hexdigest(),
                evidence_version=(result.get('soak')or{}).get('evidence_version',1),
                pending_quil_teardown_proved=(result.get('soak')or{}).get('pending_quil_close')is True,
                remaining_obligations=(result.get('soak')or{}).get('remaining_obligations',[]),
                scope='real native session, exact durable payloads and five-role private cleanup; pending attempt and terminal state are distinct; reboot checked separately')


def main():
    p=argparse.ArgumentParser();p.add_argument('session',type=pathlib.Path);a=p.parse_args()
    try:result=inspect(a.session)
    except (OSError,ValueError,KeyError,TypeError) as e:result=dict(ok=False,errors=[str(e)])
    print(json.dumps(result,indent=2));return not result['ok']
if __name__=='__main__':raise SystemExit(main())
