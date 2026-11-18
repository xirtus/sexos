#!/usr/bin/env python3
"""Many programs at once on a large screen: rows past the first few share one key and
each runs in its own protection view (kernel/src/views.rs). Six run side by side at
1920x1080, each still drawn, and all close cleanly. With several cores (SEXOS_GATE_CORES)
shared rows run on more than one core, and with PCIDs most view switches keep the TLB."""
import argparse,os,re,shutil,time
from pathlib import Path
from quil_autosave_gate import boot
from quil_workflow_gate import palette_select,palette_commands
from check_desktop_pixels import font_from_source
from program_launch_gate import text_ink,TEXT
from check_disk_persistence import ppm_rgb
from collections import Counter
ROOT=Path(__file__).resolve().parents[1]
def placed_ink(path,tile,font):
  # The text's place in the window follows the window chrome; the best of a few nearby
  # origins is taken, and only pixels of exactly the text colour count.
  x,y,w,h=tile;best=None
  for dy in range(-16,17,2):
   for dx in range(-8,9,2):
    ink,missing=text_ink(path,(x+dx,y+dy,w,h),font)
    if best is None or missing*best[0]<best[1]*ink:best=(ink,missing,dx,dy)
  return best
def colours(path,tile):
  width,_,px=ppm_rgb(path.read_bytes(),min_width=1,min_height=1);x,y,w,h=tile
  c=Counter(px[(j*width+i)*3:(j*width+i)*3+3].hex()for j in range(y,y+h,2)for i in range(x,x+w,2))
  return c.most_common(5)
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--resolution',default='1920x1080');p.add_argument('--live',type=int,default=6);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 font=font_from_source((ROOT/'servers/sexdisplay/src/main.rs').read_text())
 g=boot(a.iso,disk,a.out,'program',a.resolution);t=g['text'];q=g['q'];commands=palette_commands(ROOT)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  views=re.search(r'\[boot\.views\] views=(\d+) pcid=(\d)',t());assert views,'no protection views'
  pool=re.findall(r'\[boot\.program\.pool\.regions\] target=(\d+) pages=(\d+) spans=(\d+) view=\d+ boot_spans=(\d+)',t())
  assert pool,'no shared rows claimed their regions'
  for target,pages,spans,boot_spans in pool:
   # Whole spans: a region needs no more than its size in 2 MiB spans, plus one per region,
   # and BOOT's image and stack (claimed with the successor, logged apart) add their own.
   assert int(spans)<=int(pages)//512+8+int(boot_spans),f'row {target}: {spans} spans for {pages} pages'
  live=[]
  for _ in range(a.live):
   palette_select(g['key'],'FocusProgram',commands)
   s=len(t());q.key('ret');m=g['wait'](rf'\[hello.ready\] target=(\d+) task=(\d+) surface=(\d+) bytes={len(TEXT)}',s,60)
   live.append((m[1],int(m[2]),m[3]));g['wait'](rf'\[display.text.committed\] target={m[1]} ',s,60);time.sleep(2)
  assert len({row[0]for row in live})==a.live,'programs were not on separate rows'
  shared={row[0] for row in pool}
  assert len(shared&{row[0]for row in live})>=2,f'fewer than two shared rows ran: {live} {shared}'
  # Six windows retile as each opens; on several emulated cores Display may still be
  # redrawing one, so the picture is taken again (up to ~30 s) until every row is drawn.
  for attempt in range(10):
   time.sleep(3)
   path=a.out/f'many-{attempt}.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(1)
   drawn=[]
   for target,task,sid in live:
    tile=re.findall(rf'\[shell.tile.apply\] sid={sid} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t());x,y,w,h=map(int,tile[-1])
    ink,missing,dx,dy=placed_ink(path,(x,y,w,h),font)
    drawn.append(dict(target=int(target),task=task,tile=(x,y,w,h),ink=ink,missing=missing,offset=(dx,dy)))
   # The pointer rests at this screen's centre and may cover a few glyphs.
   if all(d['ink']>100 and d['missing']*10<=d['ink'] for d in drawn):break
   print('[pool.redraw.wait]',attempt,drawn,flush=True)
   print('[pool.redraw.colours]',colours(path,drawn[0]['tile']),flush=True)
  print('[pool.drawn]',drawn,flush=True)
  for d in drawn:assert d['ink']>100 and d['missing']*10<=d['ink'],f"row {d['target']}: text ink {d['ink']} missing {d['missing']} tile {d['tile']}"
  # Rows no longer take turns: no span opens or closes for a switch, and no full flush.
  assert '[program.pool.switch]'not in t(),'shared rows still take turns'
  switches=[tuple(map(int,m))for m in re.findall(r'\[views\.switch\] switches=(\d+) flushed=(\d+) view=(\d+) core=(\d+) pcid=(\d)',t())]
  assert switches,'no view switches'
  n,flushed,_,_,pcid=switches[-1]
  if pcid:assert flushed*2<=n,f'most view switches flushed: {flushed} of {n}'
  cores={int(core) for view,core in re.findall(r'\[views\.first\] view=(\d+) core=(\d+)',t()) if view!='0'}
  if int(os.environ.get('SEXOS_GATE_CORES','1'))>=4:assert len(cores)>=2,f'shared rows ran on one core only: {cores}'
  for target,task,sid in reversed(live):
   g['focus'](int(sid));s=len(t());q.key('f11');g['wait'](rf'\[hello.closed\] target={target} task={task}',s,60)
   g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,60);time.sleep(1)
  assert 'KERNEL PANIC'not in t()
  print({'ok':True,'drawn':drawn,'switches':switches[-1],'row_cores':sorted(cores),'pcid':int(views[2]),'pool':pool})
 finally:g['stop']()
if __name__=='__main__':main()
