#!/usr/bin/env python3
"""Thousands of background programs stay alive at once, stay isolated, and all end when told, with the desktop still answering."""
import argparse,os,re,shutil,statistics,time
from pathlib import Path
os.environ['SEXOS_GATE_MEMORY']='4G';os.environ['SEXOS_GATE_BOUND']='1500'
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
FIELDS=r'live=(\d+) started=(\d+) ended=(\d+) capacity=(\d+) pool_free=(\d+) pool_total=(\d+) switches=(\d+) switch_cycles=(\d+) kstack_peak=(\d+)'
NAMES=('live','started','ended','capacity','pool_free','pool_total','switches','switch_cycles','kstack_peak')
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--count',type=int,default=4000);p.add_argument('--hold',type=int,default=60);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[(n,(ROOT/'iso_root/programs'/n).read_bytes()) for n in ('hum','snoop','spin','doze')])
 g=boot(a.iso,disk,a.out,'records',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  for k in line:
   q.key('spc' if k==' ' else k);time.sleep(.05)
  q.key('ret')
 def census():
  s=len(t());type_line('records');m=g['wait'](r'\[records.census\] '+FIELDS,s,120)
  return dict(zip(NAMES,map(int,m.groups())))
 def wait_live(want,timeout):
  d=time.monotonic()+timeout
  while True:
   c=census()
   if c['live']==want:return c
   if time.monotonic()>d:raise TimeoutError(f'live {c["live"]}, wanted {want}')
   time.sleep(5)
 def typing():
  samples=[]
  for k in 'abcdefgh':
   s=len(t());b=time.monotonic();q.key(k);g['wait'](r'\[spindle.display.batch.admitted\]',s,120);samples.append(time.monotonic()-b);time.sleep(.3)
  for _ in samples:q.key('backspace');time.sleep(.05)
  return round(statistics.median(samples),3),round(max(samples),3)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,180);time.sleep(3)
  g['focus'](153);time.sleep(.5)
  capacity=int(g['wait'](r'\[boot.records\] capacity=(\d+)',0,10)[1]);assert capacity>=a.count,f'capacity {capacity}'
  before=typing();empty=census();assert empty['live']==0
  s=len(t());b=time.monotonic();type_line(f'spawn hum {a.count}');g['wait'](rf'\[shell.records.spawned\] live={a.count}\b',s,600);start_s=round(time.monotonic()-b,1)
  loaded=census();assert loaded['live']==a.count
  during=typing();time.sleep(a.hold)
  held=census();assert held['live']==a.count,f'lost programs while holding: {held}'
  assert '[hum.corrupt]'not in t()
  # A hostile program stops at the hardware, alone.
  s=len(t());type_line('spawn snoop 1');g['wait'](r'\[snoop.reading\]',s,120);g['wait'](r'fault.kill[^\n]*pd=(\d+)',s,120);time.sleep(5)
  assert '[snoop.survived]'not in t(),'a program read another program'
  after_snoop=wait_live(a.count,120)
  # A program that ignores requests can't hold the machine, and can't refuse.
  s=len(t());type_line('spawn spin 1');g['wait'](r'\[shell.records.spawned\] live=',s,120);spinning=typing()
  s=len(t());b=time.monotonic();type_line('stopall');stopped=wait_live(1,600);stop_s=round(time.monotonic()-b,1)
  type_line('stopall force');ended=wait_live(0,120)
  assert ended['pool_free']==ended['pool_total'],f'memory not all back: {ended}'
  # Nothing leaks across a second round.
  s=len(t());type_line('spawn hum 100');g['wait'](r'\[shell.records.spawned\] live=100\b',s,120);type_line('stopall force');again=wait_live(0,120)
  assert again['pool_free']==again['pool_total'],f'memory not all back: {again}'
  # Programs with nothing to do cost an idle machine nothing.
  s=len(t());type_line(f'spawn doze {a.count}');g['wait'](rf'\[shell.records.spawned\] live={a.count}\b',s,600);time.sleep(5)
  dozing=census();time.sleep(a.hold);dozed=census();assert dozed['live']==a.count
  idle_switches=dozed['switches']-dozing['switches'];idle_typing=typing()
  type_line('stopall');wait_live(0,300);back=census();assert back['pool_free']==back['pool_total'],f'memory not all back: {back}'
  assert '[hum.corrupt]'not in t() and 'KERNEL PANIC'not in t()
  frames=empty['pool_total']-loaded['pool_free']
  print({'ok':True,'live':a.count,'start_seconds':start_s,'stop_seconds':stop_s,'bytes_per_program':frames*4096//a.count,
   'typing_median_max':{'idle':before,'loaded':during,'spinner':spinning},'switch_cycles_avg':held['switch_cycles']//max(held['switches'],1),
   'switches':held['switches'],'hold_seconds':a.hold,'idle_switches_during_hold':idle_switches,'idle_typing':idle_typing,'kstack_peak':ended['kstack_peak'],'ended':ended['ended']})
 finally:g['stop']()
if __name__=='__main__':main()
