#!/usr/bin/env python3
"""Super key window management on a booted desktop: layouts, workspaces,
moving a window to another workspace, focus by direction, floating, the
scratchpad and the Super tap launcher. Screens are saved for review."""
import argparse,re,shutil,time
from pathlib import Path
from quil_autosave_gate import boot
from quil_workflow_gate import palette_select,palette_commands
ROOT=Path(__file__).resolve().parents[1]
ACTION=r'\[shell.wm.action\] action={} done=(\d) scene=(\d)->(\d) focus=(\d+)->(\d+) layout=(\w+)'
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 g=boot(a.iso,disk,a.out,'wm',a.resolution);t=g['text'];q=g['q']
 def send(down,keys):q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}for k in keys]});time.sleep(.08)
 def chord(keys,action):
  """Press keys in order, release in reverse; wait for the shell's record of the action."""
  s=len(t())
  for k in keys:send(True,[k])
  for k in reversed(keys):send(False,[k])
  m=g['wait'](ACTION.format(re.escape(action)),s,30)
  return dict(done=m[1]=='1',scene=(int(m[2]),int(m[3])),focus=(int(m[4]),int(m[5])),layout=m[6])
 def shot(name):time.sleep(1.5);q.cmd('screendump',{'filename':str(a.out/f'{name}.ppm')});time.sleep(.5)
 def tiles(s):return {int(m[0]):tuple(map(int,m[1:]))for m in re.findall(r'\[shell.tile.apply\] sid=(\d+) x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t()[s:])}
 results={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(3)
  # A second window so there is something to tile next to Quil.
  s=len(t());palette_select(g['key'],'FocusProgram',palette_commands(ROOT));q.key('ret')
  g['wait'](r'\[hello.ready\]',s,60);g['wait'](r'\[shell.tile.apply\] sid=359',s,60);time.sleep(2)
  base=tiles(0);assert 201 in base and 359 in base,base
  # Gaps: tiled windows keep apart from each other and from the screen edge.
  q1,h1=base[201],base[359]
  assert q1[0]>0 and q1[1]>50,('no outer gap',q1)
  assert h1[0]-(q1[0]+q1[2])>0 or q1[0]-(h1[0]+h1[2])>0,('no inner gap',q1,h1)
  shot('01-master-stack')
  # Super+Space: the next layout.
  r=chord(['meta_l','spc'],'CycleLayout');assert r['done'] and r['layout']=='dwindle',r;results['layout']=r['layout']
  r=chord(['meta_l','m'],'ToggleMonocle');assert r['layout']=='monocle',r
  shot('02-monocle')
  r=chord(['meta_l','m'],'ToggleMonocle');assert r['layout']=='dwindle',r
  # Super+H / Super+L: focus by direction.
  r=chord(['meta_l','h'],'Focus(Left)');left=r['focus'][1]
  r=chord(['meta_l','l'],'Focus(Right)');assert r['done'] and r['focus'][1]!=left,r;results['focus']=r['focus']
  # Super+Shift+H: swap places.
  s=len(t());r=chord(['meta_l','shift','h'],'Swap(Left)');assert r['done'],r
  swapped=tiles(s);assert swapped,'no retile after swap';results['swap']=swapped
  shot('03-swapped')
  # Super+V: float the focused window, centred.
  r=chord(['meta_l','v'],'ToggleFloating');assert r['done'],r
  shot('04-floating')
  r=chord(['meta_l','v'],'ToggleFloating');assert r['done'],r
  # Super+] grows the master column.
  r=chord(['meta_l','spc'],'CycleLayout');r=chord(['meta_l','spc'],'CycleLayout');r=chord(['meta_l','spc'],'CycleLayout')
  assert r['layout']=='master',r
  s=len(t());r=chord(['meta_l','bracket_right'],'GrowMaster');assert r['done'],r
  assert re.search(r'\[shell.wm.layout\] scene=0 layout=master windows=\d+ master_pct=60',t()[s:]),'ratio not applied'
  # Super+2 and back; Super+Shift+3 sends the focused window away.
  r=chord(['meta_l','2'],'Workspace(1)');assert r['done'] and r['scene']==(0,1),r
  shot('05-workspace-2')
  r=chord(['meta_l','1'],'Workspace(0)');assert r['scene']==(1,0),r
  r=chord(['meta_l','shift','3'],'MoveToWorkspace(2)');assert r['done'],r;results['moved_from_focus']=r['focus'][0]
  shot('06-after-move')
  r=chord(['meta_l','3'],'Workspace(2)');assert r['scene']==(0,2),r
  shot('07-workspace-3')
  r=chord(['meta_l','ctrl','left'],'PrevWorkspace');assert r['scene']==(2,1),r
  r=chord(['meta_l','tab'],'NextWorkspace');assert r['scene']==(1,2),r
  # Scratchpad: send away, bring back on another workspace.
  r=chord(['meta_l','shift','s'],'ToScratchpad');assert r['done'],r
  r=chord(['meta_l','1'],'Workspace(0)')
  r=chord(['meta_l','s'],'Scratchpad');assert r['done'],r
  shot('08-scratchpad')
  r=chord(['meta_l','s'],'Scratchpad');assert r['done'],r
  # Super alone opens the launcher; the Super key never reaches a program.
  s=len(t());send(True,['meta_l']);send(False,['meta_l'])
  g['wait'](ACTION.format('Launcher'),s,30);g['wait'](r'\[shell.palette.open\]',s,30)
  shot('09-launcher')
  q.key('esc');time.sleep(1)
  assert 'KERNEL PANIC' not in t()
  print({'ok':True,**{k:str(v) for k,v in results.items()}})
 finally:g['stop']()
if __name__=='__main__':main()
