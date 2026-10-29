"""Sustained real-input session extension for quil_workflow_gate (no builds)."""
import hashlib
import json
import re
import time
import threading


def run(c, seconds, acceptance=True, third_quil=False):
    if seconds < (1800 if acceptance else 1):
        raise ValueError('alpha acceptance requires at least 1800 seconds')
    q, wait, text = (c[k] for k in ('q', 'wait', 'text'))
    key, focus, capture, ctrl = (c[k] for k in ('key', 'focus', 'capture', 'ctrl'))
    out, second = c['out'], c['second']
    samples = []
    # Keep the real pointer away from text before latency measurements. A
    # cursor covering SAVE is legitimate occlusion, not a delayed text paint.
    c['park_pointer']()
    capture('soak-pointer-parked','uibeta')
    started = time.monotonic()

    def record(kind, start, observed=None, **fields):
        samples.append(dict(kind=kind, seconds=(time.monotonic() if observed is None else observed)-start,
                            session_seconds=time.monotonic()-started, **fields))
        (out/'latency.json').write_text(json.dumps(samples, indent=2)+'\n')

    def command(value, target, pattern):
        for i, char in enumerate(value, 1):
            key({' ': 'spc', '-': 'minus'}.get(char, char),
                rf'\[spindle.cursor.view\] target={target} sequence=\d+ pos={i} len={i}(?:\s|$)')
        start = len(text())
        q.key('ret')
        wait(pattern, start)
        return start

    def refresh():
        key('grave_accent', r'\[shell.palette.item\] idx=0')
        key('j', r'\[shell.palette.select\] old=0 new=1')
        key('j', r'\[shell.palette.select\] old=1 new=2')
        start = len(text())
        sent = time.monotonic()
        q.key_sequence(['ret', 'j'])
        wait(r'\[shell.action.select_next_linen\] id=[1-9]\d*', start)
        record('linen_input_receipt', sent)
        wait(r'\[linen.remote.snapshot.ok\].*complete=1', start)
        wait(r'\[linen.object_select.current\]', start)
        record('linen_refresh', sent)

    def close(target, sid):
        focus(sid)
        sent = time.monotonic()
        key('f11', rf'\[shell.app.close.request\] target={target} ')
        wait(rf'\[sext.app.pool.ready\] target={target} .*ok=1', timeout=180)
        record('close_to_zero_reuse', sent, target=target)

    cycle = 0
    while time.monotonic()-started < seconds:
        cycle += 1
        focus(201)
        sent = time.monotonic()
        key('q', r'\[quil.document.edit\]')
        record('quil_input_receipt', sent)
        second['content'] += 'q'
        wait(r'\[quil.ui.view\].*cursor='+str(len(second['content']))+r' ', timeout=30)
        capture('soak-current', '#')
        record('quil_input_pixels', sent)
        sent = time.monotonic()
        save_offset=len(text())
        observed=[]
        def watch_save():
            end=time.monotonic()+180
            while time.monotonic()<end:
                if re.search(r'\[quil.document.saved\].*dirty=0',text()[save_offset:]):
                    observed.append(time.monotonic());return
                time.sleep(.02)
        watcher=threading.Thread(target=watch_save,daemon=True);watcher.start()
        q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'s'}],'hold-time':80})
        wait(r'\[quil.document.begin\] kind=save',save_offset)
        # A real Linen rescan/input overlaps the admitted document save.
        refresh()
        wait(r'\[quil.document.saved\].*dirty=0',save_offset)
        watcher.join(timeout=1)
        if not observed:raise RuntimeError('save timing observer failed')
        record('warm_save_receipt',sent,observed=observed[0],observer_poll_seconds=.02)
        focus(201)
        capture('soak-saved', 'uibeta')
        record('warm_save_pixels', sent,includes_concurrent_linen_actions=True)
        last = re.findall(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)', text())[-1]
        second.update(bytes=int(last[0]), hash=last[1])
        # Mixed storage and command traffic uses the live Spindle main loop.
        focus(153)
        sent = time.monotonic()
        command('disk-stat 0', 12, r'\[spindle.probe.result\] target=12 kind=done job=Stat\(0\).*error=0')
        record('spindle_stat', sent)
        offset = command('echo alpha', 12, r'\[spindle.instance.command\] target=12 .*name=echo ok=1')
        wait(r'\[spindle.history.receipt\] target=12 .*kind=saved .*error=0', offset)
        refresh()
        (out/'progress.json').write_text(json.dumps(dict(cycles=cycle, elapsed_seconds=time.monotonic()-started,
            documents=[c['first'], second], last_activity='saved, Spindle stat/history, Linen refresh'))+'\n')
        print(f'[alpha.session.progress] cycles={cycle} elapsed={time.monotonic()-started:.1f}', flush=True)
        # At most 15 seconds between active cycles; no idle soak substitute.
        end = min(started+seconds, time.monotonic()+15)
        while time.monotonic() < end:
            time.sleep(min(.2, end-time.monotonic()))
    elapsed = time.monotonic()-started
    # Ordered hardware events prove Close while an admitted save is pending.
    focus(201)
    start = len(text())
    sent = time.monotonic()
    q.cmd('input-send-event', {'events': [
        {'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}
        for k,down in [('ctrl',True),('s',True),('s',False),('ctrl',False),
                       ('z',True),('z',False),('f11',True),('f11',False)]]})
    wait(r'\[quil.document.close\] pending=1', start)
    wait(r'\[sext.app.pool.ready\] target=9 .*ok=1', start)
    record('pending_quil_close_to_zero_reuse', sent)
    key('grave_accent', r'\[shell.palette.item\] idx=0')
    key('j', r'\[shell.palette.select\] old=0 new=1')
    key('ret', r'\[shell.app.relaunch\] predecessor=9 successor=16 ')
    wait(r'\[app.surface.presented\] target=16 surface=301 .*ok=1')
    # Reopen both exact saved objects in the fresh BOOT incarnation via Linen.
    refresh()
    for expected in [c['first'], second]:
        focus(200)
        for _ in range(20):
            key('j', r'\[shell.action.select_next_linen\] id=\d+')
            offset = len(text())
            q.key('ret')
            end = time.monotonic()+1
            while time.monotonic()<end and '[linen.quil.disk_doc.intent]' not in text()[offset:]:
                time.sleep(.05)
            m = re.search(r'\[linen.quil.disk_doc.intent\] object_id=\d+ path_id=(\d+) ok=1', text()[offset:])
            if m and int(m[1]) == expected['object']:
                loaded = wait(r'\[quil.persist.load.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc='+str(expected['object']), offset)
                if (int(loaded[1]),loaded[2]) != (expected['bytes'],expected['hash']):
                    raise RuntimeError('successor document bytes changed')
                break
            focus(200)
        else:
            raise RuntimeError('successor could not select '+expected['name'])
    close(16,301)
    third=None
    if third_quil:
        from quil_third_probe import run as third_probe
        third=third_probe(c,refresh,close)
    close(15,300)
    # Prove pending Spindle job retirement and its separately reserved successor.
    focus(153)
    offset = command('mkdoc soakprobe',12,r'\[spindle.probe.result\] target=12 kind=done job=Create\([^\r\n]*\) value=\d+ error=0')
    obj = int(re.search(r'job=Create\([^\r\n]*\) value=(\d+)', text()[offset:])[1])
    offset = command(f'filldocx {obj} 65536',12,r'\[files.probe.bound\] target=12 ')
    key('x',r'\[spindle.probe.input\] target=12 pending=1 ')
    key('f11',r'\[spindle.probe.closed\] target=12 pending=1 terminal=1')
    wait(r'\[files.probe.retire\] target=12 lease=0 pending=0 terminal=1',offset)
    wait(r'\[sext.app.pool.ready\] target=12 .*ok=1',offset)
    key('grave_accent',r'\[shell.palette.item\] idx=0')
    key('ret',r'\[shell.app.relaunch\] predecessor=12 successor=17 ')
    wait(r'\[spindle.history.receipt\] target=17 .*kind=loaded .*error=0')
    focus(306)
    offset = command('echo alpha17',17,r'\[spindle.instance.command\] target=17 .*name=echo ok=1')
    wait(r'\[spindle.history.receipt\] target=17 .*kind=saved .*error=0',offset)
    close(17,306)
    refresh()
    key('j',r'\[shell.action.select_next_linen\] id=[1-9]\d*')
    from check_desktop_close_log import check
    errors = check(text(),include_successor=True,include_spindle_successor=True,include_third_quil=third_quil)
    if errors:
        raise RuntimeError('sustained cleanup: '+str(errors))
    return dict(acceptance=acceptance,elapsed_seconds=elapsed, cycles=cycle, samples=samples,
                pending_quil_close=True,pending_spindle_close=True,
                boot_successors=[16,17,18] if third_quil else [16,17],third_quil=third,cleanup_errors=errors)
