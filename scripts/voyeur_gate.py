#!/usr/bin/env python3
"""Voyeur (system search) on a booted desktop: Super opens the launcher;
`2+2` answers 4 first (Abacus), `bell` is answered by more than one provider,
and a setting name puts its settings page first. Reads [voyeur.query]
serial records; screens are saved for review."""
import argparse,re,shutil,time
from pathlib import Path
from quil_autosave_gate import boot
QUERY=r'\[voyeur.query\] ticket=\d+ query={} providers=(\d+) answered=(\S+) results=(\d+) first=([^\n]+?) first_kind=(\S+) first_provider=(\S+)'
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 g=boot(a.iso,disk,a.out,'voyeur',a.resolution);t=g['text'];q=g['q']
 def send(down,keys):q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}for k in keys]});time.sleep(.08)
 def tap(k,shift=False):
  if shift:send(True,['shift'])
  send(True,[k]);send(False,[k])
  if shift:send(False,['shift'])
 def query(keys,text):
  s=len(t())
  for k in keys:tap(*k) if isinstance(k,tuple) else tap(k)
  m=g['wait'](QUERY.format(re.escape(text)),s,30)
  return {'providers':int(m[1]),'answered':m[2],'results':int(m[3]),'first':m[4].strip(),'kind':m[5],'provider':m[6]}
 def clear(n):
  for _ in range(n):tap('backspace')
 def shot(name):time.sleep(1.5);q.cmd('screendump',{'filename':str(a.out/f'{name}.ppm')});time.sleep(.5)
 results={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(3)
  s=len(t());tap('meta_l');g['wait'](r'\[shell.palette.open\]',s,30);time.sleep(1)
  r=query(['2',('equal',True),'2'],'2+2');assert (r['first'],r['kind'],r['provider'])==('4','calculator','abacus'),r;results['2+2']=r
  shot('01-two-plus-two');clear(3)
  r=query(['b','e','l','l'],'bell');assert r['providers']>=2 and r['first']=='Open Bell',r;results['bell']=r
  shot('02-bell-merged');clear(4)
  r=query(list('wallpaper'),'wallpaper');assert (r['first'],r['kind'])==('Wallpaper','setting'),r;results['wallpaper']=r
  s=len(t());tap('ret');g['wait'](r'\[voyeur.activate\] provider=settings kind=setting',s,30)
  assert 'KERNEL PANIC' not in t()
  print({'ok':True,**{k:str(v) for k,v in results.items()}})
 finally:g['stop']()
if __name__=='__main__':main()
