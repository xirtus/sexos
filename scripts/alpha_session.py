"""Sustained real-input session extension for quil_workflow_gate (no builds)."""
import math
import json
import re
import time
import threading
from check_document_disk import fnv


def save_receipt(serial, start, document, payload, current_revision=None):
    """Join one immutable snapshot and its completed revision, not any save."""
    raw=payload.encode("utf8")
    begin=re.search(rf"\[quil.document.begin\] kind=save object={document} revision=(\d+) bytes={len(raw)}(?:\s|$)",serial[start:])
    if not begin:return None
    begin_at=start+begin.start();revision=int(begin[1])
    saved=re.search(rf"\[quil.persist.save.ok\] bytes={len(raw)} hash={fnv(raw):#x} doc={document}(?:\s|$)",serial[start+begin.end():])
    if not saved:return None
    saved_at=start+begin.end()+saved.start()
    current=revision if current_revision is None else current_revision
    dirty=int(current!=revision)
    clean=re.search(rf"\[quil.document.saved\] saved_revision={revision} revision={current} dirty={dirty}(?:\s|$)",serial[saved_at+len(saved[0]):])
    if not clean:return None
    return dict(begin=begin_at,saved=saved_at,completed=saved_at+len(saved[0])+clean.start(),
                revision=revision,current_revision=current,dirty=dirty,bytes=len(raw),hash=f"{fnv(raw):#x}")


def watch_save(text, offset, document, payload, stop, observed, errors):
    """Cancellable observer; exceptions reach the controlling thread."""
    try:
        deadline=time.monotonic()+180
        while not stop.is_set() and time.monotonic()<deadline:
            receipt=save_receipt(text(),offset,document,payload)
            if receipt:
                observed.append((time.monotonic(),receipt));return
            stop.wait(.02)
    except Exception as error:
        errors.append(type(error).__name__)


def submit_save(q, wait, start):
    # One explicit chord has no QMP hold-time queue. Observe the physical
    # Control break before admitting ordinary launcher/navigation keys.
    q.cmd("input-send-event", {"events":[
        {"type":"key","data":{"down":down,"key":{"type":"qcode","data":key}}}
        for key,down in [("ctrl",True),("s",True),("s",False),("ctrl",False)]]})
    wait(r"\[quil.document.begin\] kind=save",start)
    wait(r"\[sexinput.ps2.scancode\] raw=0x9d(?:\s|$)",start)


def run(c, seconds, acceptance=True, third_quil=False):
    if isinstance(seconds,bool) or not isinstance(seconds,(int,float)) or not math.isfinite(seconds) or seconds < (1800 if acceptance else 1):
        raise ValueError('alpha acceptance requires at least 1800 seconds')
    q, wait, text = (c[k] for k in ('q', 'wait', 'text'))
    key,focus,capture=(c[k] for k in ('key','focus','capture'))
    out, second = c['out'], c['second']
    samples = []
    cycle_windows=[]
    active_cycle=0
    # Keep the real pointer away from text before latency measurements. A
    # cursor covering SAVE is legitimate occlusion, not a delayed text paint.
    c['park_pointer']()
    capture('soak-pointer-parked','uibeta')
    started = time.monotonic()

    def record(kind, start, observed=None, **fields):
        now=time.monotonic();seen=now if observed is None else observed
        samples.append(dict(sample_id=len(samples)+1,cycle=active_cycle,kind=kind,seconds=seen-start,
                            action_seconds=start-started,session_seconds=seen-started,
                            recorded_seconds=now-started,serial_end=len(text()),**fields))
        (out/'latency.json').write_text(json.dumps(samples, indent=2)+'\n')

    def command(value, target, pattern):
        for i, char in enumerate(value, 1):
            key({' ': 'spc', '-': 'minus'}.get(char, char),
                rf'\[spindle.cursor.view\] target={target} sequence=\d+ pos={i} len={i}(?:\s|$)')
        start = len(text())
        q.key('ret')
        wait(pattern, start)
        return start

    def refresh(phase="teardown"):
        key('grave_accent', r'\[shell.palette.item\] idx=0')
        key('j', r'\[shell.palette.select\] old=0 new=1')
        key('j', r'\[shell.palette.select\] old=1 new=2')
        start = len(text())
        sent = time.monotonic()
        q.key_sequence(['ret', 'j'])
        selected=wait(r'\[shell.action.select_next_linen\] id=[1-9]\d*', start)
        input_offset=start+selected.start()
        record('linen_input_receipt',sent,phase=phase,serial_start=start,receipt_offset=input_offset)
        snapshot=wait(r'\[linen.remote.snapshot.ok\].*complete=1', start)
        wait(r'\[linen.object_select.current\]', start)
        record('linen_refresh',sent,phase=phase,serial_start=start)
        return dict(serial_start=start,input=input_offset,snapshot=start+snapshot.start())

    def close(target, sid):
        focus(sid)
        start=len(text());sent=time.monotonic()
        key('f11',rf'\[shell.app.close.request\] target={target} task={target}(?:\s|$)')
        wait(rf'\[sext.app.pool.ready\] target={target} task={target} .*ok=1',start,timeout=180)
        record('close_to_zero_reuse',sent,target=target,task=target,serial_start=start)

    cycle = 0
    while time.monotonic()-started < seconds or cycle < (12 if acceptance else 1):
        cycle += 1
        active_cycle=cycle
        cycle_start=time.monotonic();cycle_serial_start=len(text())
        focus(201)
        input_offset=len(text());sent=time.monotonic()
        key('q', r'\[quil.document.edit\]')
        record('quil_input_receipt',sent,serial_start=input_offset)
        second['content'] += 'q'
        wait(r'\[quil.ui.view\].*cursor='+str(len(second['content']))+r' ',input_offset,timeout=30)
        record('quil_view_receipt',sent,serial_start=input_offset)
        capture(f'soak-{cycle:04d}-current','#')
        record('quil_input_pixels',sent,serial_start=input_offset)
        sent = time.monotonic()
        save_offset=len(text())
        observed=[];observer_errors=[];stop=threading.Event()
        watcher=threading.Thread(target=watch_save,args=(text,save_offset,second['object'],
            second['content'],stop,observed,observer_errors),daemon=True)
        watcher.start()
        try:
            submit_save(q,wait,save_offset)
            # Observe actual overlap; an admitted save can finish before the
            # Linen action, so admission alone never proves concurrency.
            linen=refresh(phase="save_admission")
            wait(r'\[quil.document.saved\].*dirty=0',save_offset)
            watcher.join(timeout=1)
            if observer_errors or not observed:raise RuntimeError('exact save timing observer failed: '+str(observer_errors))
            receipt=observed[0][1]
            record('warm_save_receipt',sent,observed=observed[0][0],observer_poll_seconds=.02,
                   serial_start=save_offset,save=receipt,linen_input_offset=linen['input'],
                   linen_input_before_save_completion=linen['input']<receipt['completed'])
        finally:
            stop.set();watcher.join(timeout=1)
            if watcher.is_alive():raise RuntimeError('save timing observer did not stop')
        focus(201)
        capture(f'soak-{cycle:04d}-saved','uibeta')
        record('warm_save_pixels',sent,serial_start=save_offset,
               linen_actions_after_admission=True,
               includes_concurrent_linen_actions=linen['input']<receipt['completed'])
        second.update(bytes=receipt['bytes'],hash=receipt['hash'])
        # Mixed storage and command traffic uses the live Spindle main loop.
        focus(153)
        sent = time.monotonic()
        stat_offset=command('disk-stat 0', 12, r'\[spindle.probe.result\] target=12 kind=done job=Stat\(0\).*error=0')
        record('spindle_stat',sent,serial_start=stat_offset)
        offset = command('echo alpha', 12, r'\[spindle.instance.command\] target=12 .*name=echo ok=1')
        if cycle == 1:
            # The shipped SexTime calculator: exact conversion of a typed value.
            offset=command('sextime 1356048000', 12, r'\[spindle.sextime\] SexTime 0 s \| UTC 2012-12-21T00:00:00Z \| Long Count 13\.0\.0\.0\.0 \(GMT 584283\) ok=1')
        wait(r'\[spindle.history.receipt\] target=12 .*kind=saved .*error=0', offset)
        refresh(phase="cycle_end")
        (out/'progress.json').write_text(json.dumps(dict(cycles=cycle, elapsed_seconds=time.monotonic()-started,
            documents=[c['first'], second], last_activity='saved, Spindle stat/history, Linen refresh'))+'\n')
        print(f'[alpha.session.progress] cycles={cycle} elapsed={time.monotonic()-started:.1f}', flush=True)
        cycle_windows.append(dict(cycle=cycle,start_seconds=cycle_start-started,
            end_seconds=time.monotonic()-started,serial_start=cycle_serial_start,serial_end=len(text())))
        # At most 15 seconds deliberately idle between active cycles.
        end = min(started+seconds, time.monotonic()+15)
        while time.monotonic() < end:
            time.sleep(min(.2, end-time.monotonic()))
    elapsed = time.monotonic()-started
    active_cycle=0
    active_elapsed=sum(w["end_seconds"]-w["start_seconds"] for w in cycle_windows)
    # Ordered hardware events attempt Close during a save; receipts below
    # must independently prove that the first Close actually found it pending.
    focus(201)
    start = len(text())
    sent = time.monotonic()
    q.cmd('input-send-event', {'events': [
        {'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}
        for k,down in [('ctrl',True),('s',True),('s',False),('ctrl',False),
                       ('z',True),('z',False),('f11',True),('f11',False)]]})
    # Unsaved 'z' plus the admitted save: Quil declines the ordinary Close
    # and keeps the work; only the user's second, confirmed Close discards.
    declined=wait(r'\[quil.close.declined\] dirty=1 pending=([01]) revision=(\d+)',start)
    shell_declined=wait(r'\[shell.app.close.declined\] target=9 task=9 reason=unsaved next_close=confirmed',start)
    close_facts=dict(attempt_start=start,declined=start+declined.start(),
        shell_declined=start+shell_declined.start(),declined_pending=int(declined[1]),
        dirty_revision=int(declined[2]))
    attempt=text()[start:close_facts['declined']]
    begin=re.search(rf'\[quil.document.begin\] kind=save object={second["object"]} revision=(\d+) bytes={len(second["content"].encode("utf8"))}(?:\s|$)',attempt)
    edit=re.search(rf'\[quil.document.edit\] revision={close_facts["dirty_revision"]} pending=1 bytes={len(second["content"].encode("utf8"))+1}(?:\s|$)',attempt[begin.end():]) if begin else None
    if not begin or not edit or int(begin[1])+1!=close_facts['dirty_revision']:
        raise RuntimeError('pending-save immutable snapshot/edit before declined Close not proved')
    close_facts.update(save_begin=start+begin.start(),dirty_edit=start+begin.end()+edit.start(),
        save_revision=int(begin[1]),document=second['object'],snapshot_bytes=len(second['content'].encode('utf8')),
        snapshot_hash=f"{fnv(second['content'].encode('utf8')):#x}")
    record('dirty_close_declined',sent,serial_start=start,close_facts=dict(close_facts))
    if close_facts['declined_pending']!=1:
        raise RuntimeError('ordinary Close found save already completed; pending-save guard not proved')
    capture('close-declined', 'UNSAVED')
    if re.search(r'\[quil.document.close\]', text()[start:]):raise RuntimeError('declined close still closed Quil')
    confirm = len(text())
    key('f11',r'\[quil.close.confirmed\] dirty=1 pending=([01])')
    closed = wait(r'\[quil.document.close\] pending=(\d)', confirm)
    pending_at_close=int(closed[1])
    confirmed=re.search(r'\[quil.close.confirmed\] dirty=1 pending=([01])',text()[confirm:])
    close_facts.update(confirm_start=confirm,confirmed=confirm+confirmed.start(),
        confirmed_pending=int(confirmed[1]),document_close=confirm+closed.start(),
        terminal_pending=pending_at_close,terminal_scope='pending_io_teardown' if pending_at_close else 'safe_completed_save_close')
    close_facts['save_complete']=save_receipt(text()[:close_facts['document_close']],start,
        second['object'],second['content'],current_revision=close_facts['dirty_revision'])
    if not pending_at_close and close_facts['save_complete'] is None:
        raise RuntimeError('nonpending confirmed Close lacks positive exact save completion')
    wait(r'\[sext.app.pool.ready\] target=9 task=9 .*ok=1',confirm)
    record('quil_confirmed_close_to_zero_reuse',sent,serial_start=start,close_facts=dict(close_facts))
    key('grave_accent', r'\[shell.palette.item\] idx=0')
    key('j', r'\[shell.palette.select\] old=0 new=1')
    relaunch_start=len(text())
    key('ret',r'\[shell.app.relaunch\] predecessor=9 successor=16 ')
    wait(r'\[app.surface.presented\] target=16 surface=301 .*ok=1',relaunch_start)
    # Reopen both exact saved objects in the fresh BOOT incarnation via Linen.
    refresh()
    for expected in [c['first'], second]:
        focus(200)
        for _ in range(20):
            key('j', r'\[shell.action.select_next_linen\] id=\d+')
            from quil_third_probe import open_selected
            observed=open_selected(c)
            if observed and observed['object']==expected['object']:
                if (observed['bytes'],observed['hash'])!=(expected['bytes'],expected['hash']):
                    raise RuntimeError('successor document bytes changed')
                break
            focus(200)
        else:
            raise RuntimeError('successor could not select '+expected['name'])
    close(16,301)
    third=None
    if third_quil:
        from quil_third_probe import run as third_probe
        from quil_reserve_catalog import quil_catalog,verify_boot_quil
        rows=quil_catalog(c["root"]);verify_boot_quil(text(),rows)
        third=third_probe(c,refresh,close,rows)
    close(15,300)
    # Prove pending Spindle job retirement and its separately reserved successor.
    focus(153)
    offset = command('mkdoc soakprobe',12,r'\[spindle.probe.result\] target=12 kind=done job=Create\([^\r\n]*\) value=\d+ error=0')
    obj = int(re.search(r'job=Create\([^\r\n]*\) value=(\d+)', text()[offset:])[1])
    offset = command(f'filldocx {obj} 65536',12,r'\[files.probe.bound\] target=12 ')
    key('x',r'\[spindle.probe.input\] target=12 pending=1 ')
    key('f11',r'\[spindle.probe.closed\] target=12 pending=1 terminal=1')
    wait(r'\[files.probe.retire\] target=12 lease=0 pending=0 terminal=1',offset)
    wait(r'\[sext.app.pool.ready\] target=12 task=12 .*ok=1',offset)
    key('grave_accent',r'\[shell.palette.item\] idx=0')
    relaunch_start=len(text())
    key('ret',r'\[shell.app.relaunch\] predecessor=12 successor=17 ')
    wait(r'\[spindle.history.receipt\] target=17 .*kind=loaded .*error=0',relaunch_start)
    focus(306)
    offset = command('echo alpha17',17,r'\[spindle.instance.command\] target=17 .*name=echo ok=1')
    wait(r'\[spindle.history.receipt\] target=17 .*kind=saved .*error=0',offset)
    close(17,306)
    refresh()
    key('j',r'\[shell.action.select_next_linen\] id=[1-9]\d*')
    from check_desktop_close_log import check
    errors = check(text(),include_successor=True,include_spindle_successor=True,include_third_quil=third_quil,additional_quil=rows[3:]if third_quil else ())
    if errors:
        raise RuntimeError('sustained cleanup: '+str(errors))
    return dict(evidence_version=2,acceptance=acceptance,elapsed_seconds=elapsed,cycles=cycle,samples=samples,
                cycle_windows=cycle_windows,active_elapsed_seconds=active_elapsed,idle_elapsed_seconds=elapsed-active_elapsed,
                pending_save_close_guard=True,quil_close=close_facts,
                pending_quil_close=bool(pending_at_close),pending_spindle_close=True,
                scope='pending-save unsaved Close guard; exact confirmed teardown state',
                remaining_obligations=[] if pending_at_close else ['Quil pending-I/O terminal teardown'],
                boot_successors=([16,17]+[row["target"]for row in rows[2:]]) if third_quil else [16,17],third_quil=third,cleanup_errors=errors)
