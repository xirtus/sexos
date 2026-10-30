#!/usr/bin/env python3
"""Finite normal-image Quil toolbar/readability workflow; never builds."""
import argparse,ast,gzip,json,os,re,shutil,subprocess,time
from pathlib import Path
import launcher_workflow_gate as launch
import clipboard_workflow_gate as inputs
import https_import_gate as storage
import tls_responsiveness_gate as desktop
from desktop_close_gate import QMP
from network_contention_gate import frozen_binding,MAX_PCAP
from check_quil_ui_pixels import view as quil_view,verify as quil_pixels
from check_desktop_pixels import final_view,verify as linen_pixels
from check_document_disk import recover,object_bytes,fnv
from check_nonunix_target import inspect as target_check
from tls_exchange_gate import command_profile

PROFILE='ascii';NAME='readone';BODY=b'readable note';FINAL=BODY+b'z'
PROFILES={'ascii':('readone',b'readable note'),'utf8':('utfnote','café € 🙂 漢'.encode('utf-8'))}
def configure(profile):
 global PROFILE,NAME,BODY,FINAL
 if profile not in PROFILES:raise ValueError('unsupported text profile')
 PROFILE=profile;NAME,BODY=PROFILES[profile];FINAL=BODY+b'z'

STAGES=('edited','naming','saved','new-empty','linen-selected','reopened','resaved','commands','returned')
SCOPE=('One actual boot/profile: original BOOT-held Quil9, all four pointer toolbar actions using the shared responsive Layout, '
       'whole admitted Quil chrome/body and complete command/help glyphs, explicit named SaveAs, user-selected Linen reopen, then Save. '
       'Exact raw QJ02 journal bytes/name and two durable generations are independently decoded after VM termination. '
       'Finite captured startup Echo only. No new capability, Unix/POSIX, tiny native viewport, cold reboot, crash/power-loss, physical hardware, SMP or hostile-containment claim.')
sha=launch.sha

def source_contract(root):
 root=Path(root);c=launch.source_contract(root)
 paths=('crates/sex-pdx/src/document_view.rs','crates/sex-pdx/src/desktop_style.rs','servers/quil/src/view.rs','servers/quil/src/main.rs',
        'servers/quil/src/document_exchange.rs','servers/sexfiles/src/document.rs','servers/sexfiles/src/document_journal.rs','servers/sexfiles/src/vfs.rs',
        'scripts/quil_readability_gate.py','scripts/tests/test_quil_readability_gate.py','scripts/https_import_gate.py','scripts/check_document_disk.py','scripts/desktop_close_gate.py','scripts/check_nonunix_target.py','scripts/tls_exchange_gate.py','crates/sex-pdx/src/text_encoding.rs','servers/quil/src/unicode_input.rs')
 src={name:(root/name).read_text()for name in paths};layout=src[paths[0]];style=src[paths[1]];view=src[paths[2]];client=src[paths[4]]
 def constant(source,name):
  m=re.search(r'pub const '+name+r':[^=]+=\s*(\d+)\s*;',source)
  if not m:raise ValueError('missing shared metric '+name)
  return int(m[1])
 cell=constant(layout,'TOOLBAR_CELL');labels=re.search(r'pub const TOOLBAR_LABELS:[^=]+=\s*\[(.*?)\];',layout,re.S)
 if not labels:raise ValueError('missing shared toolbar labels')
 labels=[ast.literal_eval(x).decode('ascii')for x in re.findall(r'b"[^"\n]*"',labels[1])]
 if cell!=8 or len(labels)!=4 or any(len(x)!=cell for x in labels):raise ValueError('toolbar label/cell contract differs')
 required=('pub fn header_rows','pub fn toolbar_per_row','self.header_rows() as u32 * s::LINE * self.scale','col / TOOLBAR_CELL','rows >= desired_toolbar + 2')
 if not all(x in layout for x in required)or 'l.header_rows() * (l.cols + 1)'not in view:raise ValueError('responsive shared layout/header wiring absent')
 if 'Phase::Commit => {\n                if v as usize != self.len'not in client or 'Phase::End => {\n                if v != 0'not in client:raise ValueError('positive exact COMMIT/END source binding differs')
 if 'msg.caller_pd==3&&msg.caller_task==3'not in src[paths[3]]or 'Layout::new(target,vw,vh).hit('not in (root/'servers/silk-shell/src/main.rs').read_text():raise ValueError('exact held toolbar input route absent')
 commands=re.search(r'let labels = \[(.*?)\];',view,re.S)
 if not commands:raise ValueError('missing command palette labels')
 c.update(cell=cell,toolbar_labels=labels,commands=[ast.literal_eval(x).decode()for x in re.findall(r'b"[^"\n]*"',commands[1])],
          command_hints=['Ctrl A select all','Ctrl C/X/V copy/cut/paste','Ctrl Shift U Unicode','Esc returns to document'],
          normal_hint='Ctrl N/O/S | Ctrl Shift S | Esc menu',compact_hint='Ctrl N/O/S | Esc menu',tiny_hint='Esc menu',
          metrics={k:constant(style,k)for k in('INSET','TEXT_TOP','GLYPH_WIDTH','GLYPH_HEIGHT','ADVANCE','LINE')},name_keys=inputs.name_keyboard(NAME,src[paths[3]]))
 for hint in c['command_hints']+[c['normal_hint'],c['compact_hint'],c['tiny_hint']]:
  if 'b"'+hint+'"'not in view:raise ValueError('actual complete hint differs')
 linen_source=(root/'servers/silk-shell/src/desktop_content.rs').read_text()
 c['linen_headers']=[ast.literal_eval(x).decode()for x in re.findall(r'out.line\((b"[^"\n]*"),cols\)',linen_source)[:2]]
 if len(c['linen_headers'])!=2 or 'cols.saturating_sub(1).min(80)'not in linen_source:raise ValueError('precise same-color Linen glyph model differs')
 c['source_sha256'].update({name:sha(text.encode())for name,text in src.items()});return c

def layout(w,h,c):
 m=c['metrics'];scale=2 if w>=256 else 1;cols=min(79,max(1,max(0,w-2*m['INSET'])//(m['ADVANCE']*scale)-1));top=m['TEXT_TOP']
 rows=1+(h-top-m['GLYPH_HEIGHT']*scale)//(m['LINE']*scale)if w>=2*m['INSET']+m['GLYPH_WIDTH']*scale and h-top>=m['GLYPH_HEIGHT']*scale else 0
 desired=1 if cols>=4*c['cell']else 2 if cols>=2*c['cell']else 4 if cols>=c['cell']else 0
 toolbar=desired if desired and rows>=desired+2 else 0;per={1:4,2:2,4:1}.get(toolbar,0);header=int(rows>=2)
 return dict(scale=scale,cols=cols,rows=rows,top=top,toolbar_rows=toolbar,per=per,header=header)

def toolbar_point(action,v,c):
 l=layout(v['w'],v['h'],c)
 if action not in (1,2,3,4)or not l['per']or any(v[k]!=l[k]for k in ('cols','rows','top')):raise ValueError('toolbar not actually visible/current shared geometry')
 i=action-1;col=(i%l['per'])*c['cell']+c['cell']//2;row=l['header']+i//l['per'];m=c['metrics']
 if col>=l['cols']:raise ValueError('toolbar click beyond rendered columns')
 return(v['x']+m['INSET']+col*m['ADVANCE']*l['scale'],v['y']+l['top']+row*m['LINE']*l['scale']+m['GLYPH_HEIGHT']*l['scale']//2)

def chrome(v,stage,c):
 l=layout(v['w'],v['h'],c);lines=v['text'].splitlines()
 if any(v[k]!=l[k]for k in ('cols','rows','top'))or not lines:raise ValueError('view does not match current responsive layout')
 if stage=='commands':
  expected=[('> 'if i==0 else'  ')+label for i,label in enumerate(c['commands'])]+c['command_hints']
  if lines[l['header']:]!=expected:raise ValueError('complete command/selection/help text missing or clipped')
 else:
  if not l['per']or not l['header']:raise ValueError('native toolbar qualification requires useful full chrome')
  expected=[''.join(c['toolbar_labels'][i:i+l['per']])for i in range(0,4,l['per'])]
  hint=next(x for x in (c['normal_hint'],c['compact_hint'],c['tiny_hint'])if len(x)<=l['cols'])
  if lines[l['header']:l['header']+l['toolbar_rows']]!=expected or lines[l['header']+l['toolbar_rows']]!=hint:raise ValueError('complete toolbar/shortcut hint missing or clipped')
  body=FINAL if stage in ('resaved','returned')else b''if stage=='new-empty'else BODY
  rendered='\n'.join(lines[l['header']+l['toolbar_rows']+1:])
  if rendered!=body.decode()+'#':raise ValueError('unexpected editor bytes/caret in readability frame')
  title='Name: '+NAME+'#'if stage=='naming'else NAME if stage in ('saved','reopened','resaved','returned')else'UNTITLED'
  if (lines[0]!=title if stage=='naming'else not lines[0].startswith(title+' ')):raise ValueError('actual name/status header differs')
 return l

def launcher_erased(trace,image,c,resolution):
 """Exact controlled-fixture ink erasure; only actual possible Linen ink is excluded.

 A newly saved row extends Linen's broad text band across the old palette
 header. Its blank cells are not ambiguous same-color pixels, so excluding
 that whole band would discard valid evidence.
 """
 palette,old_lines=launch.launcher_layout(trace);last=trace.rfind('[launcher.text.view]')
 if not any(p>last for p,_ in launch.rows(trace,'command_palette.close')):raise ValueError('post-action frame lacks latest actual launcher close')
 linen,labels=final_view(trace)
 if linen['text_color']!=c['launcher_foreground']:raise ValueError('unknown same-color underlying app')
 from check_disk_persistence import ppm_rgb
 width,height,pixels=ppm_rgb(image,min_width=1,min_height=1)
 if (width,height)!=tuple(resolution):raise ValueError('wrong erasure framebuffer')
 def ink(line,x,y,scale):
  for col,char in enumerate(line):
   for gy,bits in enumerate(launch.glyph(c['font'],char)):
    for gx in range(5):
     if bits&(1<<(4-gx)):
      for dy in range(scale):
       for dx in range(scale):yield(x+(col*6+gx)*scale+dx,y+gy*scale+dy)
 columns=min(80,max(1,(linen['w']-16)//(6*linen['scale'])));limit=min(80,columns-1);allowed=set()
 underlying=list(enumerate(c['linen_headers']))+[(2+2*row,('> 'if identity==linen['selected']else'  ')+name)for identity,row,name in labels]
 for row,line in underlying:
  shown=line[:limit]
  if len(line)>limit and shown:shown=shown[:-1]+'~'
  allowed.update(ink(shown,linen['x']+8,linen['y']+36+row*9*linen['scale'],linen['scale']))
 color=c['launcher_foreground'].to_bytes(3,'big');checked=skipped=remaining=0;lines=[]
 limit=min(40,min(80,(palette['w']-16)//(6*palette['scale'])))-1
 for row,line in old_lines:
  count=overlap=bad=0
  for x,y in ink(line[:limit],palette['x']+8,palette['y']+36+row*9*palette['scale'],palette['scale']):
   if not 0<=x<width or not 0<=y<height:raise ValueError('old launcher erasure mask outside framebuffer')
   if (x,y)in allowed:overlap+=1;continue
   count+=1;bad+=pixels[(y*width+x)*3:(y*width+x)*3+3]==color
  if count<100:raise ValueError('insufficient unambiguous launcher erasure line')
  checked+=count;skipped+=overlap;remaining+=bad;lines.append(dict(row=row,checked=count,skipped=overlap,remaining=bad))
 if remaining:raise ValueError('old launcher text remains: '+str(remaining))
 return dict(checked_ink_pixels=checked,known_linen_ink_skipped=skipped,remaining_launcher_ink=remaining,lines=lines,scope='controlled precise underlying Linen glyph exclusion')

def verify_frame(trace,image,stage,c,resolution,object_id=None):
 if stage not in STAGES:raise ValueError('foreign readability stage')
 erased=launcher_erased(trace,image,c,resolution)
 if stage=='linen-selected':
  v,labels=final_view(trace)
  if not any(identity==v['selected']and name==NAME for identity,_,name in labels):raise ValueError('saved name is not actual selected Linen row')
  errors=linen_pixels(trace,image,c['font'],resolution)
 else:
  v=quil_view(trace);chrome(v,stage,c);errors=quil_pixels(trace,image,c['font'],None,target=9,resolution=resolution)
  if stage=='reopened':
   loads=list(re.finditer(r'\[quil.persist.load.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)',trace))
   if not loads or tuple(map(lambda x:int(x,0),loads[-1].groups()))!=(len(BODY),fnv(BODY),object_id):raise ValueError('reopened frame lacks actual exact selected journal load')
 if errors:raise ValueError('whole-view pixels: '+str(errors))
 return dict(geometry=v,launcher_erasure=erased)

def joins(trace,actions):
 _,catalog=launch.one(trace,'boot.app.resources',target=9,surface=156,required=31,coordinator=3)
 task=int(catalog.get('task','0'));focus=launch.completed_actions(trace)
 if task<=0 or len(focus)!=3 or any(r['target']!=9 or r['task']!=task or r['frame']!=201 or r['action']!='Focus'or r['activated']for r in focus):raise ValueError('launcher did not focus the exact existing held Quil incarnation')
 pointer=list(re.finditer(r'\[quil.ui.pointer\] action=(\d+) sequence=(\d+)',trace))
 if len(pointer)!=trace.count('[quil.ui.pointer]')or [int(m[1])for m in pointer]!=[4,1,2,3]or any(int(b[2])<=int(a[2])for a,b in zip(pointer,pointer[1:])):raise ValueError('missing/replayed/foreign toolbar admission')
 saves=list(re.finditer(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)',trace));durable=list(re.finditer(r'\[files.document.durable\] caller=9 object=(\d+) identity=(\d+) generation=(\d+) bank=([01]) bytes=(\d+) hash=(0x[0-9a-f]+) barriers=3 readback=1',trace))
 saved=list(re.finditer(r'\[quil.document.saved\] saved_revision=(\d+) revision=(\d+) dirty=([01])',trace))
 shell=list(re.finditer(r'\[shell.document.pointer\] target=9 action=(\d+) status=0',trace))
 if len(shell)!=trace.count('[shell.document.pointer]')or [int(m[1])for m in shell]!=[4,1,2,3]:raise ValueError('missing/foreign/denied held Shell toolbar route')
 if len(saved)!=2 or any(m[1]!=m[2]or m[3]!='0'for m in saved):raise ValueError('save lacks two exact positive current-revision receipts')
 if len(saves)!=2 or len(durable)!=2:raise ValueError('expected exactly two independently durable typed saves')
 obj=int(saves[0][3]);previous=0
 for index,(save,effect,body)in enumerate(zip(saves,durable,(BODY,FINAL))):
  if tuple(int(x,0)for x in save.groups())!=(len(body),fnv(body),obj)or int(effect[1])!=obj or int(effect[5])!=len(body)or int(effect[6],0)!=fnv(body)or not previous<effect.start()<save.start():raise ValueError('save/durable bytes, object, hash or order differs')
  previous=save.start()
  if not save.end()<saved[index].start()or index==0 and not saved[0].start()<pointer[1].start():raise ValueError('saved receipt missing in actual operation window')
 if int(durable[1][2])!=int(durable[0][2])or int(durable[1][3])!=int(durable[0][3])+1 or durable[0][4]==durable[1][4]:raise ValueError('same-object journal generations/banks differ')
 loads=list(re.finditer(rf'\[quil.persist.load.ok\] bytes={len(BODY)} hash={hex(fnv(BODY))} doc={obj}(?:\s|$)',trace))
 opened=re.search(r'\[shell.document.open\] target=9 linen=200',trace)
 intent=re.search(rf'\[linen.quil.disk_doc.intent\] object_id=\d+ path_id={obj} ok=1',trace)
 if len(loads)!=1 or not opened or not intent or not pointer[0].start()<saves[0].start()<pointer[1].start()<pointer[2].start()<opened.start()<intent.start()<loads[0].start()<pointer[3].start()<saves[1].start():raise ValueError('pointer New/Open/reopen/Save order differs')
 commands=list(re.finditer(r'\[command_palette.execute\] cmd=1 name=Open Quil(?:\r?\n|$)',trace))
 news=list(re.finditer(r'\[quil.new.ok\] bytes=0(?:\r?\n|$)',trace))
 if len(commands)!=2 or len(commands)!=trace.count('[command_palette.execute]')or len(news)!=2 or len(news)!=trace.count('[quil.new.ok]'):raise ValueError('expected two explicit Quil focus commands and two New effects')
 windows=((commands[0].end(),news[0].start()),(intent.end(),loads[0].start()),(commands[1].end(),pointer[3].start()))
 if not news[0].start()<pointer[0].start()or not pointer[1].start()<news[1].start()<pointer[2].start()or not loads[0].end()<=commands[1].start():raise ValueError('explicit/automatic focus operation windows differ')
 for event,(begin,end)in zip(focus,windows):
  if not begin<=event['offset']<end:raise ValueError('exact Quil focus missing from its explicit/open/reopen window')
 # Queued focus retains the original caller/subject correlation. Neither its
 # admission nor completion may be borrowed from a different operation window.
 for event in launch.parse_actions(trace):
  if not any(begin<=event['offset']<end for begin,end in windows):raise ValueError('queued/final focus escaped its actual operation window')
 toolbar=[row for row in actions if row.get('kind')=='toolbar']
 if len(toolbar)!=4 or [r['action']for r in toolbar]!=[4,1,2,3]:raise ValueError('missing actual toolbar input manifest')
 for row,receipt,held in zip(toolbar,pointer,shell):
  if not row['serial_start']<=receipt.start()<row['serial_end']or not row['serial_start']<=held.start()<row['serial_end']:raise ValueError('toolbar receipt outside actual click boundary')
  section=trace[row['serial_start']:row['serial_end']];down=re.search(r'\[silk-shell.pointer.recv\] class=EV_BTN btn=1 pressed=true',section);up=re.search(r'\[silk-shell.pointer.recv\] class=EV_BTN btn=1 pressed=false',section)
  if not down or not up or not down.start()<up.start()or not row['serial_start']+down.start()<receipt.start():raise ValueError('semantic toolbar lacks actual separate pointer edges')
 if re.search(r'\[quil.document.result\]|\[browser\.(?:http|https)\.intent\]|KERNEL PANIC|panicked at',trace):raise ValueError('unexpected document/network/guest failure')
 return dict(task=task,focus_windows=[dict(begin=a,end=b,completed=event['offset'])for event,(a,b)in zip(focus,windows)],object=obj,identity=int(durable[0][2]),generations=[int(m[3])for m in durable],saved_hashes=[m[2]for m in saves],pointer_sequences=[int(m[2])for m in pointer])

def journal(disk,meta):
 rows=storage.workspace(disk)['rows'];matches=[r for r in rows if r['name']==NAME]
 if len(matches)!=1 or matches[0]['object']!=meta['object']or matches[0]['identity']!=meta['identity']:raise ValueError('raw durable object/name/identity differs')
 current=recover(disk,meta['object']);raw,_=object_bytes(disk,meta['object']);banks=[]
 # Validate the original headers and payloads of both retained generations.
 import struct
 for bank in (0,1):
  header=raw[bank*32768:bank*32768+512]
  if len(header)!=512 or header[:4]!=b'QJ02'or header[4]!=meta['object']or struct.unpack_from('<I',header,8)[0]!=meta['identity']or fnv(header[:504])!=struct.unpack_from('<Q',header,504)[0]:raise ValueError('both raw QJ02 banks must validate')
  generation=struct.unpack_from('<Q',header,16)[0];length=struct.unpack_from('<I',header,24)[0];digest=struct.unpack_from('<Q',header,32)[0];payload=raw[bank*32768+512:bank*32768+512+length];banks.append((generation,payload,digest))
 banks.sort()
 if [(g,p,h)for g,p,h in banks]!=[(meta['generations'][0],BODY,fnv(BODY)),(meta['generations'][1],FINAL,fnv(FINAL))]or current['payload']!=FINAL:raise ValueError('raw journal does not retain both exact saves')
 return dict(object=meta['object'],name=NAME,identity=meta['identity'],generations=meta['generations'],bytes=len(FINAL),hash=hex(fnv(FINAL)),raw_object_sha256=sha(raw))

def unicode_joins(trace,actions):
 wanted=[ord(c)for c in BODY.decode('utf-8')if ord(c)>127]
 rows=[r for r in actions if r.get('kind')=='unicode']
 if [r.get('scalar')for r in rows]!=wanted:raise ValueError('missing/reordered Unicode keyboard intents')
 begins=list(re.finditer(r'\[quil.unicode.begin\] generation=(\d+) revision=(\d+) effects=0(?:\r?\n|$)',trace))
 commits=list(re.finditer(r'\[quil.unicode.commit\] bytes=(\d+) applied=(\d+) revision=(\d+) pending=(\d+)(?:\r?\n|$)',trace))
 if len(begins)!=len(wanted)or len(commits)!=len(wanted)or '[quil.unicode.refused]'in trace or '[quil.unicode.cancel]'in trace:raise ValueError('Unicode operation counts differ')
 previous=0
 for row,begin,commit in zip(rows,begins,commits):
  start,end=row['serial_start'],row['serial_end'];scalar=row['scalar'];digits=format(scalar,'x');encoded=chr(scalar).encode('utf-8')
  if row.get('digits')!=digits or not previous<=start<=begin.start()<begin.end()<=commit.start()<commit.end()<=end:raise ValueError('Unicode input window/order differs')
  window=trace[start:end]
  edits=list(re.finditer(r'\[quil.document.edit\] revision=(\d+) pending=(\d+) bytes=(\d+)(?:\r?\n|$)',window))
  if len(edits)!=1 or int(edits[0][1])!=int(begin[2])+1 or int(edits[0][2])!=0 or int(edits[0][3])!=row['expected_bytes']:raise ValueError('Unicode entry was not one exact atomic edit')
  if tuple(map(int,commit.groups()))!=(len(encoded),1,int(begin[2])+1,0)or int(begin[1])==0:raise ValueError('Unicode commit receipt differs')
  keys=[r['keys']for r in actions if r.get('kind')=='key'and start<=r['serial_start']<end]
  if keys!=[['ctrl','shift','u']]+[[d]for d in digits]+[['ret']]:raise ValueError('Unicode entry physical keys differ')
  previous=end
 return dict(profile=PROFILE,scalars=wanted,operations=len(wanted),atomic=True)


def replay(root,out,binding,iso,expected_profile=None):
 root=Path(root);out=Path(out);manifest=json.loads((out/'manifest.json').read_text());profile=manifest.get('text_profile','ascii')
 if expected_profile is not None and expected_profile!=profile:raise ValueError('text profile differs')
 configure(profile);bound=frozen_binding(root,Path(iso),Path(binding));c=source_contract(root)
 if manifest['binding']!=bound or manifest['source_sha256']!=c['source_sha256']or manifest['scope']!=SCOPE:raise ValueError('source/image/runner binding differs')
 if not command_profile(manifest['command'])['ok']or command_profile(manifest['command'])!=manifest['platform_profile']:raise ValueError('actual native profile differs')
 raw=(out/'serial.log').read_bytes();trace=raw.decode('utf-8',errors='strict');frames=json.loads((out/'frames.json').read_text());actions=json.loads((out/'actions.json').read_text());meta=joins(trace,actions);resolution=tuple(map(int,manifest['resolution'].split('x')))
 if [f['name']for f in frames]!=list(STAGES):raise ValueError('missing/reordered readability frames')
 checked=[];previous=0
 for frame in frames:
  prefix=inputs.capture_prefix(raw,frame);path=out/frame['screenshot']
  if path.parent!=out or sha(path.read_bytes())!=frame['screenshot_sha256']or frame['serial_bytes']<previous:raise ValueError('screenshot/hash/prefix ordering differs')
  previous=frame['serial_bytes'];checked.append(dict(name=frame['name'],**verify_frame(prefix,path.read_bytes(),frame['name'],c,resolution,meta['object'])))
 for row in (r for r in actions if r.get('kind')=='toolbar'):
  prefix=inputs.capture_prefix(raw,row);v=quil_view(prefix)
  if tuple(row['point'])!=toolbar_point(row['action'],v,c):raise ValueError('click did not use actual shared toolbar geometry')
  x,y=row['point'];rx=round(x*32767/(resolution[0]-1));ry=round(y*32767/(resolution[1]-1));section=trace[row['serial_start']:row['serial_end']]
  if not re.search(rf'\[shell.abs.normalize\] raw_x={rx} raw_y={ry} .*reason=(?:ok|duplicate_sample)',section):raise ValueError('toolbar point lacks exact actual pointer normalization')
 seed=gzip.decompress((out/'seed.img.gz').read_bytes())
 if sha(seed)!=manifest['input_disk_sha256']or any(r['name']==NAME or r['object']==meta['object']for r in storage.workspace(seed)['rows']):raise ValueError('saved named object was not fresh in the actual seed disk')
 disk=gzip.decompress((out/'nvme.img.gz').read_bytes())
 if sha(disk)!=manifest['final_disk_sha256']:raise ValueError('terminal raw disk hash differs')
 if json.loads((out/'peer.json').read_text())['errors']:raise ValueError('startup fixture failed')
 return dict(ok=True,text_profile=PROFILE,binding=bound,frames=checked,joins=meta,unicode_input=unicode_joins(trace,actions),journal=journal(disk,meta),wire=launch.wire_check((out/'wire.pcap').read_bytes()),scope=SCOPE)

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--binding',required=True,type=Path);p.add_argument('--source-disk',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');p.add_argument('--model',choices=('e1000','e1000e'),default='e1000e');p.add_argument('--text-profile',choices=tuple(PROFILES));p.add_argument('--check',action='store_true');a=p.parse_args()
 root=Path(__file__).resolve().parents[1];out=a.out.resolve();iso=a.iso.resolve(strict=True);binding=a.binding.resolve(strict=True)
 if a.check:print(json.dumps(replay(root,out,binding,iso,a.text_profile),indent=2));return 0
 configure(a.text_profile or 'ascii')
 if a.source_disk is None:raise ValueError('source disk required')
 out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise ValueError('fresh evidence output required')
 c=source_contract(root);bound=frozen_binding(root,iso,binding);target=target_check(root)
 if not target['ok']:raise ValueError('actual non-Unix target differs')
 for name in ('scripts/quil_readability_gate.py','scripts/tests/test_quil_readability_gate.py'):
  if json.loads(binding.read_text())['runner_sha256'].get(name)!=c['source_sha256'][name]:raise ValueError('runner omitted from frozen cohort')
 seed=a.source_disk.read_bytes()
 if any(r['name']==NAME for r in storage.workspace(seed)['rows']):raise ValueError('fixture name already exists')
 disk=out/'nvme.img';disk.write_bytes(seed);(out/'seed.img.gz').write_bytes(gzip.compress(seed,mtime=0));serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-quil-read-{os.getpid()}.sock';resolution=tuple(map(int,a.resolution.split('x')));command=desktop.native_command(iso,disk,serial,sock,pcap,a.model,a.resolution);profile=command_profile(command)
 if not profile['ok']:raise ValueError('unsupported native profile')
 manifest=dict(text_profile=PROFILE,binding=bound,source_sha256=c['source_sha256'],scope=SCOPE,command=command,platform_profile=profile,model=a.model,resolution=a.resolution,input_disk_sha256=sha(seed),target=target);(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
 proc=None;peer=launch.StartupPeer(root,'ready');frames=[];actions=[];began=time.monotonic();error=None;park_phase=0
 def raw():return serial.read_bytes()if serial.exists()else b''
 def text():return inputs.complete_prefix(raw()).decode('utf-8',errors='strict')
 def health():
  if time.monotonic()-began>480:raise TimeoutError('bounded readability workflow')
  if proc is not None and proc.poll()is not None:raise RuntimeError('guest exited')
  if peer.errors:raise RuntimeError(str(peer.errors))
  if any(x.exists()and x.stat().st_size>MAX_PCAP for x in (serial,pcap)):raise ValueError('bounded evidence exceeded')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',text()):raise RuntimeError('guest fault')
 def wait(pattern,start=0,timeout=60):
  end=time.monotonic()+timeout
  while time.monotonic()<end:
   health();m=re.search(pattern,text()[start:])
   if m:return m
   time.sleep(.01)
  raise TimeoutError(pattern)
 def key(*keys):actions.append(dict(kind='key',keys=list(keys),serial_start=len(text())));inputs.hardware_keys(q,keys)
 def park():
  nonlocal park_phase
  launch.pointer_park(q,resolution,text,wait,actions.append,park_phase);park_phase+=1
 def picture(name,object_id=None):
  park();path=out/(name+'.ppm');deadline=time.monotonic()+30;last='no complete frame'
  while time.monotonic()<deadline:
   health();q.cmd('screendump',{'filename':str(path)});snapshot=inputs.complete_prefix(raw());frame=dict(name=name)
   try:check=verify_frame(snapshot.decode(),path.read_bytes(),name,c,resolution,object_id);break
   except(ValueError,KeyError,IndexError,OverflowError)as exc:last=str(exc);time.sleep(.05)
  else:launch.failed_frame(out,path,frame,snapshot,last);raise ValueError(name+': '+last)
  frame.update(screenshot=path.name,screenshot_sha256=sha(path.read_bytes()),serial_bytes=len(snapshot),serial_sha256=sha(snapshot),result=check);frames.append(frame);(out/'frames.json').write_text(json.dumps(frames,indent=2)+'\n');return check
 def focus():
  start=len(text());key('grave_accent');wait(rf'\[launcher.open\] count={c["launcher_app_count"]} selected=0 ok=1',start);key('j');wait(r'\[shell.palette.select\] old=0 new=1',start);key('ret');wait(r'\[launcher.action\] target=9 task=\d+ frame=201 action=(?:Focus|Restore|BringHere) .*activated=0 queued=0 accepted=1 ok=1',start);wait(r'\[command_palette.close\]',start)
 def toolbar(action,pattern):
  deadline=time.monotonic()+20
  while True:
   snapshot=inputs.complete_prefix(raw())
   try:v=quil_view(snapshot.decode());point=toolbar_point(action,v,c);break
   except ValueError:
    health()
    if time.monotonic()>deadline:raise
    time.sleep(.01)
  start=len(snapshot.decode());row=dict(kind='toolbar',action=action,point=point,serial_start=start,serial_bytes=len(snapshot),serial_sha256=sha(snapshot));actions.append(row)
  launch.pointer_click(q,*point,resolution,text,wait,actions.append,park);wait(rf'\[quil.ui.pointer\] action={action} sequence=\d+',start);result=wait(pattern,start);row['serial_end']=len(text());return result
 def typing(body):
  total=0
  for char in body.decode('utf-8'):
   start=len(text());total+=len(char.encode('utf-8'))
   if ord(char)<128:
    key('spc'if char==' 'else char)
   else:
    row=dict(kind='unicode',scalar=ord(char),digits=format(ord(char),'x'),serial_start=start,expected_bytes=total);actions.append(row)
    key('ctrl','shift','u');wait(r'\[quil.unicode.begin\] generation=\d+ revision=\d+ effects=0',start)
    for digit in row['digits']:key(digit)
    key('ret');wait(r'\[quil.unicode.commit\] bytes='+str(len(char.encode('utf-8')))+r' applied=1 revision=\d+ pending=0',start);row['serial_end']=len(text())
   wait(r'\[quil.document.edit\] revision=\d+ pending=0 bytes='+str(total)+r'(?:\s|$)',start)

 try:
  peer.start();proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);deadline=time.monotonic()+20
  while not sock.exists():
   health()
   if time.monotonic()>deadline:raise TimeoutError('QMP unavailable')
   time.sleep(.02)
  q=QMP(sock)
  if a.resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=15);key('down');key('ret')
  wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0');wait(r'\[linen.disk.publish.done\]');focus();start=len(text());key('ctrl','n');wait(r'\[quil.new.ok\] bytes=0',start);typing(BODY);picture('edited')
  toolbar(4,r'\[quil.name.mode\] on=1 save_as=1')
  for k in c['name_keys']:key(k)
  picture('naming');start=len(text());key('ret');saved=wait(rf'\[quil.persist.save.ok\] bytes={len(BODY)} hash={hex(fnv(BODY))} doc=(\d+)',start);object_id=int(saved[1]);wait(r'\[quil.document.saved\].*dirty=0',start);wait(r'\[linen.list.label\] id=\d+ row=\d+ text='+NAME+r'(?:\s|$)',start);picture('saved')
  toolbar(1,r'\[quil.new.ok\] bytes=0');picture('new-empty');toolbar(2,r'\[shell.document.open\] target=9 linen=200')
  for _ in range(40):
   try:
    v,labels=final_view(text())
    if any(identity==v['selected']and name==NAME for identity,_,name in labels):break
   except ValueError:pass
   start=len(text());key('j');wait(r'\[shell.action.select_next_linen\] id=\d+',start)
  else:raise ValueError('fresh named Linen row never selected')
  picture('linen-selected');start=len(text());key('ret');wait(r'\[linen.open.complete\] status=0',start);wait(rf'\[quil.persist.load.ok\] bytes={len(BODY)} hash={hex(fnv(BODY))} doc={object_id}(?:\s|$)',start);focus();picture('reopened',object_id)
  start=len(text());key('z');wait(rf'\[quil.document.edit\] revision=\d+ pending=0 bytes={len(FINAL)}(?:\s|$)',start);toolbar(3,r'\[quil.document.saved\].*dirty=0');picture('resaved')
  start=len(text());key('esc');wait(r'\[quil.palette.action\] kind=esc toggle_on=1',start);picture('commands');start=len(text());key('esc');wait(r'\[quil.palette.action\] kind=esc clear=1',start);picture('returned')
 except Exception as exc:error=inputs.failure_summary(exc)
 finally:
  if proc is not None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  peer.close();sock.unlink(missing_ok=True);(out/'actions.json').write_text(json.dumps(actions,indent=2)+'\n');(out/'peer.json').write_text(json.dumps(dict(errors=peer.errors),indent=2)+'\n')
  terminal=disk.read_bytes();(out/'nvme.img.gz').write_bytes(gzip.compress(terminal,mtime=0));manifest['final_disk_sha256']=sha(terminal);(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
 if error:(out/'result.json').write_text(json.dumps(dict(ok=False,error=error,scope=SCOPE),indent=2)+'\n');raise RuntimeError(error)
 try:result=replay(root,out,binding,iso)
 except Exception as exc:(out/'result.json').write_text(json.dumps(dict(ok=False,error=inputs.failure_summary(exc),scope=SCOPE),indent=2)+'\n');raise
 (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print('[quil.readability.gate.result] PASS boots=1 source_bound=1');return 0
if __name__=='__main__':raise SystemExit(main())
