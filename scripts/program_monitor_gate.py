#!/usr/bin/env python3
"""Mesh: `run mesh @/` lists the services and programs that use
the CPU with their share of one core, from the kernel's read-only statistics (syscall 87).
While `run bench spin 600` computes without a system call, Mesh must name Bench at the top
of its list at close to a whole core and keep CPU history for its graphs (R4). Home then
Delete in Mesh must end Bench through Shell before its spin finishes. After `edit todo.txt`
Mesh must list Jot's file grant, and its Grants tab (key 3) the domains' organs (R5)."""
import argparse,os,re,shutil,time
from pathlib import Path
os.environ.setdefault('SEXOS_GATE_BOUND','900')
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
SAMPLE=r'\[monitor\.sample\] rows=(\d+) busy_tenths=(\d+) top_pd=(\d+) top_tenths=(\d+) top_task=(\d+)[^\n]*\n'
GRAPH=r'\[monitor\.graph\] cores=(\d+) points=(\d+) cpu_permille=(\d+) core0_permille=(\d+) memory_permille=(\d+) calls_per_s=(\d+) disk_bytes_per_s=(\d+)\n'
LAUNCH=r'\[shell\.program\.launch\] target=(\d+) task=(\d+) surface=(\d+)'
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--min-tenths',type=int,default=700);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[(n,(ROOT/'iso_root/programs'/n).read_bytes())for n in('mesh','bench','jot')])
 g=boot(a.iso,disk,a.out,'monitor',a.resolution);t=g['text'];q=g['q']
 def typed(line):
  for k in line:
   if k=='@':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'2'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.05)
 def run(line):
  g['focus'](153);time.sleep(.5);typed(line);q.key('ret')
 def shot(name):
  path=a.out/f'{name}.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(.5)
  try:
   from PIL import Image;Image.open(path).save(a.out/f'{name}.png')
  except Exception:pass
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  s=len(t());run('run mesh @/')
  mesh=g['wait'](LAUNCH,s,180);mesh_pd,mesh_sid=int(mesh[1]),int(mesh[3])
  g['wait'](r'\[monitor\.ready\] target=(\d+) task=(\d+)',s,180)
  idle=g['wait'](SAMPLE,s,120);g['wait'](GRAPH,s,60)
  # R4: a busy program shows, with history.
  s=len(t());run('run bench spin 600')
  spin=g['wait'](r'\[bench\.spin\.start\] target=(\d+) task=(\d+)',s,180);bench,bench_task=int(spin[1]),int(spin[2])
  seen=[];deadline=time.time()+240
  while time.time()<deadline:
   for m in re.finditer(SAMPLE,t()[s:]):
    if int(m[3])==bench:seen.append(int(m[4]))
   if (seen and max(seen)>=a.min_tenths) or re.search(r'\[bench\.spin\.done\]',t()[s:]):break
   time.sleep(2)
  shot('monitor')
  assert seen,f'Mesh never listed Bench (domain {bench}) at the top'
  assert max(seen)>=a.min_tenths,f'Bench peaked at {max(seen)/10:.1f}% of a core, want {a.min_tenths/10:.0f}%'
  graphs=[m for m in re.finditer(GRAPH,t()[s:])]
  assert graphs and int(graphs[-1][2])>=2,'Mesh kept no CPU history'
  cores=int(graphs[-1][1]);assert max(int(m[3]) for m in graphs)>=min(a.min_tenths//max(cores,1),500),'the CPU graph never showed Bench'
  # R4: end it from Mesh: Home chooses the busiest line, Delete asks Shell to end it.
  g['focus'](mesh_sid);time.sleep(.5)
  s=len(t());q.key('home');g['wait'](rf'\[monitor\.select\] pd={bench} task={bench_task} program=1',s,60)
  q.key('delete');g['wait'](rf'\[monitor\.end\] target={bench} status=0\n',s,60)
  g['wait'](rf'\[shell\.program\.end\] caller={mesh_pd} target={bench} status=0\n',s,60)
  g['wait'](rf'\[app\.private\.retired\] target={bench} task={bench_task} ',s,120)
  assert not re.search(rf'\[bench\.spin\.done\] target={bench} task={bench_task}',t()),'Bench finished on its own instead of being ended'
  # R5: Jot's file grant shows in Mesh's capability graph.
  s=len(t());run('write todo.txt buy milk');g['wait'](r'\[spindle\.fs\.result\] kind=8 status=0',s,60)
  s=len(t());run('edit todo.txt');jot=g['wait'](LAUNCH,s,120);g['wait'](r'\[jot\.ready\]',s,60)
  grant=g['wait'](rf'\[monitor\.grant\] target={jot[1]} task={jot[2]} live=1 grants=1 read_bytes=\d+ text=(@/\S*todo\.txt)\n',s,60)
  g['focus'](mesh_sid);time.sleep(.5)
  s=len(t());q.key('3');g['wait'](r'\[monitor\.view\] tab=grants via=key',s,60)
  organs=g['wait'](r'\[monitor\.organ\] pd=(\d+) key=(\d+) caps=(\d+) links=(\d+)',s,30)
  time.sleep(1.5);shot('monitor-grants')
  s=len(t());q.key('2');g['wait'](r'\[monitor\.view\] tab=performance via=key',s,60);time.sleep(1.5);shot('monitor-performance')
  text=t();assert 'KERNEL PANIC' not in text and '[monitor.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'bench_pd':bench,'bench_peak_percent':max(seen)/10,'samples_on_top':len(seen),'idle_busy_percent':int(idle[2])/10,
         'cores':cores,'graph_points':int(graphs[-1][2]),'ended':True,'jot_grant':grant[1],'first_organ':organs[0],'cores_env':os.environ.get('SEXOS_GATE_CORES','1')})
 finally:g['stop']()
if __name__=='__main__':main()
