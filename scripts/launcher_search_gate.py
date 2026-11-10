#!/usr/bin/env python3
"""Type-to-search in the launcher on a booted desktop: Super opens it, typed
letters filter and rank the rows, Backspace and Esc edit the search, Enter
opens the best match. Screens are saved for review."""
import argparse,re,shutil,time
from pathlib import Path
from quil_autosave_gate import boot
SEARCH=r'\[shell.launcher.search\] query={} matches=(\d+) top=([^\n]+?) changed=(\d)'
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 g=boot(a.iso,disk,a.out,'search',a.resolution);t=g['text'];q=g['q']
 def send(down,keys):q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}for k in keys]});time.sleep(.08)
 def tap(k):send(True,[k]);send(False,[k])
 def typed(keys,query):
  s=len(t())
  for k in keys:tap(k)
  m=g['wait'](SEARCH.format(re.escape(query)),s,30)
  return int(m[1]),m[2].strip()
 def shot(name):time.sleep(1.5);q.cmd('screendump',{'filename':str(a.out/f'{name}.ppm')});time.sleep(.5)
 results={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(3)
  s=len(t());tap('meta_l');g['wait'](r'\[shell.palette.open\]',s,30);time.sleep(1)
  n,top=typed(['w','e','b'],'web');assert top=='Open Browser',(n,top);results['web']=(n,top)
  shot('01-search-web')
  for _ in range(3):tap('backspace')
  n,top=typed(['t','e','r','m'],'term');assert top=='Open Spindle',(n,top);results['term']=(n,top)
  n,top=typed(['q','q'],'termqq');assert n==0,(n,top)
  shot('02-no-match')
  # Esc clears the search; the launcher stays open with every row.
  s=len(t());tap('esc');g['wait'](r'\[shell.launcher.search\] query= matches=(\d+)',s,30)
  assert '[launcher.close]' not in t()[s:],'esc closed instead of clearing'
  # J/K still choose rows before a search begins.
  s=len(t());tap('j');g['wait'](r'\[shell.palette.select\] old=0 new=1',s,30)
  n,top=typed(['q','u','i'],'qui');assert top=='Open Quil',(n,top)
  s=len(t());tap('ret');g['wait'](r'\[command_palette.execute\] cmd=1 name=Open Quil',s,30);results['enter']='Open Quil'
  assert 'KERNEL PANIC' not in t()
  print({'ok':True,**{k:str(v) for k,v in results.items()}})
 finally:g['stop']()
if __name__=='__main__':main()
