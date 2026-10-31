"""Immutable alpha captures and read-only replay; no guest creation or authority."""
import gzip,hashlib,json,math,re,time
from pathlib import Path
from network_contention_gate import frozen_binding
from check_desktop_pixels import font_from_source,verify as linen_pixels
from check_quil_ui_pixels import verify as quil_pixels,view as quil_view
from check_disk_persistence import ppm_rgb
from check_document_disk import recover,fnv
from silkbar_clock_gate import source_contract as bar_contract,verify_pixels as bar_pixels
from launcher_workflow_gate import source_contract as launcher_contract,verify_frame as launcher_frame

VERSION=2
SERIAL_UNITS='UTF-8 decoded characters; universal newlines; replacement on decode error'
RTC='base=utc,clock=host,driftfix=none'
MAX_CAPTURES=1024

def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def serial_text(path):return Path(path).read_text(encoding='utf-8',errors='replace')

def contract(root):
 root=Path(root);bar=bar_contract(root);launcher=launcher_contract(root)
 from https_asset_gate import script_dependencies
 paths=set(bar['source_sha256'])|set(launcher['source_sha256'])|set(script_dependencies(root,('scripts/quil_workflow_gate.py','scripts/alpha_evidence.py','scripts/tests/test_alpha_evidence.py')))
 paths.update(('servers/quil/src/main.rs','scripts/check_alpha_session.py'))
 return dict(font=font_from_source((root/'servers/sexdisplay/src/main.rs').read_text()),bar=bar,launcher=launcher,source_sha256={p:sha(root/p)for p in sorted(paths)})

def require_inputs(root,binding,c):
 held=json.loads(Path(binding).read_text())
 for name,digest in c['source_sha256'].items():
  if held.get('sources',{}).get(name,held.get('runner_sha256',{}).get(name))!=digest:raise ValueError('alpha checker/source not sealed: '+name)
 return held

def verify_frame(prefix,ppm,c,resolution,kind='quil',expected='',target=9,region=None):
 if kind=='quil':
  errors=quil_pixels(prefix,ppm,c['font'],expected,target=target,resolution=resolution)
  detail=None if errors else quil_view(prefix,target)
 elif kind=='linen':errors=linen_pixels(prefix,ppm,c['font'],resolution=resolution);detail=None
 elif kind=='launcher':
  if target!=9 or expected!='Open Quil':raise ValueError('alpha exhausted launcher must select logical Quil')
  detail=launcher_frame(prefix,ppm,dict(kind='launcher',target=9,name='launcher-exhausted',title=expected),c['launcher'],resolution);errors=[]
 elif kind=='empty':
  width,height,raw=ppm_rgb(ppm,min_width=1,min_height=1)
  if (width,height)!=tuple(resolution)or not isinstance(region,dict):raise ValueError('empty-region profile missing')
  x,y,w,h=(region[k]for k in('x','y','w','h'))
  if any(type(n)is not int for n in(x,y,w,h))or x<0 or y<0 or w<=0 or h<=0 or x+w>width or y+h>height:raise ValueError('empty region outside framebuffer')
  if not re.search(r'\[silk.lifecycle.minimize.hidden\] sid=201 frame=\d+',prefix):raise ValueError('empty region lacks actual minimize')
  ink=sum(raw[(yy*width+xx)*3:(yy*width+xx)*3+3]==b'\xe0\xf0\xff'for yy in range(y,y+h)for xx in range(x,x+w));errors=[]if ink==0 else['minimized Quil foreground remains'];detail=dict(region=region,foreground_pixels=ink)
 else:raise ValueError('unsupported alpha capture kind')
 if errors:return dict(ok=False,errors=errors)
 bar=bar_pixels(prefix,ppm,c['bar'],resolution,'Snapshot')
 return dict(ok=bar['ok'],errors=bar['errors'],kind=kind,view=detail,civil_clock=bar)

class Captures:
 def __init__(self,out,c,resolution):
  self.out=Path(out);self.c=c;self.resolution=tuple(resolution);self.frames=[];self.next=1
  (self.out/'frames').mkdir(exist_ok=False)
 def candidate(self,name):
  if self.next>MAX_CAPTURES:raise ValueError('bounded alpha capture count exceeded')
  slug=re.sub(r'[^a-zA-Z0-9_-]','_',name)[:80];return self.out/'frames'/f'{self.next:06d}-{slug}.candidate.ppm'
 def commit(self,name,candidate,prefix,proof,kind,expected,target,region,verified_ppm):
  candidate=Path(candidate);final=candidate.with_name(candidate.name.replace('.candidate.ppm','.ppm'))
  if not proof['ok']or final.exists():raise ValueError('failed/reused alpha capture')
  if candidate.read_bytes()!=verified_ppm:raise ValueError('alpha candidate changed after verification')
  with final.open('xb')as stream:stream.write(verified_ppm)
  candidate.unlink()
  record=dict(sequence=self.next,name=name,kind=kind,expected=expected,target=target,region=region,screenshot=str(final.relative_to(self.out)),screenshot_sha256=sha(final),serial_end=len(prefix),serial_sha256=hashlib.sha256(prefix.encode()).hexdigest(),source_sha256=self.c['source_sha256'],proof=proof)
  self.frames.append(record);self.next+=1;self.write(False);return record
 def write(self,complete):
  (self.out/'captures.json').write_text(json.dumps(dict(version=VERSION,complete=complete,serial_offset_units=SERIAL_UNITS,frames=self.frames),indent=2)+'\n')

def finish(out,manifest,src,captures,error=None):
 """Call only after the owned QEMU has exited; raw disk writers are gone."""
 out=Path(out);disk=out/'nvme.img'
 for path,name in((Path(src),'seed.img.gz'),(disk,'nvme.img.gz')):
  with path.open('rb')as source,(out/name).open('xb')as target:
   with gzip.GzipFile(filename='',fileobj=target,mode='wb',mtime=0)as z:
    while block:=source.read(1<<20):z.write(block)
 manifest.update(evidence_version=VERSION,error=error,final_disk_sha256=sha(disk),host_utc_end=time.time())
 captures.write(error is None)
 files=[p for p in out.rglob('*')if p.is_file()and p.name not in('manifest.json','verification.json','nvme.img')]
 manifest['raw_sha256']={str(p.relative_to(out)):sha(p)for p in sorted(files)}
 (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')

def verify_third_captures(third,frames,text,rows=None):
 last=18
 if rows is not None:
  from quil_reserve_catalog import verify_boot_quil
  verify_boot_quil(text,rows)
  chain=third.get('chain')
  if not isinstance(chain,list)or len(chain)!=len(rows)-2 or third.get('catalog')!=rows or third.get('append_count')!=len(chain)or third.get('final_target')!=rows[-1]['target']:raise ValueError('incomplete source-bound reserve chain')
  prior_end=0
  for index,(step,row)in enumerate(zip(chain,rows[2:])):
   identity=dict(target=row['target'],surface=row['surface'],logical=rows[0]['target'],incarnation=row['ordinal']+1,predecessor=rows[row['ordinal']-1]['target'])
   if any(step.get(k)!=v for k,v in identity.items()):raise ValueError('reserve chain identity differs')
   a,e,b,d,z=(step.get(k)for k in ('serial_start','edit_start','save_start','close_start','serial_end'))
   if any(type(v)is not int for v in(a,e,b,d,z))or not prior_end<=a<e<b<d<z<=len(text):raise ValueError('reserve chain serial windows overlap or exceed trace')
   prior_end=z;span=text[a:z];target=row['target'];surface=row['surface'];pre=identity['predecessor'];number=identity['incarnation']
   if not re.search(rf'\[app.activation.committed\] logical=9 incarnation={number} predecessor={pre} successor={target} task={target} surface={surface} .*runtime_mint=0',text[a:e])or not re.search(rf'\[app.surface.presented\] target={target} surface={surface} .*ok=1',text[a:e]):raise ValueError('reserve lacks exact activation/presentation')
   selected=[f for f in frames if f['name']==('third-quil-edit'if index==0 else f'reserve-quil-{target}-edit')]
   if len(selected)!=1 or selected[0]!=step.get('frame')or(selected[0]['kind'],selected[0]['target'],selected[0]['expected'])!=('quil',target,'t#')or not e<selected[0]['serial_end']<=b:raise ValueError('reserve edit lacks exact immutable current frame')
   saved=step.get('saved',{});body=saved.get('content','').encode('ascii');doc=saved.get('object');h=hex(fnv(body))
   if saved.get('bytes')!=len(body)or saved.get('hash')!=h or not body.endswith(b't')or not re.search(rf'\[quil.persist.save.ok\] bytes={len(body)} hash={h} doc={doc}(?:\s|$)',text[b:d]):raise ValueError('reserve lacks exact saved immutable payload')
   loads=step.get('loaded',[])
   if len(loads)!=2 or loads[1].get('object')!=doc or(loads[1].get('bytes'),loads[1].get('hash'))!=(len(body)-1,hex(fnv(body[:-1]))):raise ValueError('reserve recovery differs from preceding save')
   for load in loads:
    if not re.search(rf'\[quil.persist.load.ok\] bytes={load["bytes"]} hash={load["hash"]} doc={load["object"]}(?:\s|$)',text[a:e]):raise ValueError('reserve lacks actual exact native load')
   if index and (saved['content']!=chain[index-1]['saved']['content']+'t'or loads[0]!=chain[index-1]['loaded'][0]):raise ValueError('reserve durable generations are not continuous')
   if not re.search(rf'\[task.finish\] pd={target} task={target} reason=0 terminal=1',text[d:z])or not re.search(rf'\[sext.app.pool.ready\] target={target} task={target} .*ok=1',text[d:z]):raise ValueError('reserve lacks actual terminal pool cleanup')
  if chain[0]['frame']!=third.get('frame'):raise ValueError('first reserve frame differs')
  last=rows[-1]['target']
 for key,name,kind,target,expected in [('frame','third-quil-edit','quil',18,'t#'),('exhaustion_frame','third-quil-exhausted-launcher','launcher',9,'Open Quil')]:
  selected=[f for f in frames if f['name']==name]
  if len(selected)!=1 or selected[0]!=third.get(key)or(selected[0]['kind'],selected[0]['target'],selected[0]['expected'])!=(kind,target,expected):raise ValueError('third Quil lacks exact immutable '+key)
 prefix=text[:third['exhaustion_frame']['serial_end']];finished=prefix.rfind(f'[task.finish] pd={last} task={last} reason=0 terminal=1')
 selection=third.get('exhaustion_selection_offset');row=third.get('exhaustion_row_offset');layout=prefix.rfind('[launcher.text.view]')
 if type(selection)is not int or type(row)is not int or not 0<=finished<selection<row<len(prefix)or not selection<layout<len(prefix)or prefix.rfind('[launcher.text.view]',selection,row)<=selection:raise ValueError('exhausted launcher lacks current post-retirement selection/layout')
 if not re.match(r'\[shell.palette.select\] old=0 new=1(?=\n|$)',prefix[selection:])or not re.match(r'\[launcher.text.row\] index=1 row=1 name=Open Quil status=CLOSED / RESERVE EXHAUSTED(?=\n|$)',prefix[row:]):raise ValueError('exhausted launcher receipt offsets differ')
 if not re.search(rf'\[sext.app.pool.ready\] target={last} task={last} .*ok=1',prefix[finished:selection]):raise ValueError('exhausted launcher predates exact third-incarnation cleanup')

def replay(root,iso,binding,out):
 root=Path(root).resolve();out=Path(out).resolve();iso=Path(iso).resolve();binding=Path(binding).resolve()
 c=contract(root);require_inputs(root,binding,c);manifest=json.loads((out/'manifest.json').read_text());result=json.loads((out/'result.json').read_text());index=json.loads((out/'captures.json').read_text())
 if manifest.get('error')is not None or result.get('ok')is not True:raise ValueError('failed alpha attempt remains diagnostic')
 if manifest.get('evidence_version')!=VERSION or index.get('version')!=VERSION or index.get('complete')is not True or index.get('serial_offset_units')!=SERIAL_UNITS:raise ValueError('unsealed alpha capture registry')
 if manifest['binding']!=frozen_binding(root,iso,binding)or manifest['pixel_source_sha256']!=c['source_sha256']:raise ValueError('alpha source/image binding differs')
 command=manifest['command'];resolution=tuple(map(int,manifest['resolution'].split('x')))
 if manifest['model']not in('e1000','e1000e')or command.count('-M')!=1 or command[command.index('-M')+1]!='q35' or command.count(manifest['model']+',netdev=peer,mac=52:54:00:12:34:56')!=1:raise ValueError('actual alpha platform/NIC differs')
 if command.count('-rtc')!=1 or command[command.index('-rtc')+1]!=RTC or command[command.index('-cdrom')+1]!=str(iso):raise ValueError('actual alpha RTC/image differs')
 if command.count('VGA,xres='+manifest['resolution'].replace('x',',yres='))!=1:raise ValueError('actual alpha resolution differs')
 raw={str(p.relative_to(out))for p in out.rglob('*')if p.is_file()and p.name not in('manifest.json','verification.json','nvme.img')}
 if raw!=set(manifest['raw_sha256']):raise ValueError('alpha raw closure differs')
 for name,digest in manifest['raw_sha256'].items():
  p=out/name
  if p.is_symlink()or not p.resolve().is_relative_to(out)or sha(p)!=digest:raise ValueError('changed/unsafe alpha raw artifact: '+name)
 for name,key in(('seed.img.gz','input_disk_sha256'),('nvme.img.gz','final_disk_sha256')):
  if hashlib.sha256(gzip.decompress((out/name).read_bytes())).hexdigest()!=manifest[key]:raise ValueError('alpha disk bytes differ')
 text=serial_text(out/'serial.log');frames=index['frames']
 if not 1<=len(frames)<=MAX_CAPTURES or len({f['screenshot']for f in frames})!=len(frames)or result['stages']!=frames:raise ValueError('alpha capture phase closure differs')
 previous=0;checked=[]
 for seq,frame in enumerate(frames,1):
  if frame['sequence']!=seq or not previous<frame['serial_end']<=len(text)or frame['source_sha256']!=c['source_sha256']:raise ValueError('reused/reordered/foreign alpha capture')
  prefix=text[:frame['serial_end']];previous=frame['serial_end']
  if hashlib.sha256(prefix.encode()).hexdigest()!=frame['serial_sha256']or sha(out/frame['screenshot'])!=frame['screenshot_sha256']:raise ValueError('alpha screenshot/prefix changed')
  proof=verify_frame(prefix,(out/frame['screenshot']).read_bytes(),c,resolution,frame['kind'],frame['expected'],frame['target'],frame['region'])
  if not proof['ok']or proof!=frame['proof']:raise ValueError('alpha raw glyph proof differs: '+frame['name'])
  checked.append(dict(sequence=seq,name=frame['name'],kind=frame['kind']))
 if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow|\[fault.kill\]|\[sexfiles.defer.drop\]',text):raise ValueError('alpha guest/request/record failure')
 from clock_source_evidence import validate_clock_sources
 for s in validate_clock_sources(text):
  if not math.floor(manifest['host_utc_start'])-1<=int(s['seconds'])<=math.ceil(manifest['host_utc_end'])+1:raise ValueError('alpha civil snapshot outside actual host UTC bracket')
 if bool(manifest['options'].get('pending_io_close'))!=bool(result.get('pending_io_close')):raise ValueError('pending fixture mode/result differs')
 if result.get('pending_io_close'):
  from pending_io_close import verify as pending_verify
  pending_verify(out,result['pending_io_close'],frames,text,command)
 from check_alpha_session import session_errors,boot_lifecycle_errors,quil_close_errors,cycle_errors,terminal_pending_errors
 if result.get('soak'):
  errors=quil_close_errors(result['soak'],text)+cycle_errors(result['soak'])+terminal_pending_errors(text,include_quil=result['soak']['pending_quil_close'])
  if errors:raise ValueError('alpha exact cycle/close evidence: '+str(errors))
  soak=result['soak'];cycles=soak['cycles'];windows=soak['cycle_windows'];body=result['documents'][1]['content'];suffix=(soak['third_quil'].get('append_count',1)if soak.get('third_quil')else 0)
  if soak.get('third_quil'):
   from quil_reserve_catalog import quil_catalog
   reserve_rows=quil_catalog(root);verify_third_captures(soak['third_quil'],frames,text,reserve_rows)
  saved_body=body[:-suffix]if suffix else body;initial=saved_body[:-cycles]
  if saved_body!=initial+'q'*cycles:raise ValueError('alpha cycle edits differ from final durable payload')
  for window in windows:
   cycle=window['cycle'];a,b=window['serial_start'],window['serial_end']
   if not 0<=a<b<=len(text):raise ValueError('alpha cycle serial window outside raw trace')
   span=text[a:b];payload=(initial+'q'*cycle).encode();doc=result['documents'][1]['object']
   required=[r'\[quil.document.edit\].*bytes='+str(len(payload))+r'(?:\s|$)',r'\[quil.persist.save.ok\] bytes='+str(len(payload))+r' hash='+re.escape(hex(fnv(payload)))+r' doc='+str(doc)+r'(?:\s|$)',r'\[spindle.probe.result\] target=12 kind=done job=Stat\(0\).*error=0',r'\[spindle.instance.command\] target=12 .*name=echo ok=1',r'\[spindle.history.receipt\] target=12 .*kind=saved .*error=0',r'\[linen.remote.snapshot.ok\].*complete=1']
   if any(not re.search(pattern,span)for pattern in required):raise ValueError('alpha cycle lacks exact edit/save/Spindle/history/Linen work: '+str(cycle))
   cycle_frames=[f for f in frames if a<f['serial_end']<=b and f['kind']=='quil'and f['target']==9]
   if len(cycle_frames)!=2 or [f['name']for f in cycle_frames]!=[f'soak-{cycle:04d}-current',f'soak-{cycle:04d}-saved']or [f['expected']for f in cycle_frames]!=['#','uibeta']:raise ValueError('alpha active cycle lacks exact immutable input/save pixels')
 if result['acceptance']:
  if manifest['options']['rehearsal']or manifest['options']['soak_seconds']<1800:raise ValueError('rehearsal/options cannot become accepted soak')
  errors=session_errors(result)
  if errors:raise ValueError('alpha acceptance: '+str(errors))
 elif result.get('soak'):
  errors=boot_lifecycle_errors(result)
  if errors:raise ValueError('alpha rehearsal lifecycle: '+str(errors))
 from check_desktop_close_log import check as cleanup
 if result.get('soak'):
  errors=cleanup(text,include_successor=True,include_spindle_successor=True,include_third_quil=bool(result['soak'].get('third_quil')),additional_quil=reserve_rows[3:]if result['soak'].get('third_quil')else ())
  if errors:raise ValueError('alpha terminal cleanup: '+str(errors))
 disk=gzip.decompress((out/'nvme.img.gz').read_bytes());documents=[]
 for expected in result['documents']:
  found=recover(disk,expected['object']);payload=found.pop('payload')
  if payload!=expected['content'].encode('ascii')or len(payload)!=expected['bytes']or found['hash']!=expected['hash']:raise ValueError('alpha durable document differs')
  documents.append(found)
 return dict(ok=True,acceptance=result['acceptance'],binding=manifest['binding'],verified_captures=len(checked),captures=checked,documents=documents,elapsed_seconds=(result.get('soak')or{}).get('elapsed_seconds'),cycles=(result.get('soak')or{}).get('cycles'),scope='Read-only replay of immutable alpha prefixes, exact glyphs, civil snapshots, source-bound image, terminal durable bytes and cleanup; rehearsal cannot become sustained acceptance. Not global epoch replay or general daily-driver qualification.')
