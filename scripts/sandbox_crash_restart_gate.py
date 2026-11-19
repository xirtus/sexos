#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Sandbox crash/restart qualification on a real image (QEMU).

1. A parent (moanrack) admits a restricted offline helper, waits for the
   authenticated Started, then faults with the helper still live.
2. The kernel ends the parent; Shell retires it and kills its helper; Files
   scrubs the parent's grants and drops its immutable job entry.
3. A new parent run (which may reuse the crashed parent's catalog row) holds
   the crashed parent's exact ticket, nonce and epochs and gets nothing: no
   result, no job entry left to cancel, and no helper START.
4. A fresh helper job then runs to a verified result, so the helper pool slot
   comes back clean.

Ordinary fault-isolation testing of SexOS's own processes. No hostile-plugin
containment claim beyond what these receipts show.
"""
import argparse, hashlib, json, os, re, shutil, time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
from moan_gate_geometry import spindle
from program_gate_geometry import maximize, current_tile

ROOT = Path(__file__).resolve().parents[1]
RELEASE = ROOT / 'target/x86_64-sex-app/release'


def sha(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--iso', type=Path, required=True)
    p.add_argument('--source-disk', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    a = p.parse_args()
    out = a.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    if any(out.iterdir()):
        raise ValueError('fresh evidence directory required')
    programs = [('moanrack', RELEASE / 'moanrack'), ('moanplug', RELEASE / 'moanplug')]
    inputs = {str(x): sha(x) for x in [a.iso, a.source_disk, *[v for _, v in programs], Path(__file__)]}
    (out / 'inputs.json').write_text(json.dumps(inputs, indent=2) + '\n')
    disk = out / 'nvme.img'
    shutil.copyfile(a.source_disk, disk)
    install(disk, [(n, v.read_bytes()) for n, v in programs])
    os.environ.setdefault('SEXOS_GATE_BOUND', '700')
    g = boot(a.iso, disk, out, 'sandbox-crash', '1280x800')
    text, q = g['text'], g['q']
    result = {'scope': 'parent crash with a live restricted helper; stale admission after restart; clean helper slot reuse'}
    try:
        g['wait'](r'\[app.private.retired\] target=41 ', 0, 120)

        # 1. Crash the parent with its helper live.
        s = spindle(g, 'run moanrack @/data/moanrack --crash-parent')
        m = g['wait'](r'\[moanrack.parent.crash\] ok=1 parent=(\d+) task=(\d+) ticket=(\d+) cookie=(\d+) child=(\d+) child_task=(\d+)', s, 240)
        parent, task, ticket, cookie, child, child_task = map(int, m.groups())
        assert child != parent
        g['wait'](rf'\[moan.role.started\] parent={parent} parent_task={task} cookie={cookie} pd={child} task={child_task}', s, 10)
        crash_at = s + m.end()
        g['wait'](rf'\[cpu.fault.terminal\] pd={parent} kind=invalid_opcode', crash_at, 60)

        # 2. Both domains retire; the helper is ended by Shell, not by its 120 s deadline.
        g['wait'](rf'\[app.private.retired\] target={parent} task={task} ', crash_at, 120)
        g['wait'](rf'\[app.private.retired\] target={child} task={child_task} ', crash_at, 60)
        retired_at = len(text())
        tail = text()[crash_at:retired_at]
        assert '[moanplug.native]' not in tail or '[moanplug.rendered]' not in tail, 'helper finished instead of being ended'
        cleanup = re.search(rf'\[files.app.cleanup\] target={parent} task={task} handles=(\d+) caps=(\d+)', text()[crash_at:])
        result['files_cleanup'] = cleanup.group(0) if cleanup else None
        assert 'KERNEL PANIC' not in text() and '[shell.fatal]' not in text()

        # 3. A new parent holding the dead parent's exact admission gets nothing.
        s = spindle(g, 'run moanrack @/data/moanrack --stale-probe')
        n = g['wait'](r'\[moanrack.stale\] ok=1 ticket=(\d+) same_files_epoch=(\d) result=(\d) cancel=(\d+) start=(0x[0-9a-f]+) started=(\d+) refused=(\d+)', s, 120)
        stale = dict(zip(['ticket', 'same_files_epoch', 'result', 'cancel', 'start', 'started', 'refused'], n.groups()))
        assert int(stale['ticket']) == ticket
        assert stale['result'] == '0', 'stale parent read a result'
        assert stale['started'] == '0', 'stale START launched a helper'
        # Files keeps no entry: an absent ticket is acknowledged OK0, a leaked one is denied.
        assert stale['cancel'] == '0', f'job entry outlived its parent (cancel={stale["cancel"]})'
        probe = re.search(r'\[shell.program.launch\] target=(\d+) task=(\d+)', text()[s:s + n.end()])
        between = text()[s:s + n.end()]
        assert '[moan.role.reserved]' not in between and '[moan.role.started]' not in between, 'stale START reached the helper pool'
        if probe:
            stale['row'], stale['task'] = map(int, probe.groups())
            stale['reused_parent_row'] = stale['row'] == parent
            assert not stale['reused_parent_row'] or stale['task'] != task
        result['stale'] = stale

        # 4. A fresh job through the same pool renders and verifies.
        s = spindle(g, 'run moanrack @/data/moanrack')
        r = g['wait'](r'\[moanrack.ready\] target=(\d+) task=(\d+)', s, 180)
        fresh, fresh_task = map(int, r.groups())
        sid = int(g['wait'](rf'\[shell.program.launch\] target={fresh} task={fresh_task} surface=(\d+)', s, 60)[1])
        tile, start = maximize(g, sid)
        w, h = tile[2] - 16, tile[3] - 42
        g['wait'](rf'\[moanrack.control\] id=0 x=\d+ y=\d+ w={w} h={h}', start, 120)
        tile = current_tile(text(), sid)
        x, y = map(int, re.findall(rf'\[moanrack.control\] id=0 x=(-?\d+) y=(-?\d+) w={w} h={h}', text())[-1])
        x += tile[0] + 8
        y += tile[1] + 34
        s = len(text())
        g['focus'](sid)
        q.cmd('input-send-event', {'events': [{'type': 'abs', 'data': {'axis': 'x', 'value': round(x * 32767 / 1279)}}, {'type': 'abs', 'data': {'axis': 'y', 'value': round(y * 32767 / 799)}}]})
        time.sleep(.1)
        for down in (True, False):
            q.cmd('input-send-event', {'events': [{'type': 'btn', 'data': {'down': down, 'button': 'left'}}]})
            time.sleep(.1)
        c = g['wait'](r'\[moanrack.started\] mode=0 cookie=(\d+)', s, 60)
        st = g['wait'](rf'\[moan.role.started\] parent={fresh} parent_task={fresh_task} cookie={c[1]} pd=(\d+) task=(\d+)', s, 120)
        new_child, new_child_task = map(int, st.groups())
        g['wait'](rf'\[moanrack.result\] mode=0 cookie={c[1]} complete=1 processed=1 retired=1', s, 180)
        assert (new_child, new_child_task) != (child, child_task), 'helper identity reused'
        result['fresh'] = dict(parent=fresh, task=fresh_task, child=new_child, child_task=new_child_task,
                               reused_helper_row=new_child == child)
        s = len(text())
        g['focus'](sid)
        q.key('f11')
        g['wait'](r'\[moanrack.closed\]', s, 60)
        g['wait'](rf'\[app.private.retired\] target={fresh} task={fresh_task} ', s, 120)
        assert 'KERNEL PANIC' not in text() and 'panicked' not in text()
        result.update(ok=True, parent=parent, task=task, ticket=ticket, child=child, child_task=child_task)
        q.cmd('quit')
        g['proc'].wait(30)
        assert all(sha(x) == v for x, v in inputs.items()), 'immutable input changed'
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result))
    finally:
        g['stop']()


if __name__ == '__main__':
    main()
