#!/usr/bin/env python3
"""One program reads its neighbour across a hardware key; only the reader dies. Needs an ISO built with SEXOS_PROGRAM_ISOLATION_PROBE=1."""
import sys,time,re,shutil
from pathlib import Path
from quil_autosave_gate import boot
from quil_workflow_gate import palette_select,palette_commands
ROOT=Path(__file__).resolve().parents[1]
iso,seed,out=Path(sys.argv[1]),Path(sys.argv[2]),Path(sys.argv[3])
out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(seed,disk)
g=boot(iso,disk,out,'p','1280x800');t=g['text'];q=g['q']
try:
 g['wait'](r'\[app.private.retired\] target=41 ',0,120)
 palette_select(g['key'],'FocusProgram',palette_commands(ROOT));s=len(t());q.key('ret');print(g['wait'](r'\[hello.ready\][^\n]*',s,60)[0]);time.sleep(2)
 palette_select(g['key'],'FocusProgram',palette_commands(ROOT));s=len(t());q.key('ret')
 print(g['wait'](r'\[hello.probe.own\][^\n]*',s,60)[0])
 m=g['wait'](r'\[paging.fault.trap\] pd=43[^\n]*',s,60);print(m[0]);print(g['wait'](r'fault.kill[^\n]*',s,30)[0])
 print(g['wait'](r'\[app.private.retired\] target=43[^\n]*',s,60)[0][:80]);time.sleep(2)
 print('breach' , 'hello.probe.breach' in t())
 palette_select(g['key'],'FocusProgram',palette_commands(ROOT));s=len(t());q.key('ret');print(g['wait'](r'\[hello.ready\][^\n]*',s,60)[0]);time.sleep(2)
 q.cmd('screendump',{'filename':str(out/'p.ppm')});time.sleep(.5)
 g['focus'](359);s=len(t());q.key('f11');print(g['wait'](r'\[hello.closed\][^\n]*',s,60)[0])
finally:g['stop']()
