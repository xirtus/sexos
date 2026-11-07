#!/usr/bin/env python3
"""One program reads its neighbour; only the reader dies. Needs an ISO built with SEXOS_PROGRAM_ISOLATION_PROBE=1 (row 43 reads 42, across keys)
or =4 with a fourth argument `4` (row 45 reads 44, which shares its key but is absent while 45 runs)."""
import sys,time,re,shutil
from pathlib import Path
from quil_autosave_gate import boot
from quil_workflow_gate import palette_select,palette_commands
ROOT=Path(__file__).resolve().parents[1]
iso,seed,out=Path(sys.argv[1]),Path(sys.argv[2]),Path(sys.argv[3])
ordinal=int(sys.argv[4]) if len(sys.argv)>4 else 2
READER,FIRST=41+ordinal,40+ordinal
out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(seed,disk)
g=boot(iso,disk,out,'p','1280x800');t=g['text'];q=g['q']
try:
 g['wait'](r'\[app.private.retired\] target=41 ',0,120)
 for _ in range(ordinal-1):
  palette_select(g['key'],'FocusProgram',palette_commands(ROOT));s=len(t());q.key('ret');print(g['wait'](r'\[hello.ready\][^\n]*',s,60)[0]);time.sleep(2)
 palette_select(g['key'],'FocusProgram',palette_commands(ROOT));s=len(t());q.key('ret')
 print(g['wait'](r'\[hello.probe.own\][^\n]*',s,60)[0])
 m=g['wait'](rf'\[paging.fault.trap\] pd={READER}[^\n]*',s,60);print(m[0]);print(g['wait'](r'fault.kill[^\n]*',s,30)[0])
 print(g['wait'](rf'\[app.private.retired\] target={READER}[^\n]*',s,60)[0][:80]);time.sleep(2)
 print('breach' , 'hello.probe.breach' in t())
 palette_select(g['key'],'FocusProgram',palette_commands(ROOT));s=len(t());q.key('ret');print(g['wait'](r'\[hello.ready\][^\n]*',s,60)[0]);time.sleep(2)
 q.cmd('screendump',{'filename':str(out/'p.ppm')});time.sleep(.5)
 g['focus'](359);s=len(t());q.key('f11');print(g['wait'](r'\[hello.closed\][^\n]*',s,60)[0])
finally:g['stop']()
