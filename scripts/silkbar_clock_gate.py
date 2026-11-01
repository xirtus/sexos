#!/usr/bin/env python3
"""Read-only joins for BOOT-held status-bar civil snapshots and actual pixels.

UTC is supplied by the explicit q35 profile, never inferred from render cadence.
These helpers neither launch guests nor grant source/authenticated-time trust.
"""
import datetime,hashlib,re
from pathlib import Path
from check_desktop_pixels import font_from_source,glyph
from check_disk_persistence import ppm_rgb
from clock_source_evidence import validate_clock_sources

STATES={0:'Unknown',1:'Snapshot',2:'Unavailable',3:'Rollback',4:'Exhausted'}

# Fixed English presentation, independent of the guest formatter and locale.
MONTHS=('JAN','FEB','MAR','APR','MAY','JUN','JUL','AUG','SEP','OCT','NOV','DEC')
LEGACY_PROFILE='q35-caption-v1'
DATE_PROFILE='date-caption-v1'

def _function(source,name):
 # These finite renderer helpers contain no braces in strings. Strip comments
 # before balancing so documentation cannot alter the accepted function body.
 source=re.sub(r'//[^\n]*','',source)
 matches=list(re.finditer(r'\bfn '+re.escape(name)+r'\s*\(',source))
 assert len(matches)==1,'missing/ambiguous renderer helper: '+name
 start=matches[0].start();opening=source.index('{',matches[0].end());depth=1;at=opening+1
 while depth and at<len(source):
  depth+=(source[at]=='{')-(source[at]=='}');at+=1
 assert depth==0,'unterminated renderer helper: '+name
 return source[start:at]

def _caption_profile(display):
 lines=_function(display,'civil_clock_lines')
 match=re.search(r'match clock\.state\(\)\s*\{(.*?)\n\s*\};',lines,re.S)
 assert match,'unsupported civil state selector'
 arm=re.compile(r'State::(Snapshot|Rollback|Exhausted|Unavailable|Unknown)(\s+if\s+clock\.is_stale\(\))?\s*=>\s*\(b"([^"]*)",\s*b"([^"]*)"\)\s*,')
 found=list(arm.finditer(match[1]));assert re.sub(r'\s+','',arm.sub('',match[1]))=='','unknown civil state arm'
 rows=[(m[1],bool(m[2]),m[3],m[4])for m in found]
 first=[('Snapshot',False,'UTC 00:00:00'),('Rollback',False,'UTC blocked'),('Exhausted',False,'UTC ended'),('Unavailable',False,'UTC unknown'),('Unknown',True,'UTC stale'),('Unknown',False,'UTC unknown')]
 legacy=['q35 1s snap','q35 1s stop','src? prec?','q35 1s fail','q35 1s old','src? prec?']
 modern=['','Time blocked','Updates ended','Source error','Sample stale','Awaiting UTC']
 expected=lambda captions:[(*a,b)for a,b in zip(first,captions)]
 compact=lambda value:re.sub(r'\s+','',value)
 if rows==expected(legacy):
  assert 'civil_clock_date('not in lines,'ambiguous date formatter with legacy caption'
  assert 'if bar.civil_enabled {return civil_clock_fg_at(x,y,bar);}'in display,'unknown legacy civil dispatch'
  return LEGACY_PROFILE
 assert rows==expected(modern),'unknown/ambiguous civil caption profile'
 code=compact(lines)
 assert 'ifclock.state()==State::Snapshot{lines[1]=civil_clock_date(clock.seconds());'in code,'date is not bound exclusively to the healthy snapshot'
 assert code.count('civil_clock_date(')==1,'date formatter called more than once per snapshot'
 date=_function(display,'civil_clock_date')
 assert re.search(r'fn civil_clock_date\(seconds:u32\)->\[u8;13\]',date),'unsupported finite date signature'
 assert 'sex_time::SexTimestamp::from_unix(seconds as i64,0)'in date and '.and_then(sex_time::Gregorian::from_timestamp)'in date,'date conversion not bound to actual sex-time API'
 assert tuple(re.findall(r'b"([A-Z]{3})"',date))==MONTHS,'unknown/ambiguous English month table'
 _date_render_shape(display)
 return DATE_PROFILE

def _date_render_shape(display):
 # Recognize the archived cached-row API and the explicit owned-scene API.
 # This checks the clock's source/caching route, not complete scene publication.
 compact=lambda value:re.sub(r'\s+','',value)
 top=compact(_function(display,'top_strip_pixel'))
 old='fntop_strip_pixel(x:usize,y:usize,bar:&SilkBar,lines:&[[u8;13];2])->u32'
 new='fntop_strip_pixel(x:usize,y:usize,bar:&SilkBar,lines:&[[u8;13];2],scene:&scene::SceneSnapshot)->u32'
 header=top.split('{',1)[0]
 assert header in(old,new),'unknown top-strip clock rendering shape'
 scene_backed=header==new
 prepared='fn render_prepared('in display
 batched='fn raster_batch('in display
 assert not prepared or scene_backed,'prepared output requires an owned scene'
 for name in('civil_clock_fg_at','clock_fg_at'):
  helper=compact(_function(display,name))
  assert helper.split('{',1)[0]=='fn'+name+'(x:usize,y:usize,bar:&SilkBar,lines:&[[u8;13];2])->Option<u32>','uncached civil pixel signature: '+name
 for name in('civil_clock_fg_at','clock_fg_at','top_strip_pixel'):
  helper=compact(_function(display,name))
  assert all(call not in helper for call in('civil_clock_lines(','civil_clock_date(')),'formatter invoked per pixel: '+name
 assert 'ifbar.civil_enabled{returncivil_clock_fg_at(x,y,bar,lines);}'in compact(_function(display,'clock_fg_at')),'cached civil dispatch missing'
 assert top.count('clock_fg_at(x,y,bar,lines)')==1,'top-strip cached glyph call missing'
 if scene_backed:
  assert 'bar_color(x,y,bar,scene)'in top,'top-strip background bypasses captured scene'
 for name,bar in((('raster_full'if prepared else'render'),'bar'),(('raster_strip'if prepared else'redraw_top_strip'),'bar'),('top_strip_render_proof','proof_bar')):
  helper=compact(_function(display,name));cache='letclock_lines=civil_clock_lines(&'+bar+'.civil_clock);'
  assert helper.count(cache)==1 and helper.count('civil_clock_lines(')==1 and helper.count('letclock_lines=')==1,'missing/duplicate per-pass clock snapshot: '+name
  assert helper.index(cache)<helper.index('foryin0..'),'calendar snapshot moved into pixel iteration: '+name
  if batched and name in('raster_full','raster_strip'):
   assert 'scene:&scene::SceneSnapshot'in helper.split('{',1)[0],'batch raster must borrow the shared scene'
   assert 'SceneLease::capture('not in helper and helper.count('letbar=&scene.bar;')==1,'batch raster recaptures or replaces its clock scene'
   assert helper.index('letbar=&scene.bar;')<helper.index(cache),'batch clock binding follows formatting'
  elif scene_backed:
   capture='letSome(lease)=SceneLease::capture('+('PROOF_W,PROOF_H,&proof_bar'if name=='top_strip_render_proof'else'w,h,bar')+')else{return'+('false'if name in('render','raster_full','raster_strip')else'')+';};'
   chain=capture+'letscene=lease.view();let'+bar+'=&scene.bar;'
   assert helper.count(chain)==1 and helper.count('SceneLease::capture(')==1,'missing/ambiguous scene-backed clock custody: '+name
   assert helper.count('letscene=')==1 and helper.count('let'+bar+'=')==1 and helper.count('let'+bar+'=&scene.bar;')==1,'clock scene binding replaced: '+name
   assert helper.index(chain)<helper.index(cache),'clock cached before owned scene capture: '+name
  else:
   assert 'SceneLease::capture('not in helper and '=&scene.bar;'not in helper,'ambiguous archived clock rendering shape: '+name
  call=('clock_fg_at(x,y,&proof_bar,&clock_lines)'if name=='top_strip_render_proof'else'top_strip_pixel(x,y,bar,&clock_lines'+(',scene'if scene_backed else'')+')')
  assert helper.count(call)==1,'cached rows not passed to actual renderer: '+name
 if batched:
  writer=compact(_function(display,'render_prepared'));batch=compact(_function(display,'raster_batch'))
  capture='letSome(lease)=SceneLease::capture(w,h,bar)'
  commit='preparation.commit(|pixels|publish_frame(fb,w,h,pixels))'
  drawn='civil_clock_drawn(&scene.bar,w,h,plan.label());'
  assert writer.count(capture)==1 and writer.count('letscene=lease.view();')==1,'one owned clock scene per batch'
  assert writer.index(capture)<writer.index('raster_batch(&target,scene,plan)')<writer.index(commit)<writer.index(drawn),'batch clock custody/publication order'
  assert 'if!'+commit+'{backup.restore();returnNone;}'in writer,'failed batch releases clock receipt'
  assert 'ifplan.clock_dirty(){'+drawn+'}'in writer,'nonclock batch releases clock receipt'
  assert batch.count('raster_full(target,scene)')==1 and batch.count('raster_strip(target,scene)')==1,'clock passes do not share the batch scene'
 elif prepared:
  writer=compact(_function(display,'render_prepared'))
  commit='preparation.commit(|pixels|publish_frame(fb,w,h,pixels))'
  drawn='civil_clock_drawn(&captured_bar,w,h,paint.label());'
  assert writer.count(commit)==1 and writer.count(drawn)==1 and writer.index(commit)<writer.index(drawn),'clock receipt precedes completed CPU publication'
  assert 'if!'+commit+'{backup.restore();returnfalse;}'in writer,'failed publication releases a clock receipt'
  assert 'letcaptured_bar=*bar;'in writer,'publication clock metadata is not captured'
  for name,kind in(('render','Full'),('redraw_top_strip','Strip')):
   wrapper=compact(_function(display,name))
   assert wrapper.count('render_prepared(fb,w,h,bar,Paint::'+kind+')')==1,'clock output bypasses preparation: '+name
 return scene_backed

def source_contract(root):
 root=Path(root);names=['crates/silkbar-model/src/lib.rs','crates/silkbar-model/src/civil_clock.rs','crates/sex-pdx/src/clock_source.rs','kernel/src/clock_source.rs','kernel/src/init.rs','servers/silkbar/src/main.rs','servers/silkbar/src/civil_clock.rs','servers/sexdisplay/src/main.rs','scripts/silkbar_clock_gate.py','scripts/clock_source_evidence.py','scripts/browser_source_evidence.py','crates/sex-pdx/src/app_catalog.rs','crates/sex-pdx/src/browser_sources.rs'];texts={n:(root/n).read_text()for n in names};model=texts[names[0]];display=texts['servers/sexdisplay/src/main.rs'];abi=texts['crates/sex-pdx/src/clock_source.rs']
 profile=_caption_profile(display)
 if profile==DATE_PROFILE:
  for name in('servers/sexdisplay/Cargo.toml','crates/sex-time/Cargo.toml','crates/sex-time/src/lib.rs'):texts[name]=(root/name).read_text()
  assert re.search(r'^sex-time\s*=\s*\{\s*path\s*=\s*"../../crates/sex-time",\s*default-features\s*=\s*false\s*\}',texts['servers/sexdisplay/Cargo.toml'],re.M),'Display calendar dependency not bound to local no_std sex-time'
  if _date_render_shape(display):
   for name in('servers/sexdisplay/src/scene.rs','servers/sexdisplay/src/bitmap_binding.rs','servers/sexdisplay/src/bitmap_transaction.rs','servers/sexdisplay/src/bitmap_stores.rs','servers/sexdisplay/src/surface_lifecycle.rs','servers/sexdisplay/src/surface_groups.rs','crates/sex-pdx/src/app_lifecycle.rs','crates/sex-pdx/src/bitmap_batch.rs'):
    texts[name]=(root/name).read_text()
   if 'fn render_prepared('in display:
    for name in('servers/sexdisplay/src/frame_output.rs','crates/sex-pdx/src/display_frames.rs','crates/sex-pdx/src/memory_region.rs'):
     texts[name]=(root/name).read_text()
 if 'fn raster_batch('in display:texts['servers/sexdisplay/src/paint_batch.rs']=(root/'servers/sexdisplay/src/paint_batch.rs').read_text()
 dimensions={}
 for name in('CHIP_X3','CHIP_Y','CLOCK_W','CHIP_H'):
  m=re.search(r'pub const '+name+r': usize = (\d+);',model);assert m,'missing source-bound civil geometry';dimensions[name]=int(m[1])
 color=re.search(r'pub const DEFAULT_THEME: Theme = Theme \{.*?text:\s*(0x[0-9A-Fa-f]+)',model,re.S);assert color,'missing civil foreground color'
 bounds={}
 for name in('MIN_SECONDS','MAX_SECONDS'):
  m=re.search(r'pub const '+name+r': u64 = ([\d_]+);',abi);assert m;bounds[name]=int(m[1].replace('_',''))
 assert 'pub const STATUS_BAR: Identity = Identity { pd: 6, task: 6 };'in abi
 if 'fn raster_batch('in display:
  assert 'ifplan.clock_dirty(){civil_clock_drawn(&scene.bar,w,h,plan.label());}'in re.sub(r'\s+','',_function(display,'render_prepared'))
 elif 'fn render_prepared('in display:
  assert 'ifmatches!(paint,Paint::Full|Paint::Strip){civil_clock_drawn(&captured_bar,w,h,paint.label());}'in re.sub(r'\s+','',_function(display,'render_prepared'))
 else:
  assert 'civil_clock_drawn(bar,w,h,"strip")'in display and 'civil_clock_drawn(bar,w,h,"full")'in display
 return dict(font=font_from_source(display),dimensions=dimensions,color=int(color[1],16)&0xffffff,bounds=bounds,caption_profile=profile,source_sha256={n:hashlib.sha256(t.encode()).hexdigest()for n,t in texts.items()})

def expected_clock_lines(contract,state,seconds,stale=False):
 profile=contract['caption_profile'];assert profile in(LEGACY_PROFILE,DATE_PROFILE),'unknown civil caption profile'
 if state=='Snapshot':
  assert type(seconds)is int and contract['bounds']['MIN_SECONDS']<=seconds<=contract['bounds']['MAX_SECONDS'],'snapshot outside validated source range'
  date=datetime.datetime.fromtimestamp(seconds,datetime.timezone.utc)
  first=f'UTC {date.hour:02d}:{date.minute:02d}:{date.second:02d}'
  second='q35 1s snap'if profile==LEGACY_PROFILE else f'{date.day:02d} {MONTHS[date.month-1]} {date.year:04d}'
  return first,second
 assert seconds==0,'unhealthy civil state retained a numeric date'
 if profile==LEGACY_PROFILE:
  rows={'Unavailable':('UTC unknown','q35 1s fail'),'Rollback':('UTC blocked','q35 1s stop'),'Exhausted':('UTC ended','src? prec?'),'Unknown':('UTC stale','q35 1s old')if stale else('UTC unknown','src? prec?')}
 else:rows={'Unavailable':('UTC unknown','Source error'),'Rollback':('UTC blocked','Time blocked'),'Exhausted':('UTC ended','Updates ended'),'Unknown':('UTC stale','Sample stale')if stale else('UTC unknown','Awaiting UTC')}
 assert state in rows,'unknown civil state';return rows[state]

def inspect_publications(trace):
 """Validate all present prefixes; a trailing staged/enqueued item may await draw."""
 assert trace.count('[boot.silkbar.clock.source] holder=6 task=6 slot=61 consumer=6 consumer_task=6 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0')==trace.count('[boot.silkbar.clock.source]')==1,'exact status-bar BOOT source absent'
 sources=[s for s in validate_clock_sources(trace)if s['pd']=='6'];samples=list(re.finditer(r'\[silkbar.civil.sample\] sequence=(\d+) state=([0-4]) seconds=(\d+) observed_seconds=(\d+) attempts=(\d+) source_known=([01]) profile=q35_cmos_utc precision=1 snapshot=1',trace));assert len(samples)==trace.count('[silkbar.civil.sample]'),'malformed civil sample'
 assert[int(s[1])for s in samples]==list(range(1,len(samples)+1)),'civil publication sequence reused/skipped';seen={};source_cursor=0;high=0;rollback=False
 for s in samples:
  seq,state,seconds,observed,attempts,known=map(int,s.groups());assert state==1 or seconds==0,'nonready civil publication retains numeric value'
  if observed:
   assert source_cursor<len(sources),'civil observation lacks raw held source';raw=sources[source_cursor];source_cursor+=1
   assert raw.start()<s.start()and int(raw['seconds'])==observed and int(raw['attempts'])==attempts and known==1,'civil source/observer join differs'
   assert source_cursor==1 or sources[source_cursor-2].start()<raw.start(),'reordered civil raw source'
  else:assert attempts==0,'failed source retains attempts'
  if state==1:assert not rollback and known==1 and seconds==observed and seconds>=high,'invalid or regressed snapshot';high=seconds
  elif state==0:assert not rollback and known==observed==0,'Unknown invents source information'
  elif state==2:assert not rollback and known==1 and observed==0,'failure returned stale source value'
  elif state==3:
   assert rollback or(known==1 and 0<observed<high),'rollback lacks lower actual sample';rollback=True
  else:assert seq==0xffffffff and observed==known==0,'exhaustion is not terminal'
  seen[seq]=dict(state=STATES[state],seconds=seconds,offset=s.start(),end=s.end())
 # A source call may be between kernel return and the next serialized sample.
 assert source_cursor<=len(sources)<=source_cursor+1,'unaccounted status-bar source reads'
 if source_cursor<len(sources)and samples:assert sources[source_cursor].start()>samples[-1].end(),'unaccounted read is not a trailing in-flight sample'
 tries=list(re.finditer(r'\[silkbar.civil.publish.try\] sequence=(\d+) state=([0-4]) seconds=(\d+) held_display=1',trace));assert len(tries)==trace.count('[silkbar.civil.publish.try]'),'malformed civil publication intent';attempted={}
 for t in tries:
  seq,state,seconds=map(int,t.groups());sample=seen.get(seq);assert sample and sample['state']==STATES[state]and sample['seconds']==seconds and sample['end']<t.start(),'publication intent differs from immutable sample';attempted.setdefault(seq,t.start())
 enqueued=list(re.finditer(r'\[silkbar.civil.enqueued\] sequence=(\d+) state=([0-4]) seconds=(\d+) held_display=1',trace));assert len(enqueued)==trace.count('[silkbar.civil.enqueued]'),'malformed civil enqueue';queued={}
 for e in enqueued:
  seq,state,seconds=map(int,e.groups());sample=seen.get(seq);assert sample and seq not in queued and sample['state']==STATES[state]and sample['seconds']==seconds and seq in attempted and attempted[seq]<e.start(),'unbound/duplicate civil enqueue';queued[seq]=dict(**sample,enqueue=e.start())
 applied=list(re.finditer(r'\[sexdisplay.clock.civil.applied\] pd=6 task=6 sequence=(\d+) state=(Unknown|Snapshot|Unavailable|Rollback|Exhausted) seconds=(\d+) high_water=(\d+) stale=0 source_known=([01]) source=(q35_cmos_utc|unknown) precision=([01]) snapshot_only=1 pixels=unconfirmed',trace));assert len(applied)==trace.count('[sexdisplay.clock.civil.applied]'),'foreign/malformed civil admission';admitted={}
 for a in applied:
  seq=int(a[1]);q=seen.get(seq);known=a[2]in('Snapshot','Unavailable','Rollback');assert q and seq not in admitted and q['state']==a[2]and q['seconds']==int(a[3])and seq in attempted and attempted[seq]<a.start(),'civil admission lacks exact held publication intent'
  assert int(a[5])==int(a[7])==int(known)and a[6]==('q35_cmos_utc'if known else'unknown'),'civil admission invents source/precision'
  admitted[seq]=dict(state=a[2],seconds=int(a[3]),offset=a.start(),high_water=int(a[4]))
 assert '[sexdisplay.clock.civil.rejected]'not in trace,'unexpected native civil rejection'
 return admitted

def repaint_metrics(trace,contract=None):
 """Join repaint decisions to real admissions; a decision is never a draw."""
 if contract is None:contract=source_contract(Path(__file__).resolve().parents[1])
 assert contract['caption_profile']in(LEGACY_PROFILE,DATE_PROFILE),'unknown repaint caption profile'
 admitted=inspect_publications(trace)
 pattern=r'\[sexdisplay.clock.civil.applied\] pd=6 task=6 sequence=(\d+) state=(Unknown|Snapshot|Unavailable|Rollback|Exhausted) seconds=(\d+) high_water=\d+ stale=0 source_known=[01] source=(?:q35_cmos_utc|unknown) precision=[01] snapshot_only=1 pixels=unconfirmed repaint=([01])(?=\n|$)'
 updates=list(re.finditer(pattern,trace));assert len(updates)==len(admitted),'missing/malformed civil repaint decision'
 stale=list(re.finditer(r'\[sexdisplay.clock.civil.stale\] sequence=(\d+) high_water=(\d+) state=Unknown seconds=0 stale=1 ticks=\d+ synthesized=0(?=\n|$)',trace))
 assert len(stale)==trace.count('[sexdisplay.clock.civil.stale]'),'malformed civil health transition'
 draws=list(re.finditer(r'\[display.civil.draw\] sequence=(\d+) state=(Unknown|Snapshot|Unavailable|Rollback|Exhausted) seconds=(\d+) stale=([01]) pixels=written path=(?:full|strip) x=\d+ y=\d+ w=\d+ h=\d+(?=\n|$)',trace))
 assert len(draws)==trace.count('[display.civil.draw]'),'malformed completed civil draw'
 # Ordinary-image startup paints Unknown. Canonical receiver health may later
 # hide Snapshot; merge those transitions before checking both renderer rows.
 state,seconds,is_stale,sequence='Unknown',0,False,0;requested=0
 def visible(s,t,old):return(s,(t if contract['caption_profile']==DATE_PROFILE else t%86400)if s=='Snapshot'else 0,old if s=='Unknown'else False)
 for _,kind,event in sorted([(m.start(),'update',m)for m in updates]+[(m.start(),'stale',m)for m in stale]+[(m.start(),'draw',m)for m in draws]):
  if kind=='draw':
   assert(int(event[1]),event[2],int(event[3]),bool(int(event[4])))==(sequence,state,seconds,is_stale),'completed draw differs from actual receiver/health state'
  elif kind=='stale':
   assert state=='Snapshot'and int(event[1])==sequence and int(event[2])==admitted[sequence]['high_water'],'unbound civil expiry'
   state,seconds,is_stale='Unknown',0,True
  else:
   seq,new_state,new_seconds,redraw=int(event[1]),event[2],int(event[3]),int(event[4])
   assert seq in admitted and admitted[seq]['offset']==event.start(),'repaint decision lacks actual admission'
   changed=visible(state,seconds,is_stale)!=visible(new_state,new_seconds,False)
   assert redraw==int(changed),'repaint decision differs from complete visible clock rows'
   state,seconds,is_stale,sequence=new_state,new_seconds,False,seq;requested+=redraw
 assert updates,'no civil admissions observed'
 return dict(admissions=len(updates),clock_repaint_requests=requested,unchanged_admissions=len(updates)-requested,completed_civil_draws=len(draws),unchanged_fraction=(len(updates)-requested)/len(updates),scope='Exact held-source admissions and both-row repaint decisions. Completed draws may also follow other dirty causes. No input-latency, CPU-utilization or wall-time speedup claim.')

def verify_pixels(trace,ppm,contract,resolution=None,expected_state=None):
 try:
  admitted=inspect_publications(trace);draws=list(re.finditer(r'\[display.civil.draw\] sequence=(\d+) state=(Unknown|Snapshot|Unavailable|Rollback|Exhausted) seconds=(\d+) stale=([01]) pixels=written path=(full|strip) x=(\d+) y=(\d+) w=(\d+) h=(\d+)',trace));assert len(draws)==trace.count('[display.civil.draw]')and draws,'missing/malformed completed civil draw';draw=draws[-1];seq=int(draw[1]);state=draw[2];seconds=int(draw[3]);stale=bool(int(draw[4]));x,y,w,h=map(int,draw.groups()[5:]);width,height,raw=ppm_rgb(ppm,min_width=1,min_height=1)
  assert resolution is None or(width,height)==tuple(resolution),'civil framebuffer resolution differs'
  d=contract['dimensions'];cx=d['CHIP_X3']-max(0,d['CHIP_X3']+d['CLOCK_W']+16-width)
  assert(x,y,w,h)==(cx,d['CHIP_Y'],d['CLOCK_W'],d['CHIP_H'])and x+w<=width and y+h<=height,'civil glyph geometry differs'
  if expected_state:assert state==expected_state and not stale,'civil presentation has not reached expected source state'
  if seq:
   a=admitted.get(seq);assert a and a['offset']<draw.start(),'draw lacks exact admitted source sequence'
   assert (state,seconds)==(a['state'],a['seconds'])if not stale else state=='Unknown'and seconds==0 and a['state']=='Snapshot','draw differs from admitted state'
  else:assert state=='Unknown'and seconds==0 and not stale,'initial clock invents civil time'
  first,second=expected_clock_lines(contract,state,seconds,stale)
  wanted=set()
  for row,text in enumerate((first,second)):
   for col,ch in enumerate(text):
    for gy,bits in enumerate(glyph(contract['font'],ch)):
     for gx in range(5):
      if bits&(1<<(4-gx)):wanted.add((col*6+gx,1+row*10+gy))
  color=contract['color'].to_bytes(3,'big');mismatches=0
  for yy in range(h):
   for xx in range(w):
    at=((y+yy)*width+x+xx)*3;mismatches+=(raw[at:at+3]==color)!=((xx,yy)in wanted)
  assert wanted and not mismatches,'civil foreground/erasure mismatches='+str(mismatches)
  return dict(ok=True,errors=[],caption_profile=contract['caption_profile'],sequence=seq,state=state,seconds=seconds,stale=stale,lines=[first,second],geometry=dict(x=x,y=y,w=w,h=h),checked_pixels=w*h,expected_ink_pixels=len(wanted),draw_offset=draw.start(),scope='Exact BOOT source/publication/admission/completed draw and full clock-cell foreground; snapshot precision1s, no interpolation or clock authentication.')
 except(AssertionError,ValueError,KeyError,IndexError,TypeError,OverflowError)as error:return dict(ok=False,errors=[str(error)])
