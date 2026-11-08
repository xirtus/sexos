#!/usr/bin/env python3
"""A program that never answers Close is ended after five seconds, cleaned up like any exit, and its row runs again. Needs an ISO built with SEXOS_PROGRAM_HANG_PROBE=1."""
import sys,time,re,shutil
from pathlib import Path
from quil_autosave_gate import boot
from quil_workflow_gate import palette_select,palette_commands
ROOT=Path(__file__).resolve().parents[1]
iso,seed,out=Path(sys.argv[1]),Path(sys.argv[2]),Path(sys.argv[3])
out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(seed,disk)
g=boot(iso,disk,out,'force','1280x800');t=g['text'];q=g['q']
def open_program():
 palette_select(g['key'],'FocusProgram',palette_commands(ROOT));s=len(t());q.key('ret')
 m=g['wait'](r'\[hello.ready\] target=(\d+) task=(\d+) surface=(\d+)',s,60);time.sleep(2);return m
try:
 g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
 m=open_program();g['focus'](int(m[3]));time.sleep(.5)
 s=len(t());q.key('f11');began=time.monotonic()
 print(g['wait'](rf'\[shell.app.force\] target={m[1]} task={m[2]} ok=1',s,60)[0])
 print(g['wait'](rf'\[app.program.forced\] target={m[1]} task={m[2]}[^\n]*',s,30)[0])
 print(g['wait'](rf'fault.kill[^\n]*pd={m[1]}[^\n]*',s,30)[0])
 print(g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} [^\n]*',s,90)[0][:90])
 print('seconds to end',round(time.monotonic()-began,1))
 again=open_program();print('runs again',again[0])
 assert 'KERNEL PANIC' not in t()
 print({'ok':True})
finally:g['stop']()
