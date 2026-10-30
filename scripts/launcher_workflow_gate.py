#!/usr/bin/env python3
"""Finite source-bound launcher/window qualification. Never builds an image.

One boot covers original Spindle12 and BOOT-reserved successor17. Shell route
and lifecycle catalogs remain authority; labels and numeric IDs are only joins.
"""
import argparse,ast,hashlib,json,os,re,shutil,socket,subprocess,threading,time
from pathlib import Path
import clipboard_workflow_gate as input_gate
import tls_responsiveness_gate as desktop
from desktop_close_gate import QMP
from network_contention_gate import frozen_binding,cleanup_receipts,MAX_PCAP,syn_records
from check_launcher_pixels import verify as launcher_pixels,layout as launcher_layout
from check_spindle_viewport_pixels import verify as spindle_pixels
from check_desktop_pixels import font_from_source,glyph
from check_disk_persistence import ppm_rgb
from check_tcp_capture import inspect as tcp_capture
from check_nonunix_target import inspect as target_check
from check_quil_ui_pixels import verify as quil_pixels,view as quil_view
from tls_exchange_gate import ControlledPeer,ECHO,command_profile

SPINDLES=(12,17)
TARGETS={9:dict(frame=201,surface=156,panes=(156,),marker='qkeep'),12:dict(frame=153,surface=154,panes=(154,160,161,162),marker='keep12'),17:dict(frame=306,surface=302,panes=(302,303,304,305),marker='keep17')}
SCOPE=('One sealed boot/profile: original Quil9 Focus/Restore/BringHere and Spindle12/17 Open/Focus/Restore/BringHere; unchanged held identities and typed text, full Spindle cleanup, then one BOOT-reserved successor. '
       'Whole launcher and four-pane glyph pixels use immutable serial prefixes. Post-action pictures require source-bound launcher text-ink erasure, with the same-color Linen text band excluded and nontrivial remaining coverage. Cleanboot checks policy receipts, absence of Spindle ink in the source-defined legacy pane footprint and no Spindle canvas over its intersection with the actual Browser interior. '
       'No whole-desktop app-absence, public network, general app restart, runtime mint, process VAS, POSIX, physical hardware, SMP or hostile-containment claim.')
ACTION=r'\[launcher.action\] target=(\d+) task=(\d+) frame=(\d+) action=(Open|Focus|Restore|BringHere) from=(\d+) to=(\d+) visible=([01]) focused=([01]) activated=([01]) queued=([01]) accepted=([01]) ok=([01]) pixels=unconfirmed'

def sha(data):return hashlib.sha256(data).hexdigest()
def rows(trace,tag):return[(m.start(),dict(re.findall(r'(\w+)=([^\s\]]+)',m[1])))for m in re.finditer(r'\['+re.escape(tag)+r'\]([^\r\n]*)',trace)]
def one(trace,tag,**wanted):
 found=[(p,r)for p,r in rows(trace,tag)if all(r.get(k)==str(v)for k,v in wanted.items())]
 if len(found)!=1:raise ValueError(f'{tag}: missing/duplicate exact receipt {wanted}')
 return found[0]
def source_contract(root):
 root=Path(root)
 paths=('servers/silk-shell/src/main.rs','servers/silk-shell/src/launcher.rs','servers/sexdisplay/src/main.rs','servers/sexdisplay/src/surface_groups.rs',
        'servers/sexdisplay/src/surface_lifecycle.rs','apps/spindle/src/main.rs','apps/spindle/src/viewport.rs','apps/spindle/src/display_delivery.rs',
        'crates/sex-pdx/src/spindle_view.rs','crates/silkbar-model/src/lib.rs','crates/sex-pdx/src/app_lifecycle.rs',
        'scripts/launcher_workflow_gate.py','scripts/tests/test_launcher_workflow_gate.py','scripts/check_launcher_pixels.py',
        'scripts/check_spindle_viewport_pixels.py','scripts/clipboard_workflow_gate.py','scripts/tls_responsiveness_gate.py',
        'scripts/network_contention_gate.py','scripts/check_tcp_capture.py','scripts/check_disk_persistence.py','scripts/check_desktop_pixels.py','crates/sex-pdx/src/desktop_style.rs','servers/quil/src/main.rs','servers/quil/src/view.rs','scripts/check_quil_ui_pixels.py','servers/silk-shell/src/desktop_content.rs','apps/kaleidoscope/src/main.rs')
 sources={name:(root/name).read_text()for name in paths};shell=sources[paths[0]];policy=sources[paths[1]];display=sources[paths[2]];groups=sources[paths[3]];app=sources[paths[5]];bar=sources[paths[9]]
 if not all(label in policy for label in('Open Spindle','Focus Spindle','Restore Spindle','Bring Here Spindle'))or 'launcher_focus_existing('not in shell or '[launcher.action]'not in shell:raise ValueError('truthful exact live launcher action wiring absent')
 if 'g.frame_seen || g.parent_required()'not in groups or '[display.child.parent_gate]'not in display or 'awaiting_parent(surface)'not in display:raise ValueError('exact pre-parent display policy binding absent')
 def constant(source,name):
  m=re.search(r'\bconst '+name+r':[^=]+=\s*(0x[\da-fA-F_]+|\d+)\s*;',source)
  if not m:raise ValueError('missing source constant '+name)
  return int(m[1].replace('_',''),0)
 commands=re.search(r'const COMMAND_LIST:\s*\[CommandDef;\s*(\d+)\]\s*=\s*\[(.*?)\n\];',shell,re.S)
 if not commands:raise ValueError('missing actual launcher command catalog')
 command_rows=re.findall(r'CommandDef\s*\{\s*command:\s*Command::(\w+),\s*name:\s*"([^"]+)"\s*\}',commands[2])
 if len(command_rows)!=int(commands[1])or not command_rows or command_rows[0][0]!='FocusSpindle':raise ValueError('launcher catalog length/selected Spindle entry differs')
 app_count=re.search(r'let app_count:\s*u8\s*=\s*(\d+)\s*;',shell)
 if not app_count or not 0<int(app_count[1])<=len(command_rows):raise ValueError('missing actual launcher app subset count')
 grid=[constant(app,k)for k in('GRID_X0','GRID_Y0','SURF_W','SURF_H','GRID_SURF_X','GRID_SURF_Y')]
 fg=constant(app,'CONTENT_TEXT_COLOR')&0xffffff
 chrome_match=re.search(r'\bconst SILK_CHROME_TEMPLATE_DEFAULT[^=]*=\s*\w+\s*\{(.*?)\n\};',shell,re.S)
 if not chrome_match:raise ValueError('missing actual Shell ChromeTemplate')
 chrome={}
 for name in ('top_bar_light_gap_px','top_bar_light_size_px','top_bar_height_px'):
  item=re.search(r'\b'+name+r':\s*(\d+)',chrome_match[1])
  if not item or int(item[1])<=0:raise ValueError('invalid source-bound light hitbox')
  chrome[name]=int(item[1])
 style=sources['crates/sex-pdx/src/desktop_style.rs'];quil=sources['servers/quil/src/main.rs'];initial=re.search(r'const QUIL_TEXT_INIT:.*?=\s*(b"[^\n]*");',quil)
 if not initial or 'native_canvas(owner,surface_id)'not in display or '0x00303860u32'not in display:raise ValueError('native canvas/Browser source binding differs')
 palette_ink=re.search(r'PALETTE_TEXT.queue\(&v.bytes\[\.\.v.len\],(0x[\da-fA-F]+),',shell)
 header=re.search(r'v.line\(b"","([^"]+)",cols\)',policy)
 linen=sources['servers/silk-shell/src/desktop_content.rs']
 if not palette_ink or not header or 'row_y:36 + 2*line_h'not in linen or 'out.line(b"",cols)'not in linen or 'let line_h = 9 * scale;'not in linen:raise ValueError('launcher/Linen header erasure source model differs')
 ink=int(palette_ink[1],0)&0xffffff
 if ink in (fg,constant(style,'TEXT')&0xffffff,0xcdd6f4)or '0x00CDD6F4'not in sources['apps/kaleidoscope/src/main.rs']:raise ValueError('controlled app text color conflicts with launcher erasure')
 return dict(launcher_header=header[1],launcher_foreground=ink,launcher_count=len(command_rows),launcher_app_count=int(app_count[1]),exhausted_title=command_rows[0][1],chrome=chrome,canvas=constant(style,'NATIVE_CANVAS'),quil_foreground=constant(style,'TEXT'),quil_initial=ast.literal_eval(initial[1]),font=font_from_source(display),grid=[grid[0],grid[1],grid[2]*grid[4],grid[3]*grid[5]],foreground=fg,
             workspace_x=[constant(bar,'WS_X0'),constant(bar,'WS_X1')],workspace_y=constant(bar,'WS_Y'),
             source_sha256={name:sha(value.encode())for name,value in sources.items()})

def parse_actions(trace):
 matches=list(re.finditer(ACTION,trace))
 if len(matches)!=trace.count('[launcher.action]'):raise ValueError('malformed/foreign launcher action receipt')
 result=[]
 for m in matches:
  target,task,frame=map(int,m.groups()[:3]);action=m[4];origin,to,visible,focused,activated,queued,accepted,ok=map(int,m.groups()[4:])
  if target not in TARGETS or frame!=TARGETS[target]['frame']or task<=0 or not accepted or queued==ok:raise ValueError('wrong live action subject/frame or rejected effect')
  if focused and not visible:raise ValueError('focused receipt lacks actual admitted visibility')
  if action!='Open'and activated:raise ValueError('existing window action activated a new incarnation')
  result.append(dict(offset=m.start(),end=m.end(),target=target,task=task,frame=frame,action=action,origin=origin,to=to,visible=visible,focused=focused,activated=activated,queued=queued,accepted=accepted,ok=ok))
 return result

def completed_actions(trace,start=0):
 result=[];pending=None
 for row in parse_actions(trace):
  if row['offset']<start:continue
  if row['target']==17 and row['action']=='Open':
   if pending is not None or (row['activated'],row['queued'],row['ok'])!=(1,1,0):raise ValueError('successor activation must remain one queued accepted intent')
   result.append(row);continue
  if row['queued']:
   if pending is not None:raise ValueError('duplicate/interleaved queued action')
   pending=row;continue
  if pending is not None:
   if any(row[key]!=pending[key]for key in ('target','task','frame','action','origin','to','activated')):raise ValueError('queued action completed a foreign subject/effect')
   pending=None
  if (row['visible'],row['focused'],row['accepted'],row['ok'])!=(1,1,1,1):raise ValueError('completed live action lacks actual admitted focus/visibility')
  result.append(row)
 if pending is not None:raise ValueError('queued action never completed')
 return result

def lifecycle_joins(trace,start=0):
 actions=completed_actions(trace,start)
 expected=[(9,'Focus',0,0,0),(9,'Restore',0,0,0),(9,'BringHere',0,1,0),(12,'Open',0,0,0),(12,'Focus',0,0,0),(12,'Restore',0,0,0),(12,'BringHere',0,1,0),(17,'Open',1,1,1),(17,'Focus',1,1,0),(17,'Restore',1,1,0),(17,'BringHere',1,0,0)]
 if [(r['target'],r['action'],r['origin'],r['to'],r['activated'])for r in actions]!=expected:raise ValueError('missing/replayed/out-of-order finite launcher action sequence')
 catalogs={};cleanups=[]
 for target in TARGETS:
  p,c=one(trace,'boot.app.resources',target=target,surface=TARGETS[target]['surface'],required=31,coordinator=3)
  task=int(c.get('task','0'));pages=int(c.get('private_pages','0'))
  if task<=0 or pages<=0 or any(r['task']!=task for r in actions if r['target']==target):raise ValueError('action task differs from held BOOT catalog')
  catalogs[target]=dict(task=task,pages=pages)
  relevant=[r for r in actions if r['target']==target]
  focus,restore,bring=(relevant[0],relevant[1],relevant[2])if target==9 else(relevant[1],relevant[2],relevant[3])
  minimize=[(p,r)for p,r in rows(trace,'silk.lifecycle.minimize.hidden')if r.get('sid')==str(TARGETS[target]['frame'])]
  if len(minimize)!=1:raise ValueError('missing/duplicate minimized exact existing frame')
  frame_id=minimize[0][1].get('frame')
  restores=[(p,r)for p,r in rows(trace,'silk.lifecycle.restore.ok')if r.get('sid')==str(TARGETS[target]['frame'])]
  if len(restores)!=1 or restores[0][1].get('frame')!=frame_id or not focus['offset']<minimize[0][0]<restores[0][0]<restore['offset']:raise ValueError('Restore lacks ordered same-frame minimize/restore')
  moves=[(p,r)for p,r in rows(trace,'silk.frame.scene.move.done')if r.get('frame')==frame_id]
  if len(moves)!=1 or not restore['offset']<moves[0][0]<bring['offset']or moves[0][1].get('from')!=str(bring['origin'])or moves[0][1].get('to')!=str(bring['to'])or moves[0][1].get('ownership_unique')!='1'or moves[0][1].get('ok')!='1':raise ValueError('BringHere lacks one exact existing-frame reparent')
  if target==9:continue
  close_pos,_=one(trace,'app.close.accepted',target=target,task=task,coordinator=3)
  cleanup=cleanup_receipts(trace,target,task,close_pos)
  retired_pos,retired=one(trace,'app.private.retired',target=target,task=task,acked=31,required=31,all_flushed=1,pool_reusable=1)
  pool_pos,pool=one(trace,'sext.app.pool.ready',target=target,task=task,reused=1,zero_owner='sext',ok=1)
  if not relevant[-1]['offset']<close_pos<retired_pos<pool_pos or int(retired.get('pages','0'))!=pages or int(retired.get('bytes','0'))!=pages*4096 or pool.get('bytes')!=pool.get('zeroed')or int(pool.get('bytes','0'))!=pages*4096:raise ValueError('exact ordered private page retirement/scrub differs')
  cleanups.append(dict(cleanup,pages=pages,retired_offset=retired_pos,pool_offset=pool_pos))
  if any(p>retired_pos for p,r in rows(trace,'spindle.view.accepted')if r.get('target')==str(target)):raise ValueError('retired predecessor rendered again')
 reserve_pos,reserve=one(trace,'boot.app.successor',logical=12,incarnation=2,predecessor=12,successor=17,task=catalogs[17]['task'],surface=302,key=12,dormant=1,runtime_mint=0)
 commit_pos,commit=one(trace,'app.activation.committed',logical=12,incarnation=2,predecessor=12,successor=17,task=catalogs[17]['task'],surface=302,runtime_alloc=0,runtime_mint=0,bsp=1)
 if len([(p,r)for p,r in rows(trace,'app.activation.committed')if r.get('logical')=='12'])!=1 or not reserve_pos<cleanups[0]['pool_offset']<commit_pos<actions[7]['offset']:raise ValueError('successor activation did not follow exact predecessor cleanup')
 if int(reserve.get('leaves','0'))!=catalogs[17]['pages']or int(commit.get('pages','0'))!=catalogs[17]['pages']:raise ValueError('BOOT successor page restoration differs')
 start_pos,_=one(trace,'spindle.incarnation.start',target=17,task=catalogs[17]['task'],surface=302,fresh=1)
 present_pos,_=one(trace,'app.surface.presented',target=17,surface=302,sequence=1,ok=1)
 if not commit_pos<start_pos<present_pos<actions[8]['offset']:raise ValueError('successor lacks fresh start/presentation before subsequent Focus')
 denied,_=one(trace,'launcher.execute.denied',idx=0,reason='boot_reserve_exhausted')
 if denied<cleanups[1]['pool_offset']:raise ValueError('reserve exhaustion precedes final cleanup')
 if re.search(r'\[spindle.net.intent\]|\[browser\.(?:http|https)\.intent\]',trace):raise ValueError('unexpected scripted network work')
 return dict(actions=actions,catalogs=catalogs,cleanup=cleanups,activation_offset=commit_pos)

def no_ghost(trace,image,contract,resolution):
 width,height,pixels=ppm_rgb(image,min_width=1,min_height=1)
 if (width,height)!=tuple(resolution):raise ValueError('wrong actual framebuffer resolution')
 x,y,w,h=contract['grid'];left,top=max(0,x),max(50,y);right,bottom=min(width,x+w),min(height,y+h)
 if right<=left or bottom<=top:raise ValueError('no clipped pre-parent footprint to inspect')
 color=contract['foreground'].to_bytes(3,'big');ink=sum(pixels[(py*width+px)*3:(py*width+px)*3+3]==color for py in range(top,bottom)for px in range(left,right))
 if ink:raise ValueError('Spindle pre-parent glyph/sliver ink remains')
 gates=[r for _,r in rows(trace,'display.child.parent_gate')if r.get('target')=='12']
 if len(gates)!=4 or {int(r.get('surface','0'))for r in gates}!=set(TARGETS[12]['panes'])or any(any(r.get(k)!=v for k,v in dict(parent='153',observed='0',standalone='0',retained='1').items())for r in gates):raise ValueError('missing/duplicate/foreign exact pre-parent gate receipts')
 if any(row['target']in SPINDLES for row in parse_actions(trace))or re.search(r'\[spindle.placeholder.open\]|\[shell.app.viewport\] target=12 ',trace):raise ValueError('cleanboot already opened a Shell Spindle frame')
 browser=[r for _,r in rows(trace,'shell.tile.apply')if r.get('sid')=='300'and all(k in r for k in ('x','y','w','h'))]
 if not browser:raise ValueError('no actual Browser tile for opaque ghost check')
 bx,by,bw,bh=(int(browser[-1][k],0)for k in ('x','y','w','h'))
 a,b,c,d=max(left,bx+8),max(top,by+36),min(right,bx+bw-8),min(bottom,by+bh-8)
 if not 0<=bx<bx+bw<=width or not 50<=by<by+bh<=height or c<=a or d<=b:raise ValueError('no valid Browser body/legacy-grid intersection')
 canvas=contract['canvas'].to_bytes(3,'big');opaque=sum(pixels[(py*width+px)*3:(py*width+px)*3+3]==canvas for py in range(b,d)for px in range(a,c))
 if opaque:raise ValueError('opaque Spindle canvas remains over Browser body')
 return dict(checked_pixels=(right-left)*(bottom-top),forbidden_ink=ink,canvas_checked_pixels=(c-a)*(d-b),forbidden_canvas=opaque,clipped_legacy_footprint=[left,top,right-left,bottom-top],browser_body_intersection=[a,b,c-a,d-b])

def spindle_geometry(trace,target):
 m=list(re.finditer(rf'\[spindle.view.accepted\] target={target} task=(\d+) sequence=(\d+) w=(\d+) h=(\d+) cols=(\d+) top_rows=(\d+) bottom_rows=(\d+) sender=3',trace))
 if not m:raise ValueError('no exact Spindle viewport')
 current=m[-1];task,sequence,w,h=map(int,current.groups()[:4]);sid=TARGETS[target]['panes'][0]
 views=re.findall(rf'\[shell.app.viewport\] target={target} sid={sid} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',trace)
 if not views:raise ValueError('no exact parent-fitted first pane')
 x,y,_,_=map(int,views[-1]);return dict(task=task,sequence=sequence,x=x-8,y=y-36,w=w,h=h)

def frame_action(trace,frame):
 target=frame['target'];phase=frame['name'].rsplit('-',1)[-1]
 wanted={'typed':'Focus'if target==9 else'Open','opened':'Open','focused':'Focus','restored':'Restore','brought':'BringHere'}.get(phase)
 matching=[row for row in completed_actions(trace)if row['target']==target]
 if not matching or matching[-1]['action']!=wanted:raise ValueError('screenshot prefix predates its exact completed action')
 if target==17 and wanted=='Open':
  if not rows(trace,'app.surface.presented')or not any(row.get('target')=='17'and row.get('surface')=='302'and row.get('ok')=='1'for _,row in rows(trace,'app.surface.presented')):raise ValueError('queued successor Open lacks actual presentation in screenshot prefix')
 return matching[-1]

def pointer_move(q,x,y,resolution,text,wait,record,kind='move'):
 if not 0<=x<resolution[0]or not 0<=y<resolution[1]:raise ValueError('pointer outside framebuffer')
 raw_x=round(x*32767/(resolution[0]-1));raw_y=round(y*32767/(resolution[1]-1));start=len(text());record(dict(kind=kind,x=x,y=y,serial_start=start))
 q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':raw_x}},{'type':'abs','data':{'axis':'y','value':raw_y}}]})
 wait(rf'\[shell.abs.normalize\] raw_x={raw_x} raw_y={raw_y} .*reason=(?:ok|duplicate_sample)',start)

def pointer_park(q,resolution,text,wait,record,phase):
 # Alternate adjacent safe pixels so USB emits a fresh movement even when
 # successive screenshots need no intervening click. No timer substitutes
 # for the actual Shell normalization receipt.
 pointer_move(q,resolution[0]-20-(phase%2),10,resolution,text,wait,record,'park')

def pointer_click(q,x,y,resolution,text,wait,record,park=None):
 pointer_move(q,x,y,resolution,text,wait,record,'click')
 start=len(text());q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':True,'button':'left'}}]})
 wait(r'\[silk-shell.pointer.recv\] class=EV_BTN btn=1 pressed=true',start)
 start=len(text());q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':False,'button':'left'}}]})
 wait(r'\[silk-shell.pointer.recv\] class=EV_BTN btn=1 pressed=false',start)
 if park is not None:park()

def failed_frame(out,path,frame,snapshot,error):
 out=Path(out);path=Path(path);prefix=out/(frame['name']+'.failed.serial-prefix')
 if prefix.exists():raise ValueError('failed frame evidence already exists')
 prefix.write_bytes(snapshot)
 receipt=dict(frame,screenshot=path.name,screenshot_sha256=sha(path.read_bytes()),serial_bytes=len(snapshot),serial_sha256=sha(snapshot),serial_prefix=prefix.name,result=dict(ok=False,errors=[str(error)]))
 diagnostic=out/(frame['name']+'.failed.json')
 if diagnostic.exists():raise ValueError('failed frame diagnostic already exists')
 diagnostic.write_text(json.dumps(receipt,indent=2)+'\n');return receipt

def launcher_erased(trace,image,contract,resolution):
 view,lines=launcher_layout(trace);layout_pos=trace.rfind('[launcher.text.view]')
 if not any(p>layout_pos for p,_ in rows(trace,'command_palette.close')):raise ValueError('post-action frame lacks actual latest launcher close')
 linen=rows(trace,'linen.list.text')
 if not linen:raise ValueError('unknown same-color Linen overlap')
 item=linen[-1][1]
 try:
  x,y,w,h,scale,count=(int(item[k])for k in ('x','y','w','h','scale','rows'))
  if scale not in (1,2)or not 1<=count<=5 or w<=16 or h<=44 or int(item['text_color'],0)!=contract['launcher_foreground']:raise ValueError('invalid Linen text overlap')
 except(KeyError,TypeError)as exc:raise ValueError('incomplete Linen text overlap')from exc
 # Linen uses two heading lines and two lines/item; reserve all cells in
 # that actual text band conservatively, including its empty status lines.
 excluded=(x+8,y+36,x+w-8,min(y+h,y+36+(2+2*count)*9*scale))
 width,height,pixels=ppm_rgb(image,min_width=1,min_height=1)
 if(width,height)!=tuple(resolution):raise ValueError('wrong launcher-erasure framebuffer')
 color=contract['launcher_foreground'].to_bytes(3,'big');checked=skipped=remaining=0;line_checks=[]
 columns=min(80,(view['w']-16)//(6*view['scale']));limit=min(columns,40)-1
 if lines[0][1]!=contract['launcher_header']:raise ValueError('launcher header source differs')
 for row,line in lines:
  line_checked=line_skipped=line_remaining=0
  for col,char in enumerate(line[:limit]):
   for gy,bits in enumerate(glyph(contract['font'],char)):
    for gx in range(5):
     if not bits&(1<<(4-gx)):continue
     for dy in range(view['scale']):
      for dx in range(view['scale']):
       px=view['x']+8+(col*6+gx)*view['scale']+dx;py=view['y']+36+(row*9+gy)*view['scale']+dy
       if not 0<=px<width or not 0<=py<height:raise ValueError('launcher erasure mask outside framebuffer')
       if excluded[0]<=px<excluded[2]and excluded[1]<=py<excluded[3]:line_skipped+=1;continue
       line_checked+=1;line_remaining+=pixels[(py*width+px)*3:(py*width+px)*3+3]==color
  if line_checked<100:raise ValueError(f'insufficient unambiguous launcher erasure coverage: line={row}')
  checked+=line_checked;skipped+=line_skipped;remaining+=line_remaining
  line_checks.append(dict(row=row,checked_ink_pixels=line_checked,linen_overlap_skipped=line_skipped,remaining_ink=line_remaining))
 if remaining:raise ValueError(f'old launcher text remains: ink_pixels={remaining}')
 return dict(checked_ink_pixels=checked,linen_overlap_skipped=skipped,remaining_launcher_ink=remaining,lines=line_checks,scope='controlled source-bound text ink erasure')

def verify_frame(trace,image,frame,contract,resolution):
 kind=frame['kind'];target=frame.get('target',0)
 erasure=launcher_erased(trace,image,contract,resolution)if kind in ('spindle','quil','hidden')else None
 if kind=='boot':return no_ghost(trace,image,contract,resolution)
 if kind=='launcher':
  errors=launcher_pixels(trace,image,contract['font'],resolution)
  if errors:raise ValueError('launcher glyphs: '+str(errors))
  view,_=launcher_layout(trace);index=1 if target==9 else 0
  labels=re.findall(r'\[launcher.text.row\] index=(\d+) row=\d+ name=([^\r\n\[]*?) status=([^\r\n\[]*)',trace[trace.rfind('[launcher.text.view]'):])
  matching=[dict(name=name,status=status)for identity,name,status in labels if int(identity)==index]
  if len(matching)!=1:raise ValueError('missing exact selected launcher row')
  row=matching[0]
  if view['selected']!=index or row.get('name')!=frame['title']:raise ValueError('selected launcher action title differs')
  if frame['name']=='launcher-exhausted'and not ('CLOSED'in row.get('status','')and'EXHAUSTED'in row.get('status','')):raise ValueError('exhausted launcher status is not terminal')
  admitted=[p for p,r in rows(trace,'launcher.text.admitted')if r.get('complete')=='1'and r.get('selected')==str(index)and r.get('applied')=='1']
  layout_pos=trace.rfind('[launcher.text.view]')
  if not admitted or admitted[-1]<layout_pos:raise ValueError('current launcher lacks positive text Applied')
  return dict(title=row['name'],status=row.get('status'),geometry=view)
 if kind=='spindle':
  action=frame_action(trace,frame)
  errors=spindle_pixels(trace,image,contract['font'],target=target,require_prompt='> '+frame['marker'],require_echo=False,require_all_ink=False,resolution=resolution)
  if errors:raise ValueError('Spindle glyphs: '+str(errors))
  geometry=spindle_geometry(trace,target)
  if geometry['task']!=action['task']:raise ValueError('rendered Spindle task differs from completed held action')
  if geometry['sequence']<frame.get('minimum_sequence',0):raise ValueError('frame predates its explicit new viewport')
  cursor=list(re.finditer(rf'\[spindle.cursor.view\] target={target} sequence={geometry["sequence"]} pos=(\d+) len=(\d+) start=(\d+) end=(\d+) caret=(\d+)',trace))
  if not cursor or tuple(map(int,cursor[-1].groups()[:4]))!=(len(frame['marker']),len(frame['marker']),0,len(frame['marker'])):raise ValueError('typed prompt source changed/truncated during window action')
  if not re.search(rf'\[shell.focus.set\] id={TARGETS[target]["frame"]}(?:\s|$)',trace):raise ValueError('actual Shell frame focus absent')
  return dict(marker=frame['marker'],geometry=geometry,launcher_erasure=erasure)
 if kind=='quil':
  frame_action(trace,frame)
  errors=quil_pixels(trace,image,contract['font'],TARGETS[9]['marker'],target=9,resolution=resolution)
  if errors:raise ValueError('Quil glyphs: '+str(errors))
  view=quil_view(trace)
  if view['cursor']!=len(TARGETS[9]['marker']):raise ValueError('Quil caret changed during existing-frame action')
  edits=rows(trace,'quil.document.edit')
  if not edits or int(edits[-1][1].get('bytes','0'))!=len(TARGETS[9]['marker']):raise ValueError('Quil stored length changed during window action')
  return dict(marker=TARGETS[9]['marker'],geometry=view,launcher_erasure=erasure)
 if kind=='hidden':
  if not any(row.get('sid')==str(TARGETS[target]['frame'])for _,row in rows(trace,'silk.lifecycle.minimize.hidden')):raise ValueError('hidden screenshot prefix lacks actual minimize')
  width,height,pixels=ppm_rgb(image,min_width=1,min_height=1)
  if(width,height)!=tuple(resolution):raise ValueError('wrong hidden framebuffer resolution')
  g=frame['geometry'];color=contract['quil_foreground'if target==9 else'foreground'].to_bytes(3,'big');ink=0
  for y in range(max(50,g['y']+(12 if target==9 else 36)),min(height,g['y']+g['h'])):
   for x in range(max(0,g['x']+8),min(width,g['x']+g['w']-8)):ink+=pixels[(y*width+x)*3:(y*width+x)*3+3]==color
  if ink:raise ValueError('minimized Spindle ink remains in original frame body')
  return dict(remaining_spindle_ink=ink,launcher_erasure=erasure)
 raise ValueError('unknown captured frame kind')

def minimize_point(target,geometry,contract):
 chrome=contract['chrome'];gap=chrome['top_bar_light_gap_px'];size=chrome['top_bar_light_size_px'];height=chrome['top_bar_height_px']
 x=geometry['x']-(8 if target==9 else 0);y=geometry['y']-(36 if target==9 else 0)
 return x+2*gap+size+size//2,y+height//2

def frame_specs(contract):
 specs=[dict(name='boot-clean',kind='boot',target=0,marker='',title='')]
 for action,kind,name in [('Focus','launcher','focus'),('','quil','typed'),('','hidden','hidden'),('Restore','launcher','restore'),('','quil','restored'),('BringHere','launcher','bring'),('','quil','brought')]:
  specs.append(dict(name=('launcher'if kind=='launcher'else'quil')+'-9-'+name,kind=kind,target=9,marker=''if kind!='quil'else'',title=(action.replace('BringHere','Bring Here')+' Quil')if kind=='launcher'else''))
 for target in SPINDLES:
  for name,kind,action,marker in [('open','launcher','Open',''),('opened','spindle','',''),('typed','spindle','',TARGETS[target]['marker']),('focus','launcher','Focus',''),('focused','spindle','',TARGETS[target]['marker']),('hidden','hidden','',''),('restore','launcher','Restore',''),('restored','spindle','',TARGETS[target]['marker']),('bring','launcher','Bring Here',''),('brought','spindle','',TARGETS[target]['marker'])]:
   specs.append(dict(name=('launcher'if kind=='launcher'else'spindle')+f'-{target}-'+name,kind=kind,target=target,marker=marker,title=action+' Spindle'if action else''))
 specs.append(dict(name='launcher-exhausted',kind='launcher',target=17,marker='',title=contract['exhausted_title']))
 return specs

def frame_contract(frames,contract):
 specs=frame_specs(contract)
 if len(frames)!=len(specs):raise ValueError('missing finite launcher frame set')
 for row,expected in zip(frames,specs):
  if any(row.get(key)!=value for key,value in expected.items()):raise ValueError('captured frame kind/identity/marker/action expectation differs')
 return specs

class StartupPeer(ControlledPeer):
 def start(self):
  listener=self.listener(46078)
  def serve():
   try:self.serve_echo(listener)
   except Exception as exc:
    if not self.stop.is_set():self.errors.append(type(exc).__name__+': '+str(exc))
  thread=threading.Thread(target=serve,daemon=True);self.threads=[thread];thread.start()

def wire_check(raw):
 syns=syn_records(raw)
 if len(syns)!=1 or syns[0]['ticket']!=1:raise ValueError('unexpected extra/missing fixed startup TCP flow')
 wire=tcp_capture(raw,[dict(payload_hex=ECHO.hex(),response_hex=ECHO.hex())],guest_ports=(48001,),initiator='guest')
 if not wire['ok']:raise ValueError('startup Echo capture: '+str(wire['errors']))
 return wire

def replay(root,out,binding_path,iso):
 root=Path(root);out=Path(out);bound=frozen_binding(root,Path(iso),Path(binding_path));contract=source_contract(root);manifest=json.loads((out/'manifest.json').read_text())
 if manifest['binding']!=bound or manifest['source_sha256']!=contract['source_sha256']or manifest['scope']!=SCOPE:raise ValueError('replay source/image/runner scope differs')
 command=manifest['command'];profile=command_profile(command)
 if not profile['ok']or manifest['platform_profile']!=profile:raise ValueError('native actual RTC/CPU/target profile differs')
 raw=(out/'serial.log').read_bytes();trace=raw.decode('utf-8',errors='strict');resolution=tuple(map(int,manifest['resolution'].split('x')))
 if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',trace):raise ValueError('guest failure')
 checks=[];frames=json.loads((out/'frames.json').read_text())
 frame_contract(frames,contract)
 previous=0
 for frame in frames:
  if frame['serial_bytes']<previous:raise ValueError('captured prefix order reversed')
  previous=frame['serial_bytes'];prefix=input_gate.capture_prefix(raw,frame);path=out/frame['screenshot']
  if path.parent!=out or sha(path.read_bytes())!=frame['screenshot_sha256']:raise ValueError('screenshot path/hash differs')
  if frame['kind']=='hidden':
   prior='quil-9-typed'if frame['target']==9 else f'spindle-{frame["target"]}-focused'
   expected_geometry=next((row['geometry']for row in checks if row['name']==prior),None)
   if frame.get('geometry')!=expected_geometry:raise ValueError('hidden frame does not use actual preceding visible geometry')
  checks.append(dict(name=frame['name'],**verify_frame(prefix,path.read_bytes(),frame,contract,resolution)))
 by_name={check['name']:check for check in checks}
 for target in SPINDLES:
  typed=by_name[f'spindle-{target}-typed']['geometry'];focused=by_name[f'spindle-{target}-focused']['geometry'];restored=by_name[f'spindle-{target}-restored']['geometry'];brought=by_name[f'spindle-{target}-brought']['geometry']
  if focused!=typed or restored['task']!=typed['task']or brought['task']!=typed['task']or not typed['sequence']<restored['sequence']<brought['sequence']:raise ValueError('existing Focus changed geometry or Restore/BringHere used obsolete task/viewport')
 lifecycle=lifecycle_joins(trace,len(input_gate.capture_prefix(raw,frames[0])))
 wire=wire_check((out/'wire.pcap').read_bytes())
 if json.loads((out/'peer.json').read_text()).get('errors'):raise ValueError('startup fixture failed')
 return dict(ok=True,binding=bound,frames=checks,lifecycle=lifecycle,wire=wire,scope=SCOPE)

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--iso',required=True,type=Path);parser.add_argument('--binding',required=True,type=Path);parser.add_argument('--source-disk',type=Path);parser.add_argument('--out',required=True,type=Path);parser.add_argument('--check',action='store_true');parser.add_argument('--model',choices=('e1000','e1000e'),default='e1000e');parser.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');args=parser.parse_args()
 root=Path(__file__).resolve().parents[1];out=args.out.resolve();iso=args.iso.resolve(strict=True);binding_path=args.binding.resolve(strict=True)
 if args.check:
  result=replay(root,out,binding_path,iso);print(json.dumps(result,indent=2));return 0
 if args.source_disk is None:raise ValueError('native workflow requires source-disk')
 out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise ValueError('fresh evidence directory required')
 contract=source_contract(root);bound=frozen_binding(root,iso,binding_path);target=target_check(root)
 if not target['ok']:raise ValueError('non-Unix scalar target check failed')
 held=json.loads(binding_path.read_text())
 for name in ('scripts/launcher_workflow_gate.py','scripts/tests/test_launcher_workflow_gate.py'):
  if held['runner_sha256'].get(name)!=contract['source_sha256'][name]:raise ValueError('launcher runner not in exact frozen cohort')
 disk=out/'nvme.img';shutil.copyfile(args.source_disk,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-launcher-{os.getpid()}.sock';resolution=tuple(map(int,args.resolution.split('x')));command=desktop.native_command(iso,disk,serial,sock,pcap,args.model,args.resolution);profile=command_profile(command)
 if not profile['ok']:raise ValueError('unsupported native RTC/CPU command')
 manifest=dict(binding=bound,source_sha256=contract['source_sha256'],command=command,platform_profile=profile,scope=SCOPE,resolution=args.resolution,model=args.model,input_disk_sha256=sha(disk.read_bytes()),qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),target=target)
 (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');peer=StartupPeer(root,'ready');peer.start();proc=None;frames=[];actions=[];began=time.monotonic();error=None
 def raw():return serial.read_bytes()if serial.exists()else b''
 def text():return input_gate.complete_prefix(raw()).decode('utf-8',errors='strict')
 def health():
  if time.monotonic()-began>480:raise TimeoutError('bounded480second launcher workflow')
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if peer.errors:raise RuntimeError(str(peer.errors))
  if any(path.exists()and path.stat().st_size>MAX_PCAP for path in(serial,pcap)):raise ValueError('bounded raw evidence exceeded')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',text()):raise RuntimeError('guest failure')
 def wait(pattern,start=0,timeout=60):
  end=time.monotonic()+timeout
  while time.monotonic()<end:
   health();match=re.search(pattern,text()[start:])
   if match:return match
   time.sleep(.01)
  raise TimeoutError(pattern)
 def key(*keys):actions.append(dict(kind='key',keys=list(keys),serial_start=len(raw())));input_gate.hardware_keys(q,keys)
 park_phase=0
 def park():
  nonlocal park_phase
  pointer_park(q,resolution,text,wait,actions.append,park_phase);park_phase+=1
 def click(x,y):pointer_click(q,x,y,resolution,text,wait,actions.append,park)
 def picture(name,kind,target=0,marker='',title='',minimum_sequence=0,geometry=None):
  park();path=out/(name+'.ppm');deadline=time.monotonic()+30;last='no checked frame'
  while time.monotonic()<deadline:
   health();q.cmd('screendump',{'filename':str(path)});snapshot=input_gate.complete_prefix(raw());frame=dict(name=name,kind=kind,target=target,marker=marker,title=title,minimum_sequence=minimum_sequence,geometry=geometry)
   try:check=verify_frame(snapshot.decode(),path.read_bytes(),frame,contract,resolution);break
   except(ValueError,KeyError,IndexError,OverflowError)as exc:last=str(exc);time.sleep(.05)
  else:
   failed_frame(out,path,frame,snapshot,last)
   raise ValueError('frame '+name+': '+last)
  frame.update(screenshot=path.name,screenshot_sha256=sha(path.read_bytes()),serial_bytes=len(snapshot),serial_sha256=sha(snapshot),result=check);frames.append(frame);(out/'frames.json').write_text(json.dumps(frames,indent=2)+'\n');return check
 def launcher(target,action):
  start=len(text());key('grave_accent');wait(rf'\[launcher.open\] count={contract["launcher_app_count"]} selected=0 ok=1',start);wait(rf'\[shell.palette.open\] ok=1 selected=0 count={contract["launcher_count"]}',start)
  if target==9:
   start=len(text());key('j');wait(r'\[shell.palette.select\] old=0 new=1',start)
  family='Quil'if target==9 else'Spindle'
  title=(contract['exhausted_title']if action=='Exhausted'else action+' '+family if action!='BringHere'else'Bring Here '+family)
  picture('launcher-exhausted'if action=='Exhausted'else f'launcher-{target}-'+action.lower().replace('bringhere','bring'),'launcher',target,title=title)
  start=len(text());key('ret')
  if action=='Exhausted':wait(r'\[launcher.execute.denied\] idx=0 .*reason=boot_reserve_exhausted',start);return
  completion=r'activated=1 queued=1 accepted=1 ok=0'if target==17 and action=='Open'else r'activated=0 queued=0 accepted=1 ok=1'
  wait(rf'\[launcher.action\] target={target} task=\d+ frame={TARGETS[target]["frame"]} action={action} .*'+completion+r' pixels=unconfirmed',start)
  wait(r'\[command_palette.close\]',start)
 try:
  proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);deadline=time.monotonic()+20
  while not sock.exists():
   health()
   if time.monotonic()>deadline:raise TimeoutError('QMP unavailable')
   time.sleep(.02)
  q=QMP(sock)
  if args.resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=15);key('down');key('ret')
  wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0');wait(r'\[linen.disk.publish.done\]');wait(r'\[spindle.display.batch.admitted\] target=12 ')
  picture('boot-clean','boot')
  launcher(9,'Focus');start=len(text());key('ctrl','n');wait(r'\[quil.new.ok\] bytes=0',start)
  for index,char in enumerate(TARGETS[9]['marker'],1):
   start=len(text());key(char);wait(r'\[quil.document.edit\] revision=\d+ pending=0 bytes='+str(index)+r'(?:\s|$)',start)
  geometry=picture('quil-9-typed','quil',9)['geometry'];start=len(text());click(*minimize_point(9,geometry,contract));wait(r'\[silk.lifecycle.minimize.hidden\] sid=201 frame=\d+',start)
  picture('quil-9-hidden','hidden',9,geometry=geometry)
  launcher(9,'Restore');picture('quil-9-restored','quil',9)
  start=len(text());click(contract['workspace_x'][1]+10,contract['workspace_y']+10);wait(r'\[shell.scene.switch\] from=0 to=1',start)
  launcher(9,'BringHere');picture('quil-9-brought','quil',9)
  start=len(text());click(contract['workspace_x'][0]+10,contract['workspace_y']+10);wait(r'\[shell.scene.switch\] from=1 to=0',start)
  for target in SPINDLES:
   marker=TARGETS[target]['marker'];launcher(target,'Open')
   if target==17:wait(r'\[app.surface.presented\] target=17 surface=302 sequence=1 ok=1')
   picture(f'spindle-{target}-opened','spindle',target)
   for char in marker:key(char)
   picture(f'spindle-{target}-typed','spindle',target,marker)
   launcher(target,'Focus');picture(f'spindle-{target}-focused','spindle',target,marker)
   geometry=spindle_geometry(text(),target);start=len(text());click(*minimize_point(target,geometry,contract));wait(rf'\[silk.lifecycle.minimize.hidden\] sid={TARGETS[target]["frame"]} frame=\d+',start)
   picture(f'spindle-{target}-hidden','hidden',target,geometry=geometry)
   launcher(target,'Restore');picture(f'spindle-{target}-restored','spindle',target,marker,minimum_sequence=geometry['sequence']+1)
   destination=1 if target==12 else 0;start=len(text());click(contract['workspace_x'][destination]+10,contract['workspace_y']+10);wait(rf'\[shell.scene.switch\] from={1-destination} to={destination}',start)
   launcher(target,'BringHere');picture(f'spindle-{target}-brought','spindle',target,marker)
   start=len(text());key('f11');wait(rf'\[sext.app.pool.ready\] target={target} task=\d+ .*ok=1',start)
  launcher(17,'Exhausted')
 except Exception as exc:error=input_gate.failure_summary(exc)
 finally:
  (out/'actions.json').write_text(json.dumps(actions,indent=2)+'\n')
  if proc is not None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  peer.close();sock.unlink(missing_ok=True);(out/'peer.json').write_text(json.dumps(dict(errors=peer.errors),indent=2)+'\n')
  if error:(out/'result.json').write_text(json.dumps(dict(ok=False,errors=[error],scope=SCOPE),indent=2)+'\n')
 if error:raise RuntimeError(error)
 result=replay(root,out,binding_path,iso);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print('[launcher.workflow.gate.result] PASS source_bound=1 profiles=1 boots=1');return 0
if __name__=='__main__':raise SystemExit(main())
