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

def source_contract(root):
 root=Path(root);names=['crates/silkbar-model/src/lib.rs','crates/silkbar-model/src/civil_clock.rs','crates/sex-pdx/src/clock_source.rs','kernel/src/clock_source.rs','kernel/src/init.rs','servers/silkbar/src/main.rs','servers/silkbar/src/civil_clock.rs','servers/sexdisplay/src/main.rs','scripts/silkbar_clock_gate.py','scripts/clock_source_evidence.py'];texts={n:(root/n).read_text()for n in names};model=texts[names[0]];display=texts['servers/sexdisplay/src/main.rs'];abi=texts['crates/sex-pdx/src/clock_source.rs']
 dimensions={}
 for name in('CHIP_X3','CHIP_Y','CLOCK_W','CHIP_H'):
  m=re.search(r'pub const '+name+r': usize = (\d+);',model);assert m,'missing source-bound civil geometry';dimensions[name]=int(m[1])
 color=re.search(r'pub const DEFAULT_THEME: Theme = Theme \{.*?text:\s*(0x[0-9A-Fa-f]+)',model,re.S);assert color,'missing civil foreground color'
 bounds={}
 for name in('MIN_SECONDS','MAX_SECONDS'):
  m=re.search(r'pub const '+name+r': u64 = ([\d_]+);',abi);assert m;bounds[name]=int(m[1].replace('_',''))
 assert 'pub const STATUS_BAR: Identity = Identity { pd: 6, task: 6 };'in abi
 for literal in('UTC 00:00:00','q35 1s snap','UTC unknown','src? prec?','q35 1s fail','UTC stale','q35 1s old','UTC blocked','q35 1s stop','UTC ended'):
  assert 'b"'+literal+'"'in display,'civil label changed: '+literal
 assert 'if bar.civil_enabled {return civil_clock_fg_at(x,y,bar);}'in display and 'civil_clock_drawn(bar,w,h,"strip")'in display and 'civil_clock_drawn(bar,w,h,"full")'in display
 return dict(font=font_from_source(display),dimensions=dimensions,color=int(color[1],16)&0xffffff,bounds=bounds,source_sha256={n:hashlib.sha256(t.encode()).hexdigest()for n,t in texts.items()})

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

def repaint_metrics(trace):
 """Join repaint decisions to real admissions; a decision is never a draw."""
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
 def visible(s,t,old):return(s,t%86400 if s=='Snapshot'else 0,old if s=='Unknown'else False)
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
  if state=='Snapshot':
   assert contract['bounds']['MIN_SECONDS']<=seconds<=contract['bounds']['MAX_SECONDS'];first='UTC '+datetime.datetime.fromtimestamp(seconds,datetime.timezone.utc).strftime('%H:%M:%S');second='q35 1s snap'
  elif state=='Unavailable':first,second='UTC unknown','q35 1s fail'
  elif state=='Rollback':first,second='UTC blocked','q35 1s stop'
  elif state=='Exhausted':first,second='UTC ended','src? prec?'
  elif stale:first,second='UTC stale','q35 1s old'
  else:first,second='UTC unknown','src? prec?'
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
  return dict(ok=True,errors=[],sequence=seq,state=state,seconds=seconds,stale=stale,lines=[first,second],geometry=dict(x=x,y=y,w=w,h=h),checked_pixels=w*h,expected_ink_pixels=len(wanted),draw_offset=draw.start(),scope='Exact BOOT source/publication/admission/completed draw and full clock-cell foreground; snapshot precision1s, no interpolation or clock authentication.')
 except(AssertionError,ValueError,KeyError,IndexError,TypeError,OverflowError)as error:return dict(ok=False,errors=[str(error)])
