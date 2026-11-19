#!/usr/bin/env python3
"""Native offline CLAP processing/crash/timeout qualification through the restricted
helper coordinator (moan.role.* receipts); no hostile sandbox claim."""
import argparse,hashlib,json,os,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
from moan_gate_geometry import spindle
from program_gate_geometry import maximize,current_tile
from program_egui_gate import colours,client_rect
from moan_plugin_pixels import expected_status,model_view,visible_pixels
ROOT=Path(__file__).resolve().parents[1]
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',type=Path,required=True);p.add_argument('--source-disk',type=Path,required=True);p.add_argument('--out',type=Path,required=True);a=p.parse_args();out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise ValueError('fresh evidence directory required')
 programs=[('moanrack',ROOT/'target/x86_64-sex-app/release/moanrack'),('moanplug',ROOT/'target/x86_64-sex-app/release/moanplug')]
 inputs={str(p):sha(p)for p in [a.iso,a.source_disk,*[v for _,v in programs],*sorted((ROOT/'scripts').rglob('*.py'))]};(out/'inputs.json').write_text(json.dumps(inputs,indent=2)+'\n')
 (out/'programs').mkdir()
 for name,path in programs:shutil.copyfile(path,out/'programs'/name)
 (out/'source-commit.txt').write_text(__import__('subprocess').check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True))
 disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk);install(disk,[(name,path.read_bytes())for name,path in programs]);os.environ['SEXOS_GATE_BOUND']='700'
 g=boot(a.iso,disk,out,'moan-plugins','1280x800');q=g['q'];text=g['text'];result={'scope':'offline CLAP/VST3 helper crash containment; no hostile-plugin or ordinary OS binary compatibility claim'}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);s=spindle(g,'run moanrack @/data/moanrack');m=g['wait'](r'\[moanrack.ready\] target=(\d+) task=(\d+)',s,180);parent,task=map(int,m.groups());sid=int(g['wait'](rf'\[shell.program.launch\] target={parent} task={task} surface=(\d+)',s,60)[1]);tile,start=maximize(g,sid);w,h=tile[2]-16,tile[3]-42;g['wait'](rf'\[moanrack.control\] id=7 x=\d+ y=\d+ w={w} h={h}',start,120)
  def click(which):
   tile=current_tile(text(),sid);w,h=tile[2]-16,tile[3]-42;matches=re.findall(rf'\[moanrack.control\] id={which} x=(-?\d+) y=(-?\d+) w={w} h={h}',text());assert matches,(which,w,h);x,y=map(int,matches[-1]);assert 0<=x<w and 0<=y<h;(ox,oy)=(tile[0]+8,tile[1]+34);x+=ox;y+=oy;assert 0<=x<1280 and 0<=y<800;g['focus'](sid);q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/1279)}},{'type':'abs','data':{'axis':'y','value':round(y*32767/799)}}]});time.sleep(.1)
   for down in [True,False]:q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.1)
  def capture_status(name,expected,since):
   # Keep the cursor out of the label: only its real Weft pixels are compared.
   tile=current_tile(text(),sid);client=client_rect(tile,w,h)
   q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round((client[0]+w-8)*32767/1279)}},{'type':'abs','data':{'axis':'y','value':round((client[1]+h-8)*32767/799)}}]})
   g['wait'](rf'\[moanrack.paint\] target={parent} task={task} surface={sid} .* text_hex={expected.encode().hex()}\r?\n',since,60)
   g['wait'](rf'\[display.app.pixels\] target={parent} surface={sid} w={w} h={h}\r?\n',since,60)
   deadline=time.monotonic()+15;attempts=[]
   for n in range(64):
    path=out/f'{name}-capture-{n+1:03d}.ppm';q.cmd('screendump',{'filename':str(path)})
    try:
     view=model_view(text(),parent,task,sid,(w,h),expected,since);proof=visible_pixels(path.read_bytes(),client,view)
     if len(colours(path.read_bytes(),client))<=8:raise ValueError('whole client looks blank')
     attempts.append(dict(file=path.name,ok=True,proof=proof));shutil.copyfile(path,out/f'{name}.ppm');(out/f'{name}-captures.json').write_text(json.dumps(dict(ok=True,attempts=attempts),indent=2)+'\n');return
    except ValueError as error:attempts.append(dict(file=path.name,ok=False,error=str(error)))
    if time.monotonic()>=deadline:break
    time.sleep(.1)
   (out/f'{name}-captures.json').write_text(json.dumps(dict(ok=False,attempts=attempts),indent=2)+'\n');raise AssertionError('fresh visible status failed: '+expected)
  for mode in [0,1,0,2,0,4,5,6]:
   s=len(text());click(mode);began=g['wait'](rf'\[moanrack.started\] mode={mode} cookie=(\d+)',s,60);cookie=int(began[1]);child=g['wait'](rf'\[moan.role.started\] parent={parent} parent_task={task} cookie={cookie} pd=(\d+) task=(\d+)',s,120)
   expected=1 if mode in (0,4,6) else 0;result_marker=g['wait'](rf'\[moanrack.result\] mode={mode} cookie={cookie} complete={expected} processed={expected} retired=1',s,180);result_end=s+result_marker.end()
   if mode==4:g['wait'](r'\[moanplug.rendered\] frames=1025 blocks=5 ticks=\d+ format=vst3 offline=1',s,60)
   if mode==0:g['wait'](r'\[moanplug.rendered\] frames=1025 blocks=5 ticks=\d+ format=clap offline=1',s,60)
   elif mode==1:assert re.search(rf'fault.kill \[cpu.fault.terminal\] pd={child[1]} kind=invalid_opcode ',text()[s:]),'missing exact child invalid-opcode fault receipt'
   elif mode==2:g['wait'](rf'\[app.program.forced\] target={child[1]} task={child[2]} ',s,60)
   if mode==5:
    assert 'format=clap offline=1' not in text()[s:],'restart output must not publish'
    g['wait'](r'\[moanplug.rejected\] format=clap reason=restart',s,60)
    g['wait'](rf'\[app.private.retired\] target={child[1]} task={child[2]} ',s,60)
   if mode==6:
    segment=text()[s:];done=re.search(rf'\[app.private.retired\] target={child[1]} task={child[2]} ',segment);resume=re.search(r'\[moanrack.window.waited\] receipts_pending=1',segment);assert done and resume and done.start()<resume.start(),'missing causal window-wait mailbox preservation'
   capture_status(f'mode-{mode}-result-{cookie}',expected_status(mode),result_end)
   s=len(text());click(7);alive_marker=g['wait'](r'\[moanrack.alive\] n=(\d+)',s,60)
   capture_status(f'mode-{mode}-parent-{cookie}',f'Parent action {int(alive_marker[1])}',s+alive_marker.end())
  result.update(ok=True,processed_runs=5,crash_recovered=True,timeout_recovered=True,final_block_restart_rejected=True,window_wait_preserved_receipts=True,parent=parent,task=task)
  s=len(text());g['focus'](sid);q.key('f11');g['wait'](r'\[moanrack.closed\]',s,60);g['wait'](rf'\[app.private.retired\] target={parent} task={task} ',s,120);q.cmd('quit');g['proc'].wait(30)
  assert all(sha(Path(p))==value for p,value in inputs.items()),'immutable input changed'
  result['artifacts']={p.name:sha(p)for p in out.iterdir()if p.is_file()and p.suffix not in ['.img']};(out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
 finally:g['stop']()
if __name__=='__main__':main()
