#!/usr/bin/env python3
"""Real app HTTP intent, controlled loopback bytes, TCP capture and body pixels."""
import argparse,ast,hashlib,json,os,re,shutil,socket,struct,subprocess,threading,time
from pathlib import Path
from desktop_close_gate import QMP
from check_tcp_capture import inspect as capture,fingerprint,frames
from check_dns_capture import inspect as dns_capture,query as dns_query,response as dns_response
from check_nic_command import inspect as nic
from check_desktop_pixels import font_from_source,glyph,verify_kaleido_layout
from check_disk_persistence import ppm_rgb

DNS_MODES=('dns-disallowed','dns-zero-ttl','dns-malformed','dns-silence','dns-cancel','dns-close','dns-cname-disallowed','dns-cname-zero-ttl','dns-cname-cycle','dns-cname-conflict','dns-bad-pointer','dns-cname-unrelated')
DNS_SUCCESS=('dns-compressed','dns-cname','dns-cname-reordered','dns-cname-sections')
STREAM_MODES=('stream','stream-short','stream-reset','stream-cancel-retry','stream-close')
MODES=('echo','malformed','short-body','oversized','redirect','reset','silence','cancel-retry','close','switch','reader')+DNS_MODES+DNS_SUCCESS+STREAM_MODES
BODY=b'LIVE SEXOS HTTP BODY 80 / FROM CONTROLLED PEER.'
ECHO=bytes((n*17+78)&255 for n in range(1025))
# Whole document is independent controlled peer data, including interior blanks
# and text well beyond the64-byte overview. Last byte must remain reachable.
READER_TAIL=b'FINAL FULL DOCUMENT BYTE Z'
READER_BODY=(b''.join((b' '*48 if n%9==8 else(f'{n:03d} SEXOS READER LINE / SOURCE BYTES ARE DATA').encode().ljust(48,b'.'))+b'\n'for n in range(50)))[:2048-len(READER_TAIL)]+READER_TAIL
assert len(READER_BODY)==2048
STREAM_TAIL=b'FINAL STREAM DOCUMENT BYTE Z'
STREAM_BODY=(b''.join((b' '*48 if n%11==10 else(f'{n:03d} SEXOS STREAM LINE / SOURCE BYTES ARE DATA').encode().ljust(48,b'.'))+b'\n'for n in range(300)))[:12288-len(STREAM_TAIL)]+STREAM_TAIL
assert len(STREAM_BODY)==12288

def load_pixel_contract(root):
 paths={'renderer':'servers/sexdisplay/src/main.rs','preview':'apps/kaleidoscope/src/body_text.rs','compose':'apps/kaleidoscope/src/browser_view.rs','app':'apps/kaleidoscope/src/main.rs','style':'crates/sex-pdx/src/desktop_style.rs','surface':'servers/sexdisplay/src/surface_lifecycle.rs','wire':'crates/sex-http/src/codec.rs','wire_facade':'apps/kaleidoscope/src/http_wire.rs','wire_lib':'crates/sex-http/src/lib.rs','wire_manifest':'crates/sex-http/Cargo.toml','wire_config':'crates/sex-net-config/src/lib.rs','client':'apps/kaleidoscope/src/http_probe.rs','reader':'apps/kaleidoscope/src/document_reader.rs','network_abi':'crates/sex-pdx/src/network_job.rs','text_encoding':'crates/sex-pdx/src/text_encoding.rs'}
 sources={key:(root/path).read_text()for key,path in paths.items()}
 if not re.fullmatch(r'(?s)\s*//[^\n]*\n//! Source-path facade:[^#]*#\[path="\.\./\.\./\.\./crates/sex-http/src/codec\.rs"\]\s*mod codec;\s*pub use self::codec::\*;\s*',sources['wire_facade']):raise ValueError('production HTTP facade no longer uses the single shared codec')
 if '#![no_std]'not in sources['wire_lib']or 'pub use codec::*;'not in sources['wire_lib']or 'mod codec;'not in sources['wire_lib']:raise ValueError('shared HTTP no_std library binding changed')
 if 'sex_net_config::HOST_NAME'not in sources['wire']:raise ValueError('shared HTTP hostname no longer comes from immutable BOOT data')
 def number(key,pattern):
  match=re.search(pattern,sources[key])
  if not match:raise ValueError('production HTTP pixel contract changed: '+pattern)
  return sum(int(term.strip().replace('_',''),0)for term in match[1].split('+'))
 def literal(key,pattern):
  match=re.search(pattern,sources[key])
  if not match:raise ValueError('missing production HTTP literal: '+pattern)
  return ast.literal_eval(match[1])
 compose=sources['compose']
 echo={view:ast.literal_eval(value)for view,value in re.findall(r'network_probe::View::(\w+)=>\s*(b"[^"\n]*")',compose)}
 dns={view:ast.literal_eval(value)for view,value in re.findall(r'dns_probe::View::(\w+)=>append\([^\n]*,(b"[^"\n]*")\)',compose)}
 http={view:ast.literal_eval(value)for view,value in re.findall(r'http_probe::View::(\w+)=>\s*(b"[^"\n]*")',compose)}
 if not any(call in sources['app']for call in('self.ui.queue(&text.bytes[..text.len]','self.ui.queue_selected(&text.bytes[..text.len]'))or 'browser_view::compose('not in sources['app']:raise ValueError('Browser no longer publishes composed atomic snapshots')
 if 'self.ui.queue_selected(&text.bytes[..text.len]'in sources['app']and ('metadata,visible_selection)'not in sources['app']or 'visible_selection=f.selection'not in sources['app']):raise ValueError('Browser selected snapshot metadata binding changed')
 if 'FRAME_TOP_BAR_HEIGHT_PX + 8'not in sources['renderer']:raise ValueError('renderer text origin changed')
 if 'let dns_start=preview.len()+64'not in compose or 'let http_start=dns_start+64'not in compose or 'let body_start=http_start+64'not in compose:raise ValueError('production status extent layout changed')
 reader=sources['reader'].split('#[cfg(test)]',1)[0]
 dynamic_path=any(call in sources['app']for call in('self.reader.render_path(self.http.document(),self.http.document_path())','self.reader.render_path_selected(self.http.document(),self.http.document_path(),self.selection.range())'))
 if not(dynamic_path or 'self.reader.render(self.http.document())'in sources['app'])or 'selected:http_view_code(self.http.view())|256'not in sources['app']:raise ValueError('production full document reader binding changed')
 if dynamic_path and('host.iter().chain(b" ").chain(path)'not in reader or 'clean(*byte)'not in reader or '[browser.http.document]'not in sources['client']or 'positive_release=1'not in sources['client']):raise ValueError('production released document/path rendering contract changed')
 if not any(call in reader for call in('self.render_source(body,path,b"Plain HTTP / no TLS",HOST)','self.render_source(body,path,b"Plain HTTP / no TLS",HOST,selection)'))or 'put(&mut frame,0,header)'not in reader:raise ValueError('production plain reader source selection changed')
 if re.search(r'self\.anchor\s*=\s*info\.offset',reader)or 'pub fn source_anchor(&self)'not in reader:raise ValueError('production reader no longer retains requested source-byte anchor')
 # Bind the UTF-8 reader to the exact shared decoder and byte-preserving
 # receive path. ASCII pixel fixtures remain a strict subset of this profile.
 for token in ('use sex_pdx::text_encoding as text;','end=text::next(body,end);cells+=1;',
               'text::encode(display_scalar(scalar),&mut bytes)', 'text::REPLACEMENT',
               'let body_rows=(rows-3).min((CAPACITY-(3*cols+2))/cost);',
               'replace_row(&mut frame,shown+2,&serialized[..length])',
               'row_at(frame,row)+frame.cols-length', '.min(MAX_ROWS)',
               'text::is_boundary(body,lo)&&text::is_boundary(body,hi)'):
  if token not in reader:raise ValueError('production UTF-8 reader contract changed: '+token)
 if 'self.document[self.document_len]=byte;'not in sources['client']:raise ValueError('production HTTP body no longer retains exact received bytes')
 if 'document_reader::replace_footer(&mut text.bytes,text.len,cols,label)'not in sources['app']:raise ValueError('production reader footer no longer follows serialized byte offset')
 if number('reader',r'const MAX_ROWS: usize = (\d+)')!=number('surface',r'TEXT_MAX_ROWS: usize = (\d+)'):raise ValueError('production reader whole-frame row cap differs from Display')
 reader_header=literal('reader',r'self\.render_source\(body,path,(b"[^"\n]*"),HOST(?:,selection)?\)')
 controls=[]
 for name in ('full','compact','minimal'):
  match=re.search(r'let '+name+r': &\[u8\] = if https \{ (b"[^"\n]*") \} else \{ (b"[^"\n]*") \}',reader)
  if not match:raise ValueError('missing source-bound reader '+name+' controls')
  controls.append(tuple(ast.literal_eval(value)for value in match.groups()))
 https_header=literal('reader',r'self\.render_source\(body,path,(b"[^"\n]*"),b"probe\.sexos\.test:46080"(?:,selection)?\)')
 https_headers=[https_header,b'HTTPS / lab root',b'Lab HTTPS',b'Lab TLS']
 source_literals={ast.literal_eval(value)for value in re.findall(r'(b"[^"\n]*")',reader)}
 if any(value not in source_literals for value in https_headers[1:]):raise ValueError('missing source-bound responsive laboratory header')
 for token in ('frame.end>frame.first','range[length] = b\'-\'','range[length] = b\'/\'','controls.len()+1+length<=frame.cols'):
  if token not in reader:raise ValueError('production finite range/footer contract changed')
 https_reader_footers=[controls[0][0],b'V I',b'I'];reader_footers=[controls[0][1],b'V back',b'V']
 return dict(reader_encoding='utf-8-scalars',reader_max_rows=number('reader',r'const MAX_ROWS: usize = (\d+)'),reader_tab_cells=1,reader_replacement=0xfffd,reader_max_columns=number('reader',r'pub const MAX_COLUMNS: usize = (\d+)'),reader_controls=controls,reader_https_headers=https_headers,wire_max_body=number('wire',r'pub const MAX_BODY: usize = (\d+)'),wire_max_headers=number('wire',r'pub const MAX_HEADERS: usize = (\d+)'),stream_ring=number('network_abi',r'pub const STREAM_RING_BYTES: usize = (\d+)'),stream_max_wire=number('network_abi',r'pub const MAX_STREAM_BYTES: usize = ([0-9_ +]+);'),stream_total_ticks=number('network_abi',r'pub const STREAM_TOTAL_TICKS: u64 = ([0-9_]+)'),stream_idle_ticks=number('network_abi',r'pub const STREAM_IDLE_TICKS: u64 = ([0-9_]+)'),dynamic_reader_path=dynamic_path,reader_max_path=number('wire',r'pub const MAX_PATH: usize = (\d+)')if dynamic_path else 48,reader_max_ticket=number('network_abi',r'pub const MAX_TICKET: u32 = ([0-9_]+)'),reader_header=reader_header,reader_footers=reader_footers,https_reader_footers=https_reader_footers,reader_path=literal('reader',r'const PATH: &\[u8\] = (b"[^"\n]*")'),reader_capacity=number('reader',r'const CAPACITY: usize = (\d+)'),reader_max_body=number('reader',r'const MAX_BODY: usize = (\d+)'),glyph_height=number('reader',r'const GLYPH_HEIGHT: u32 = (\d+)'),font=font_from_source(sources['renderer']),prefix=literal('preview',r'preview\.append\((b"[^"\n]*")\)'),echo=echo,dns=dns,http=http,
  dns_prefix=literal('compose',r'append\([^;\n]*,(b"\\n\\nControlled DNS: ")\)'),dns_footer=literal('compose',r'append\([^;\n]*,(b"\\nD query  X cancel")\)'),
  http_prefix=literal('compose',r'append\([^;\n]*,(b"\\n\\nPlain HTTP / no TLS: ")\)'),http_footer=literal('compose',r'append\([^;\n]*,(b"\\nH fetch[^"\n]*")\)'),
  inset=number('style',r'INSET: u32 = (\d+)'),advance=number('style',r'ADVANCE: u32 = (\d+)'),line=number('style',r'LINE: u32 = (\d+)'),scale_threshold=number('style',r'width >= (\d+)'),top=number('renderer',r'FRAME_TOP_BAR_HEIGHT_PX: usize = (\d+)')+8,max_rows=number('surface',r'TEXT_MAX_ROWS: usize = (\d+)'),capacity=number('surface',r'TEXT_BUF_CAP: usize = (\d+)'),
  request=literal('wire',r'const REQUEST: &\[u8\] = (b"[^"\n]*")'),color=number('app',r'self\.ui\.queue(?:_selected)?\(&text\.bytes\[\.\.text\.len\],(0x[0-9A-Fa-f]+)'),
  source_sha256={paths[key]:hashlib.sha256(value.encode()).hexdigest()for key,value in sources.items()})

WHEEL_SCOPE='Actual QMP USB-tablet relative wheel bytes through BOOT SexUSB5/task5, Input4/task4, exact live Shell Browser15/task15 route43 and retained bounded reader. Three visual rows/notch, whole reader glyphs and endpoint clamps; hover/title/modal/closed rejection, finite burst with actual report coalescing observed, no navigation refetch. No physical hardware, arbitrary HID descriptors, queue-saturation or hostile-input isolation claim.'
WHEEL_SERIAL_UNITS='UTF-8 decoded characters; universal newlines; replacement on decode error'

def serial_text(path):
 # Capture offsets and prefix hashes use text characters, never raw byte offsets.
 # Keep the whole original serial file separately hashed in raw_sha256.
 return Path(path).read_text(encoding='utf-8',errors='replace')

WHEEL_PATTERNS={
 'device':r'\[sexusb.wheel.device\] slot=(\d+) bytes=(\d+) absolute=([01]) delta=(-?\d+) packed=(\d+) up_positive=1',
 'received':r'\[sexinput.wheel.received\] caller=(\d+) task=(\d+) absolute=([01]) delta=(-?\d+) packed=(\d+) up_positive=1',
 'normalized':r'\[sexinput.wheel.normalized\] delta=(-?\d+) word=(\d+) reserved=(\d+) class=(\d+) admitted=1 up_positive=1',
 'queued':r'\[shell.browser.wheel.queued\] pd=(\d+) task=(\d+) route=(\d+) wheel=(-?\d+) x=(-?\d+) y=(-?\d+) accepted=1 capacity=32',
 'delivered':r'\[shell.browser.wheel.delivered\] pd=(\d+) task=(\d+) route=(\d+) wheel=(-?\d+) accepted=1 status=0',
 'reader':r'\[browser.reader.wheel\] wheel=(-?\d+) before=(\d+) anchor=(\d+) changed=([01]) viewport=(\d+) retained_body_only=1',
 'rejected':r'\[shell.browser.wheel.rejected\] reason=target wheel=(-?\d+) x=(-?\d+) y=(-?\d+) effects=0'}
WHEEL_PATTERNS={kind:pattern+r'(?=\r?\n|$)'for kind,pattern in WHEEL_PATTERNS.items()}
WHEEL_TAGS={'device':'sexusb.wheel.device','received':'sexinput.wheel.received','normalized':'sexinput.wheel.normalized','queued':'shell.browser.wheel.queued','delivered':'shell.browser.wheel.delivered','reader':'browser.reader.wheel','rejected':'shell.browser.wheel.rejected'}
WHEEL_PENDING_PLAN=[('pending-burst',-3,'pending'),('closed',-1,'closed')]
WHEEL_PLAN=[('top',1,'body'),('down',-1,'body'),('up',1,'body'),('burst',-3,'body'),('end',-1,'body'),('outside',-1,'outside'),('title',-1,'title'),('modal',-1,'modal'),('closed',-1,'closed')]

def load_wheel_contract(root):
 root=Path(root);contract=load_pixel_contract(root)
 files=('servers/sexusb/src/main.rs','servers/sexinput/src/main.rs','servers/silk-shell/src/main.rs','servers/silk-shell/src/app_input.rs','crates/sex-pdx/src/app_scroll.rs','crates/sex-pdx/src/lib.rs')
 source={name:(root/name).read_text()for name in files}
 requirements={files[0]:('wheel: if len >= 6 { buf[5] as i8 } else { 0 }','((report.wheel as u8 as u64) << 40)','record_device_wheel(en_slot_id, intr_actual, true, td.wheel, packed_axes)'),files[1]:('req.caller_pd == 5 && req.caller_task == 5','emit(scroll.wire(), 0, EV_WHEEL)','0x0000_ff01_ffff_ffff'),files[2]:('COMMAND_PALETTE_OPEN || ATLAS_MODE_ENABLED','if sid!=300{return None}','px<x as i64+8 || py<y as i64+36','item.destination.route,OP_HID_EVENT,scroll.wire(),0,EV_WHEEL','msg.caller_task!=sex_pdx::shell_ready::INPUT_PD_ID'),files[4]:('CLASS: u64 = 5','reserved != 0 || class != CLASS','event.wire() == word')}
 for name,tokens in requirements.items():
  for token in tokens:
   if token not in source[name]:raise ValueError('wheel source contract changed: '+token)
 reader=(root/'apps/kaleidoscope/src/document_reader.rs').read_text();app=(root/'apps/kaleidoscope/src/main.rs').read_text()
 if 'let delta = -(wheel as i16) * 3;'not in reader or 'req.type_id==0x202&&req.caller_pd==3&&req.caller_task==3'not in app or 'if !self.reading || self.asset_panel || self.import_panel { return; }'not in app:raise ValueError('actual bounded reader/exact Shell wheel guard changed')
 from launcher_workflow_gate import source_contract as launcher_contract
 contract['launcher']=launcher_contract(root);contract['source_sha256'].update(contract['launcher']['source_sha256']);contract['source_sha256'].update({name:hashlib.sha256(value.encode()).hexdigest()for name,value in source.items()});contract['reader_require_position']=True
 return contract

def wheel_move(frame,wheel):
 if type(wheel)is not int or not -128<=wheel<=127 or wheel==0:raise ValueError('wheel must be canonical nonzero signed byte')
 last=max(0,frame['total_lines']-frame['body_rows']);first=max(0,min(last,frame['first']-wheel*3))
 return(frame['starts'][first]if frame['starts']else 0,first!=frame['first'])

def wheel_rows(trace,kind):
 matches=list(re.finditer(WHEEL_PATTERNS[kind],trace))
 if len(matches)!=trace.count('['+WHEEL_TAGS[kind]+']'):raise ValueError('malformed/foreign wheel '+kind+' receipt')
 return matches

def wheel_viewport(trace):
 notices=list(re.finditer(r'\[browser.viewport\] target=15 task=15 sequence=(\d+) w=(\d+) h=(\d+) origin=held_shell',trace))
 if not notices:raise ValueError('wheel Browser held viewport missing')
 viewport,w,h=map(int,notices[-1].groups());matches=re.findall(rf'\[shell.document.viewport\] target=15 task=15 surface=300 sequence={viewport} x=(-?\d+) y=(-?\d+) w={w} h={h} enqueued=1',trace)
 if len(matches)!=1:raise ValueError('wheel viewport lacks exact queued geometry')
 x,y=map(int,matches[0]);return dict(x=x,y=y,w=w,h=h),viewport

def wheel_events(wheel):
 if type(wheel)is not int or not 1<=abs(wheel)<=3:raise ValueError('finite real wheel burst must be1..3notches')
 return[{'type':'btn','data':{'down':down,'button':'wheel-up'if wheel>0 else'wheel-down'}}for _ in range(abs(wheel))for down in(True,False)]

def inspect_wheel_action(trace,action,body,contract,resolution):
 start=action['serial_start'];end=action['serial_end'];returned=action['serial_qmp_return'];pointer=action['pointer_start']
 if any(type(n)is not int for n in(start,end,returned,pointer))or not 0<=pointer<=start<=returned<=end<=len(trace):raise ValueError('wheel QMP/pointer window out of bounds')
 if any(type(action.get(k))not in(int,float)for k in('host_start','host_qmp_return','host_end'))or not action['host_start']<=action['host_qmp_return']<=action['host_end']:raise ValueError('wheel QMP return clock window reversed')
 wheel=action['wheel'];target=action['target'];x=action['x'];y=action['y'];before=action['before']
 if any(type(n)is not int for n in(x,y,before,action['after']))or not 0<=before<=len(body):raise ValueError('wheel pointer/anchor metadata not bounded integers')
 if type(wheel)is not int or not -127<=wheel<=127 or not wheel or target not in('body','pending','outside','title','modal','closed'):raise ValueError('invalid explicit wheel intent')
 if action['transport']!='input-send-event'or action['events']!=wheel_events(wheel):raise ValueError('actual QMP wheel button request differs')
 if not 0<=x<resolution[0]or not 0<=y<resolution[1]:raise ValueError('wheel pointer outside framebuffer')
 rx=round(x*32767/(resolution[0]-1));ry=round(y*32767/(resolution[1]-1));sx=rx*(resolution[0]-1)//32767;sy=ry*(resolution[1]-1)//32767
 normalization=rf'\[shell.abs.normalize\] raw_x={rx} raw_y={ry} sx={sx} sy={sy} accepted=(?:1 reason=ok|0 reason=duplicate_sample)'
 if not re.search(normalization,trace[pointer:start]):raise ValueError('wheel lacks actual preceding normalized tablet position')
 segment=trace[start:end];rows={kind:wheel_rows(segment,kind)for kind in WHEEL_PATTERNS};device=rows['device'];received=rows['received'];normalized=rows['normalized']
 if not device or not len(device)==len(received)==len(normalized)or len(device)>abs(wheel):raise ValueError('wheel device/Input report custody missing/extra')
 values=[int(m[4])for m in device]
 if sum(values)!=wheel or any(v==0 or(v>0)!=(wheel>0)for v in values):raise ValueError('real USB wheel report sum/direction differs from QMP intent')
 if target!='closed':
  tile,viewport=wheel_viewport(trace[:start]);frame=None if target=='pending'else reader_geometry(trace[:start],body,contract,before);inside=tile['x']+8<=sx<tile['x']+tile['w']-8 and tile['y']+36<=sy<tile['y']+tile['h']-8
  if frame is not None:
   positions=list(re.finditer(r'\[browser.reader.position\] anchor=(\d+) viewport=(\d+) tls=0 retained_body_only=1',trace[:start]))
   if not positions or tuple(map(int,positions[-1].groups()))!=(before,viewport):raise ValueError('wheel before-anchor differs from actual preceding reader position')
  if target in('body','pending','modal')and not inside or target in('outside','title')and inside:raise ValueError('wheel target classification differs from actual Browser body')
  if target=='title'and not(tile['x']<=sx<tile['x']+tile['w']and tile['y']<=sy<tile['y']+36):raise ValueError('title rejection not inside actual frame chrome')
  if target=='modal':
   from check_launcher_pixels import layout
   modal,_=layout(trace[:start])
   if not(modal['x']+8<=sx<modal['x']+modal['w']-8 and modal['y']+36<=sy<modal['y']+modal['h']-8)or trace[:start].rfind('[command_palette.close]')>trace[:start].rfind('[launcher.text.view]'):raise ValueError('modal wheel lacks actual overlapping open launcher')
 else:
  if not re.search(r'\[sext.app.pool.ready\] target=15 task=15 .*ok=1',trace[:start]):raise ValueError('closed rejection precedes actual terminal cleanup')
  frame=None;viewport=None
 if target in('body','pending'):
  if rows['rejected']or not len(rows['queued'])==len(rows['delivered'])==len(device)or len(rows['reader'])!=(len(device)if target=='body'else 0):raise ValueError('body/pending wheel did not preserve exact queued/delivered/App event policy')
  if target=='pending'and ('[browser.reader.layout]'in segment or '[browser.reader.wheel]'in segment or not re.search(r'\[net.job.stream.drain\] owner=15 task=15 ticket=2 [^\n]*state_before=Receiving',trace[:start])):raise ValueError('pending wheel lacked actual Receiving or affected absent reader')
 else:
  if rows['queued']or rows['delivered']or rows['reader']or len(rows['rejected'])!=len(device):raise ValueError('outside/modal/closed wheel affected Browser')
 anchor=before;events=[]
 for i,(d,r,n,v)in enumerate(zip(device,received,normalized,values)):
  expected_packed=rx|(ry<<16)|(1<<32)|((v&255)<<40)
  if not 1<=int(d[1])<=255 or tuple(map(int,d.groups()[1:]))!=(6,1,v,expected_packed)or tuple(map(int,r.groups()))!=(5,5,1,v,expected_packed)or tuple(map(int,n.groups()))!=(v,v&0xffffffffffffffff,0,5):raise ValueError('wheel raw6-byte/packed/sign/exactUSBsender/class differs')
  if not d.start()<r.start()<n.start():raise ValueError('wheel device/Input pre-forward order differs')
  if target in('body','pending'):
   q=rows['queued'][i];delivered=rows['delivered'][i]
   if tuple(map(int,q.groups()))!=(15,15,43,v,sx,sy)or tuple(map(int,delivered.groups()))!=(15,15,43,v):raise ValueError('wheel queue changed exact live catalog target/route/coordinates')
   if not r.start()<q.start()<delivered.start():raise ValueError('wheel held route queue/return order differs')
   if target=='pending':continue
   app=rows['reader'][i];current=reader_frame(body,tile['w'],tile['h'],contract,anchor);after,changed=wheel_move(current,v)
   if tuple(map(int,app.groups()))!=(v,anchor,after,int(changed),viewport)or not r.start()<q.start()<app.start()or not q.start()<delivered.start():raise ValueError('wheel App anchor arithmetic/queued custody/order differs')
   anchor=after;events.append(dict(wheel=v,before=int(app[2]),anchor=anchor,changed=changed,callee_before_shell_return=app.start()<delivered.start(),callee_before_input_return=app.start()<n.start()))
  else:
   rejected=rows['rejected'][i]
   if tuple(map(int,rejected.groups()))!=(v,sx,sy)or rejected.start()<r.start():raise ValueError('wheel rejection coordinates/direction/order differ')
 if action['after']!=anchor:raise ValueError('declared wheel action result differs from independent source-byte model')
 return dict(ok=True,name=action['name'],target=target,wheel=wheel,reports=len(device),before=before,anchor=anchor,events=events)

def inspect_wheel(trace,actions,body,contract,resolution,pending=False):
 if [(a['name'],a['wheel'],a['target'])for a in actions]!=(WHEEL_PENDING_PLAN if pending else WHEEL_PLAN):raise ValueError('finite wheel action plan/order differs')
 checks=[];previous=0
 for action in actions:
  if action['pointer_start']<previous:raise ValueError('wheel action windows overlap/reorder')
  checks.append(inspect_wheel_action(trace,action,body,contract,resolution));previous=action['serial_end']
 if not pending:
  if actions[0]['before']!=0:raise ValueError('wheel Home clamp did not start at source zero')
  for index in(1,2,3,5,6,7):
   if actions[index]['before']!=actions[index-1]['after']:raise ValueError('wheel actions lost retained source continuity')
  end_action=actions[4];tile,_=wheel_viewport(trace[:end_action['serial_start']]);last=reader_frame(body,tile['w'],tile['h'],contract,0);wanted=reader_move(last,'end')
  if end_action['before']!=wanted or not re.search(r'\[shell.browser.input\] code=107 down=1 slot=43 accepted=1',trace[actions[3]['serial_end']:end_action['pointer_start']]):raise ValueError('bottom wheel clamp lacks actual End and independent final source row')
 for kind in WHEEL_PATTERNS:
  if len(wheel_rows(trace,kind))!=sum(len(wheel_rows(trace[a['serial_start']:a['serial_end']],kind))for a in actions):raise ValueError('wheel receipt outside actual QMP windows: '+kind)
 suffix=trace[actions[0]['pointer_start']:]
 if re.search(r'\[net\.dns\.egress\.submit\]|\[net\.job\.resolved\.admitted\]|\[browser.http.body\]|\[browser.dns.answer\]',suffix):raise ValueError('wheel/navigation refetched or emitted new owned source work')
 cleanup=trace.find('[net.app.cleanup.ack] target=15 task=15',actions[0]['pointer_start'])
 for m in re.finditer(r'\[net.job.egress.submit\] ticket=(\d+)',suffix):
  at=actions[0]['pointer_start']+m.start()
  if not pending or int(m[1])!=2 or cleanup>=0 and at>cleanup:raise ValueError('wheel emitted unowned/new/retired network work')
 if pending and re.search(r'\[browser.http.body\]|\[browser.reader.layout\]',trace):raise ValueError('partial stream became a published reader')
 return dict(ok=True,actions=checks,device_reports=sum(c['reports']for c in checks),scope=WHEEL_SCOPE)

def wheel_frame_phases(actions,frames,pending=False):
 by_name={f['screenshot'][:-4]:f for f in frames}
 if len(by_name)!=len(frames):raise ValueError('duplicate wheel screenshot identity')
 for index,action in enumerate(actions):
  name='browser-wheel-'+action['name']
  if action['name']=='end':name='browser-wheel-end-clamp'
  if action['name']=='pending-burst':name='browser-wheel-pending'
  if action['name']=='closed'and pending:name='browser-wheel-after-close'
  frame=by_name.get(name)
  boundary=actions[index+1]['pointer_start']if index+1<len(actions)else None
  if frame is None or frame['serial_end']<action['serial_end']or boundary is not None and frame['serial_end']>boundary:raise ValueError('wheel screenshot outside actual action/capture phase')
  if action['target']in('body','outside','title')and frame.get('requested_anchor')!=action['after']:raise ValueError('wheel pixels use an old/wrong source anchor')
 if not pending:
  modal=next(a for a in actions if a['name']=='modal');erased=by_name.get('browser-wheel-modal-erased');initial=by_name.get('browser-reader-initial');end=by_name.get('browser-wheel-end-entry');end_action=next(a for a in actions if a['name']=='end')
  if not erased or erased['requested_anchor']!=modal['after']or erased['serial_end']<modal['serial_end']or not erased.get('launcher_erasure'):raise ValueError('modal restoration lacks unchanged reader/launcher erasure')
  if not initial or initial['requested_anchor']!=0 or initial['serial_end']>actions[0]['pointer_start']:raise ValueError('wheel initial paint not before first real input')
  if not end or end['requested_anchor']!=end_action['before']or end['serial_end']>end_action['pointer_start']:raise ValueError('End entry paint not before bottom clamp')
 return True

def verify_wheel_evidence(root,iso,binding,out):
 from network_contention_gate import frozen_binding,cleanup_receipts
 from launcher_workflow_gate import launcher_erased
 from check_launcher_pixels import verify as launcher_pixels
 from https_asset_gate import verify_parking
 root=Path(root).resolve(strict=True);out=Path(out).resolve(strict=True);iso=Path(iso).resolve(strict=True);binding=Path(binding).resolve(strict=True);bound=frozen_binding(root,iso,binding);contract=load_wheel_contract(root);manifest=json.loads((out/'manifest.json').read_text());evidence=json.loads((out/'wheel-evidence.json').read_text())
 if manifest.get('wheel')is not True or manifest.get('binding')!=bound or manifest['mode']not in('reader','stream','stream-close')or manifest['pixel_source_sha256']!=contract['source_sha256']:raise ValueError('wheel frozen image/source/mode binding differs')
 command=manifest['command']
 if command[command.index('-cdrom')+1]!=str(Path(iso).resolve())or command.count('usb-tablet,bus=xhci.0')!=1 or 'nec-usb-xhci,id=xhci'not in command:raise ValueError('actual QMP USB-tablet device path missing')
 required={'serial.log','wire.pcap','endpoint-exchanges.json','dns-endpoint-exchanges.json'}|{f['screenshot']for f in evidence['reader_frames']+evidence['status_frames']+evidence['other_frames']}
 if set(evidence['raw_sha256'])!=required:raise ValueError('wheel raw hash closure differs')
 for name,wanted in evidence['raw_sha256'].items():
  path=(out/name).resolve()
  if not path.is_relative_to(out.resolve())or hashlib.sha256(path.read_bytes()).hexdigest()!=wanted:raise ValueError('wheel raw file differs: '+name)
 if evidence.get('serial_offset_units',WHEEL_SERIAL_UNITS)!=WHEEL_SERIAL_UNITS or evidence.get('serial_prefix_hash_encoding','UTF-8')!='UTF-8':raise ValueError('wheel serial offset/hash convention differs')
 trace=serial_text(out/'serial.log');resolution=tuple(map(int,manifest['resolution'].split('x')));body=STREAM_BODY if manifest['mode']in STREAM_MODES else READER_BODY
 checks=inspect_wheel(trace,evidence['actions'],body,contract,resolution,manifest['mode']=='stream-close');pixels=[]
 required_reader=[]if manifest['mode']=='stream-close'else ['browser-reader-initial','browser-wheel-top','browser-wheel-down','browser-wheel-up','browser-wheel-burst','browser-wheel-end-entry','browser-wheel-end-clamp','browser-wheel-outside','browser-wheel-title','browser-wheel-modal-erased','browser-wheel-home','browser-reader-pgdn','browser-reader-down','browser-reader-pgup','browser-reader-home','browser-reader-end','browser-reader-up','browser-reader-expanded','browser-reader-restored','browser-reader-resumed']
 required_status=['browser-wheel-pending']if manifest['mode']=='stream-close'else['browser-http','browser-reader-overview','browser-reader-erased','browser-reader-cannot-reopen']
 required_other=['browser-closed','browser-wheel-after-close']if manifest['mode']=='stream-close'else['browser-wheel-modal','browser-wheel-closed']
 for group,names in [('reader_frames',required_reader),('status_frames',required_status),('other_frames',required_other)]:
  if [r['screenshot']for r in evidence[group]]!=[name+'.ppm'for name in names]:raise ValueError('missing/reordered finite wheel frame set: '+group)
 expected=evidence['reader_frames']+evidence['status_frames']+evidence['other_frames'];parked=evidence['parked_frames']
 wheel_frame_phases(evidence['actions'],expected,manifest['mode']=='stream-close')
 if len(parked)!=len(expected)or len({r['screenshot']for r in parked})!=len(expected)or any(r not in expected for r in parked):raise ValueError('parked frame set differs from complete pixel capture set')
 if manifest['http_request_hex']!=contract['request'].hex()or manifest['http_body_hex']!=body.hex():raise ValueError('wheel source-defined HTTP fixture differs')
 for frame in evidence['reader_frames']+evidence['status_frames']+evidence['other_frames']:
  prefix=frame_prefix(trace,frame,frame in evidence['reader_frames']);image=(out/frame['screenshot']).read_bytes()
  if hashlib.sha256(image).hexdigest()!=frame['screenshot_sha256']:raise ValueError('wheel screenshot SHA differs')
  if frame in evidence['reader_frames']:
   check=verify_reader_pixels(prefix,image,body,contract,frame['requested_anchor'],resolution)
   if not check['ok']:raise ValueError(str(check['errors']))
   if frame.get('launcher_erasure'):launcher_erased(prefix,image,contract['launcher'],resolution)
  elif frame in evidence['status_frames']:
   check=verify_http_pixels(prefix,image,frame['expected_view'],overview_preview(body),contract,resolution)
   if not check['ok']:raise ValueError(str(check['errors']))
  elif frame['kind']=='launcher':
   errors=launcher_pixels(prefix,image,contract['font'],resolution)
   if errors:raise ValueError(str(errors))
  else:
   width,height,_=ppm_rgb(image,min_width=1,min_height=1)
   if(width,height)!=resolution:raise ValueError('closed framebuffer resolution differs')
   check=cleanup_receipts(prefix,15,15,evidence['close_start'])
  pixels.append(dict(name=frame['screenshot'],ok=True))
 park_frames=[dict(name=f['screenshot'][:-4],serial_end=f['serial_end'])for f in evidence['parked_frames']]
 verify_parking(trace,park_frames,evidence['parking'],resolution)
 peer=json.loads((out/'endpoint-exchanges.json').read_text());dns_peer=json.loads((out/'dns-endpoint-exchanges.json').read_text());rows=peer['connections'];queries=dns_peer['queries']
 if len(rows)!=2 or len(queries)!=1 or rows[0]['request_hex']!=ECHO.hex()or rows[0]['response_hex']!=ECHO.hex()or rows[1]['request_hex']!=contract['request'].hex()or rows[1]['response_hex']!=wire_response('stream'if manifest['mode']=='stream-close'else manifest['mode'],body).hex()or rows[0]['aborted']or rows[1]['aborted']!=(manifest['mode']=='stream-close'):raise ValueError('wheel introduced extra/changed controlled exchanges')
 raw=(out/'wire.pcap').read_bytes();wire=capture(raw,[dict(payload_hex=r['request_hex'],response_hex=r['response_hex'],**({'aborted':True,'allow_empty_reply':True}if r['aborted']else{}))for r in rows],guest_ports=range(48001,65536),initiator='guest');dns=dns_capture(raw,'echo',dns_peer['response_send_attempts'],expected=[dict(ticket=r['ticket'],response_hex=r['response_hex'])for r in queries]);joins=resolution_joins(trace,raw,[(2,1)],dns);stream=stream_receipts(trace,[2],contract,[2]if manifest['mode']=='stream'else[],[2]if manifest['mode']in STREAM_MODES else[])
 if manifest['mode']=='stream-close':
  planned=wire_response('stream',body);end=planned.index(b'\r\n\r\n')+4+2048
  if rows[1]['prefix_bytes']!=end or rows[1]['host_sent_prefix_hex']!=planned[:end].hex()or not 1024<=wire['connections'][1]['incoming_bytes']<=end:raise ValueError('pending wheel lacks actual bounded Receiving wire prefix')
 if not all(r['ok']for r in(wire,dns,joins,stream,nic(trace,1,True,True))):raise ValueError('wheel fixed wire/DNS/admission/stream/authority regression')
 return dict(ok=True,binding=bound,serial_offset_units=WHEEL_SERIAL_UNITS,serial_prefix_hash_encoding='UTF-8',wheel=checks,pixels=pixels,wire=wire,dns=dns,streaming=stream,scope=WHEEL_SCOPE)

def overview_preview(body):
 """Independent overview contract: printable words,64 cells, ellipsis on loss."""
 words=bytes(32 if b<=32 or b==127 else b if b<127 else 63 for b in body).split()
 text=b' '.join(words)
 return (text[:61]+b'...'if len(text)>64 else text.ljust(64,b' '))

def reader_path(path,contract):
 """Independent canonical origin-form data check; never URL authority."""
 if not isinstance(path,bytes)or not 1<=len(path)<=contract['reader_max_path']or not path.startswith(b'/')or b'//'in path or any(segment in(b'.',b'..')for segment in path.split(b'/'))or any(not(48<=byte<=57 or 65<=byte<=90 or 97<=byte<=122 or byte in b'/.-_~')for byte in path):raise ValueError('invalid expected released document path')
 return path

def reader_frame(body,width,height,contract,anchor=0,expected_path=None):
 """Independent ASCII fixture model; UTF-8 native profiles use their own oracle."""
 if any(byte>=127 or byte<32 and byte not in(9,10)for byte in body):raise ValueError('ASCII reader oracle requires UTF-8 profile for non-ASCII or unsupported controls')
 if len(body)>contract['reader_max_body']:raise ValueError('expected document exceeds production body bound')
 if type(anchor)is not int or not 0<=anchor<=contract['reader_max_body']:raise ValueError('requested source anchor outside bounded body extent')
 factor=2 if width>=contract['scale_threshold']else 1
 cols=min(contract['reader_max_columns'],max(0,(width-2*contract['inset'])//(contract['advance']*factor)-1))
 available=height-contract['top']-contract['glyph_height']*factor
 rows=min(contract['max_rows'],contract['reader_capacity']//(cols+1),1+available//(contract['line']*factor))if cols and available>=0 else 0
 if rows<4:raise ValueError('actual reader tile has no fully visible body row')
 sanitized=bytes(b if b==10 or 32<=b<=126 else 32 if b<128 else 63 for b in body).rstrip(b' \n')
 lines=[];starts=[];source=0
 if sanitized:
  for paragraph in sanitized.split(b'\n'):
   chunks=[paragraph[n:n+cols]for n in range(0,len(paragraph),cols)]or[b'']
   for n,chunk in enumerate(chunks):starts.append(source+n*cols);lines.append(chunk)
   source+=len(paragraph)+1
 visible=rows-3;last=max(0,len(lines)-visible)
 anchored=max((i for i,start in enumerate(starts)if start<=anchor),default=0);first=min(anchored,last)
 host=re.search(br'\r\nHost: ([^\r\n]+)\r\n',contract['request'])
 if not host:raise ValueError('missing source-bound HTTP hostname')
 shown=lines[first:first+visible]
 https=contract['reader_header']==contract['reader_https_headers'][0]
 header=contract['reader_https_headers'][0 if cols>=26 else 1 if cols>=16 else 2 if cols>=9 else 3]if https else contract['reader_header']
 original=contract['reader_footers'][0 if cols>=25 else 1 if cols>=6 else 2]
 progress=f'{first+1 if shown else 0}-{first+len(shown)}/{len(lines)}'.encode('ascii')
 choices=[pair[0 if https else 1]for pair in contract['reader_controls']]
 controls=next((hint for hint in choices if len(hint)+1+len(progress)<=cols),None)
 footer=controls.ljust(cols-len(progress),b' ')+progress if controls is not None else original
 path=reader_path(contract['reader_path'][1:]if expected_path is None else expected_path,contract)
 output=[header,host[1]+b' '+path]+shown+[b'']*(visible-len(shown))+[footer]
 output=[line[:cols].ljust(cols,b' ')for line in output]
 return dict(path=path.decode('ascii'),cols=cols,rows=rows,body_rows=visible,first=first,end=first+len(shown),total_lines=len(lines),source_bytes=len(body),scale=factor,anchor=anchor,source_anchor=anchor,first_byte=starts[first]if starts else 0,progress=progress.decode('ascii')if controls is not None else None,starts=starts,lines=output,bytes=rows*(cols+1)-1)

def reader_position(trace,layout_end,frame,tls,required=False):
 """Optional legacy fixture receipt; mandatory in fresh native100 probes."""
 # A bounded producer record can follow another producer's unfinished legacy
 # fragment. Match its exact token/fields and newline without reconstructing
 # any split fields or assuming all other log writers are atomic.
 tail=trace[layout_end:];records=list(re.finditer(r'\[browser.reader.position\][^\r\n]*',tail))
 if not records:
  if required:raise ValueError('missing retained source-byte position receipt')
  return None
 if len(records)!=1:raise ValueError('duplicate/foreign current reader position receipt')
 record=records[0]
 if not re.match(r'\r?\n',tail[record.end():]):raise ValueError('unterminated retained reader position receipt')
 match=re.fullmatch(r'\[browser.reader.position\] anchor=(\d+) viewport=(\d+) tls=(0|1) retained_body_only=1',record[0])
 if not match or tuple(map(int,match.groups()))!=(frame['source_anchor'],frame['viewport'],tls):raise ValueError('retained reader position differs from source/viewport/origin')
 applied=re.search(r'\[browser.view.applied\]',tail)
 if not applied or record.end()>=applied.start():raise ValueError('reader position not before actual current Applied')
 return dict(anchor=int(match[1]),viewport=int(match[2]),tls=int(match[3]))

def frame_prefix(trace,frame,reader=False):
 """Replay the exact textual serial snapshot used for one verified capture."""
 end=frame.get('serial_end');wanted=frame.get('serial_sha256')
 if type(end)is not int or not 0<end<=len(trace)or not isinstance(wanted,str)or not re.fullmatch(r'[0-9a-f]{64}',wanted):raise ValueError('missing/invalid immutable frame serial boundary')
 prefix=trace[:end]
 if hashlib.sha256(prefix.encode()).hexdigest()!=wanted:raise ValueError('immutable frame serial prefix changed')
 if 'requested_anchor'not in frame:raise ValueError('missing explicit frame anchor/origin')
 anchor=frame['requested_anchor']
 if reader and(type(anchor)is not int or not 0<=anchor<=12288)or not reader and anchor is not None:raise ValueError('invalid stored reader/status anchor')
 return prefix

def reader_move(frame,action):
 first=frame['first'];last=max(0,frame['total_lines']-frame['body_rows'])
 target={'up':max(0,first-1),'down':min(last,first+1),'pgup':max(0,first-frame['body_rows']),'pgdn':min(last,first+frame['body_rows']),'home':0,'end':last}[action]
 return frame['starts'][target]if frame['starts']else 0

def released_reader_document(trace,body,contract,expected_path=None):
 """Bind current presentation data to the latest exact positive job release."""
 releases=list(re.finditer(r'^\[browser.http.released\][^\r\n]*',trace,re.M))
 documents=list(re.finditer(r'^\[browser.http.document\][^\r\n]*',trace,re.M))
 if not releases or not documents:raise ValueError('missing positive released document/path receipt')
 release=re.fullmatch(r'\[browser.http.released\] ticket=(\d+) view=Ready private_job=0',releases[-1][0])
 document=re.fullmatch(r'\[browser.http.document\] ticket=(\d+) path=(\S+) status=200 private_bytes=(\d+) positive_release=1',documents[-1][0])
 if not release or not document:raise ValueError('latest document lacks exact positive Ready release/path receipt')
 ticket=int(release[1])
 if not 1<=ticket<=contract['reader_max_ticket']or int(document[1])!=ticket or documents[-1].start()<=releases[-1].end():raise ValueError('released document path belongs to an old/unreleased ticket')
 if int(document[3])!=len(body):raise ValueError('released document byte extent differs from independent body')
 views=list(re.finditer(r'^\[browser.http.view\] state=(\w+) tls=0',trace,re.M))
 if views and views[-1].start()>documents[-1].end()and views[-1][1]!='Ready':raise ValueError('released document superseded by current non-Ready HTTP view')
 path=reader_path(document[2].encode('ascii'),contract)
 if expected_path is not None and path!=reader_path(expected_path,contract):raise ValueError('released document path differs from independent expected path')
 layouts=list(re.finditer(r'^\[browser.reader.layout\][^\r\n]*',trace,re.M))
 if layouts and documents[-1].end()>=layouts[-1].start():raise ValueError('released document path receipt must precede current reader layout')
 return path,ticket

def reader_geometry(trace,body,contract,anchor=0,expected_path=None):
 # Zoom changes the actual frame without emitting another tiling receipt.
 # Bind the shell's exact queued geometry to the notice accepted by Browser.
 queued=re.findall(r'\[shell.document.viewport\] target=15 task=15 surface=300 sequence=(\d+) x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) enqueued=1',trace)
 notices=re.findall(r'\[browser.viewport\] target=15 task=15 sequence=(\d+) w=(\d+) h=(\d+) origin=held_shell',trace)
 if not queued or not notices:raise ValueError('missing exact queued or held-shell viewport receipt')
 sequence,nw,nh=map(int,notices[-1]);matches=[tuple(map(int,row))for row in queued if int(row[0])==sequence]
 if len(matches)!=1:raise ValueError('accepted viewport lacks one exact shell geometry')
 _,x,y,w,h=matches[0]
 if(w,h)!=(nw,nh):raise ValueError('queued and accepted viewport differ')
 path,ticket=released_reader_document(trace,body,contract,expected_path)if contract.get('dynamic_reader_path')else(expected_path,None)
 frame=reader_frame(body,w,h,contract,anchor,path);frame.update(tile=dict(x=x,y=y,w=w,h=h),viewport=sequence)
 if ticket is not None:frame['document_ticket']=ticket
 return frame

def verify_reader_pixels(trace,ppm,body,contract,anchor=0,resolution=None,expected_path=None):
 try:
  expected=reader_geometry(trace,body,contract,anchor,expected_path);tile=expected['tile'];width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1)
  if resolution and (width,height)!=tuple(resolution):raise ValueError('wrong actual framebuffer resolution')
  if tile['x']<0 or tile['y']<50 or tile['x']+tile['w']>width or tile['y']+tile['h']>height:raise ValueError('reader tile outside framebuffer')
  layouts=list(re.finditer(r'\[browser.reader.layout\] first=(\d+) end=(\d+) body_rows=(\d+) rows=(\d+) cols=(\d+) total_lines=(\d+) source_bytes=(\d+) viewport=(\d+) origin=validated_body tls=0 pixels=unconfirmed',trace))
  wanted=tuple(expected[key]for key in ['first','end','body_rows','rows','cols','total_lines','source_bytes','viewport'])
  if not layouts or tuple(map(int,layouts[-1].groups()))!=wanted:raise ValueError('reader layout receipt differs from independent document/geometry/navigation model')
  reader_position(trace,layouts[-1].end(),expected,0,contract.get('reader_require_position',False))
  if not re.search(rf'\[browser.view.applied\] bytes={expected["bytes"]} http=258 body_bytes={len(body)} pixels=unconfirmed',trace[layouts[-1].end():]):raise ValueError('missing positive atomic reader Applied receipt after current layout')
  scale=expected['scale'];color=contract['color'].to_bytes(3,'big');checked=0;mismatch=0;ink=0
  columns=min(80,max(0,(tile['w']-2*contract['inset'])//(contract['advance']*scale)))
  # Inspect the entire former text band, including the spare column and all
  # rows below the bounded reader snapshot. Stale overview/body ink must erase.
  band_rows=min(contract['max_rows'],max(0,(tile['h']-contract['top']+contract['line']*scale-1)//(contract['line']*scale)))
  for row in range(band_rows):
   line=expected['lines'][row]if row<expected['rows']else b''
   for column in range(columns):
    bitmap=glyph(contract['font'],chr(line[column]))if column<len(line)else(0,)*7
    for gy in range(contract['line']):
     for gx in range(contract['advance']):
      on=gy<contract['glyph_height']and gx<5 and bool(bitmap[gy]&(1<<(4-gx)))
      for dy in range(scale):
       py=tile['y']+contract['top']+(row*contract['line']+gy)*scale+dy
       if py>=tile['y']+tile['h']:continue
       for dx in range(scale):
        px=tile['x']+contract['inset']+(column*contract['advance']+gx)*scale+dx
        actual=pixels[(py*width+px)*3:(py*width+px)*3+3]==color;checked+=1;ink+=on;mismatch+=actual!=on
  errors=[]if not mismatch else[f'whole reader glyph/padding/erasure mismatches={mismatch}']
  if not ink:errors.append('reader frame contains no checked ink')
  return dict(ok=not errors,errors=errors,geometry={k:v for k,v in expected.items()if k not in('starts','lines')},expected_rows=[line.decode('ascii')for line in expected['lines']],checked_pixels=checked,expected_ink_pixels=ink)
 except(ValueError,KeyError,IndexError,UnicodeError,OverflowError)as exc:return dict(ok=False,errors=[str(exc)])

def wait_reader_pixels(q,text,out,name,body,contract,anchor,health,timeout=30,resolution=None,expected_path=None,extra_check=None):
 deadline=time.monotonic()+timeout;attempts=0;screenshot=out/(name+'.ppm');result=dict(ok=False,errors=['no reader screenshot']);snapshot=''
 while time.monotonic()<deadline:
  health();attempts+=1;q.cmd('screendump',{'filename':str(screenshot)});snapshot=text();result=verify_reader_pixels(snapshot,screenshot.read_bytes(),body,contract,anchor,resolution,expected_path)
  if result['ok']and extra_check is not None:
   try:extra_check(snapshot,screenshot.read_bytes())
   except(ValueError,AssertionError)as exc:result.update(ok=False,errors=[str(exc)])
  if result['ok']:break
  time.sleep(.2)
 result.update(attempts=attempts,screenshot=screenshot.name,screenshot_sha256=hashlib.sha256(screenshot.read_bytes()).hexdigest()if screenshot.exists()else None,source_sha256=contract['source_sha256'],serial_end=len(snapshot),serial_sha256=hashlib.sha256(snapshot.encode()).hexdigest(),requested_anchor=anchor,scope='Independent complete bounded controlled document model, exact positively released document ticket/path/length, actual held-shell tile/viewport, complete reader chrome/body/footer glyphs, padding/spare column and stale text-band erasure. Applied/layout markers alone cannot pass; no additional network authority.')
 (out/(name+'-pixels.json')).write_text(json.dumps(result,indent=2)+'\n');return result

def http_status_geometry(text,expected,body,contract):
 tiles=re.findall(r'\[shell\.tile\.apply\] sid=300 x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text)
 previews=re.findall(r'\[browser\.preview\.layout\] bytes=(\d+) newline=1 cached_or_sample=1',text)
 echoes=re.findall(r'\[browser\.net\.job\.view\] state=(\w+) web=0',text)
 dns_views=re.findall(r'\[browser\.dns\.view\] state=([^\r\n]+?) web=0',text)
 if not tiles or not previews:raise ValueError('missing actual Browser geometry or preview length')
 echo=echoes[-1]if echoes else'Verified'if '[browser.net.job.released] ticket=1 view=Verified private_job=0'in text else None
 if echo not in contract['echo']:raise ValueError('missing supported preceding Echo status')
 dns=dns_views[-1]if dns_views else'Idle'
 if dns in contract['dns']:dns_label=contract['dns'][dns]
 else:
  address=re.fullmatch(r'Answer\(\[(\d+), (\d+), (\d+), (\d+)\]\)',dns)
  if not address or any(int(b)>255 for b in address.groups()):raise ValueError('unsupported preceding DNS status')
  dns_label='.'.join(address.groups()).encode()
 if expected not in contract['http']:raise ValueError('unsupported HTTP status')
 if len(body)>64 or any(b!=10 and not 32<=b<=126 for b in body):raise ValueError('invalid expected normalized HTTP body')
 x,y,w,h=map(int,tiles[-1]);length=int(previews[-1]);offset=length+128
 if not len(contract['prefix'])<length or length+258>contract['capacity']:raise ValueError('composed text exceeds production extent')
 scale=2 if w>=contract['scale_threshold']else 1;columns=max(1,min(80,(w-2*contract['inset'])//(contract['advance']*scale)))
 prior=contract['echo'][echo].ljust(64,b' ')+(contract['dns_prefix']+dns_label+contract['dns_footer']).ljust(64,b' ')
 status=(contract['http_prefix']+contract['http'][expected]+contract['http_footer']).ljust(64,b' ')
 if len(prior)!=128 or len(status)!=64:raise ValueError('invalid production fixed status extent')
 rendered_body=body.ljust(64,b' ')if expected=='Ready'else b' '*64
 data=contract['prefix']+b'?'*(length-len(contract['prefix']))+prior+status+b'\n\n'+rendered_body
 starts=[0];column=0
 for index,byte in enumerate(data):
  if byte==10:starts.append(index+1);column=0
  else:
   column+=1
   if column>=columns and index+1<len(data):starts.append(index+1);column=0
 if len(starts)>contract['max_rows']:raise ValueError('HTTP snapshot exceeds renderer row table')
 rows=[]
 for row,start in enumerate(starts):
  if start<=offset:continue
  end=starts[row+1]if row+1<len(starts)else len(data)
  rows.append(dict(row=row,text=data[start:end].rstrip(b'\n').decode('ascii'),x=x+contract['inset'],y=y+contract['top']+row*contract['line']*scale,start=start,end=end,body=end>length+194))
 if not rows or not any('Plain HTTP'in r['text']for r in rows):raise ValueError('missing complete HTTP label region')
 if any(r['y']+contract['line']*scale>y+h for r in rows):raise ValueError('complete HTTP/body rows are clipped by actual tile')
 return dict(tile=dict(x=x,y=y,w=w,h=h),scale=scale,columns=columns,preview_bytes=length,http_offset=offset,body_offset=length+194,body_bytes=64,echo_view=echo,dns_view=dns,rows=rows)

def verify_http_pixels(text,ppm,expected,body,contract,resolution=None):
 try:
  geometry=http_status_geometry(text,expected,body,contract);width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1);tile=geometry['tile'];scale=geometry['scale'];errors=[];checked=0;ink=0;body_checked=0;color=contract['color'].to_bytes(3,'big')
  if resolution and (width,height)!=tuple(resolution):raise ValueError('wrong actual framebuffer resolution')
  if tile['x']<0 or tile['y']<50 or tile['x']+tile['w']>width or tile['y']+tile['h']>height:raise ValueError('actual Browser tile outside framebuffer')
  for row in geometry['rows']:
   mismatch=0
   for column in range(geometry['columns']):
    bitmap=glyph(contract['font'],row['text'][column])if column<len(row['text'])else(0,)*7
    for gy in range(contract['line']):
     for gx in range(contract['advance']):
      on=gy<7 and gx<5 and bool(bitmap[gy]&(1<<(4-gx)))
      for dy in range(scale):
       for dx in range(scale):
        px=row['x']+(column*contract['advance']+gx)*scale+dx;py=row['y']+gy*scale+dy
        actual=pixels[(py*width+px)*3:(py*width+px)*3+3]==color;mismatch+=actual!=on;checked+=1;ink+=on;body_checked+=row['body']
   if mismatch:errors.append(f'Browser HTTP/body row={row["row"]} glyph_mismatches={mismatch}')
  if not ink or not body_checked:errors.append('missing complete HTTP label or body region')
  return dict(ok=not errors,errors=errors,expected=expected,expected_body_hex=body.hex()if expected=='Ready'else'',geometry=geometry,checked_pixels=checked,body_region_pixels=body_checked,expected_ink_pixels=ink)
 except(ValueError,KeyError,OverflowError,UnicodeError)as exc:return dict(ok=False,errors=[str(exc)],expected=expected)

def wait_http_pixels(q,text,out,expected,body,contract,health,name='browser-http',timeout=30,resolution=None):
 began=time.monotonic();deadline=began+timeout;attempts=0;result=dict(ok=False,errors=['no screenshot']);screenshot=out/(name+'.ppm');trace=''
 while time.monotonic()<deadline:
  try:
   health();attempts+=1;q.cmd('screendump',{'filename':str(screenshot)});trace=text();raw=screenshot.read_bytes();result=verify_http_pixels(trace,raw,expected,body,contract,resolution)
   result['errors']+=verify_kaleido_layout(trace,raw,contract['font'],resolution=resolution);result['ok']=not result['errors']
   if result['ok']:break
  except Exception as exc:result=dict(ok=False,errors=['HTTP pixel poll failed: '+repr(exc)],expected=expected);break
  time.sleep(.2)
 result.update(attempts=attempts,seconds=time.monotonic()-began,screenshot=screenshot.name,screenshot_sha256=hashlib.sha256(screenshot.read_bytes()).hexdigest()if screenshot.exists()else None,source_sha256=contract['source_sha256'],serial_end=len(trace),serial_sha256=hashlib.sha256(trace.encode()).hexdigest(),requested_anchor=None,scope='Complete actual tiled Plain HTTP/no TLS label, keyboard footer, padding, blank rows and all64body cells including trailing erasure, source-bound production font/style/composer and actual preceding status geometry. Serial completion alone cannot pass.')
 (out/(name+'-pixels.json')).write_text(json.dumps(result,indent=2)+'\n');return result

def wire_response(mode,body=BODY,max_body=None):
 status=b'HTTP/1.1 200 OK\r\n';extra=b'';length=len(body)
 if mode=='short-body':length+=9
 elif mode=='stream-short':body=body[:-9]
 elif mode=='oversized':
  if max_body is None:max_body=load_pixel_contract(Path(__file__).resolve().parents[1])['wire_max_body']
  body=b'X'*(max_body+1);length=len(body)
 elif mode=='redirect':status=b'HTTP/1.1 302 Found\r\n';extra=b'Location: http://outside.sexos.test/elsewhere\r\n'
 elif mode=='malformed':extra=b'Content-Length: '+str(length).encode()+b'\r\n'
 return status+b'Content-Type: text/plain; charset=us-ascii\r\nContent-Length: '+str(length).encode()+b'\r\n'+extra+b'Connection: close\r\n\r\n'+body

def resolution_joins(trace,pcap,expected,dns_validation):
 """Join positive app cleanup, private admission and the actual first SYN.

 The serial egress hash/length names the exact captured frame, rather than
 treating a serial state transition or host accept call as TCP wire proof.
 """
 errors=[];joins=[]
 admissions=list(re.finditer(r'\[net\.job\.resolved\.admitted\] owner=15 task=15 ticket=(\d+) dns_generation=(\d+) observed=(\d+) expires=(\d+) now=(\d+) fixed=1 at_begin=1',trace))
 if [(int(m[1]),int(m[2]))for m in admissions]!=expected:errors.append('resolved admission tickets/generations differ from exact intended HTTP jobs')
 syns={}
 try:
  for index,frame in enumerate(frames(pcap)):
   if len(frame)<54 or frame[12:14]!=b'\x08\0':continue
   ip=frame[14:];ihl=(ip[0]&15)*4
   if ip[0]>>4!=4 or ip[9]!=6 or ip[12:16]!=bytes([10,0,2,15])or ihl<20 or len(ip)<ihl+20:continue
   tcp=ip[ihl:];port=int.from_bytes(tcp[:2],'big')
   if 48001<=port<=65535 and tcp[13]&0x12==2:
    syns.setdefault(port-48000,dict(packet=index,length=len(frame),hash=hex(fingerprint(frame))))
  for admission in admissions:
   ticket,generation,observed,expires,now=map(int,admission.groups())
   release=re.search(rf'\[browser.dns.released\] ticket={generation} view=Answer\(\[10, 0, 2, 2\]\) private_job=0',trace)
   egress=re.search(rf'\[net.job.egress.submit\] ticket={ticket} sequence=(\d+) length=(\d+) hash=(0x[0-9a-f]+) scope=dma_admission_not_delivery',trace)
   answer=next((r for r in dns_validation.get('responses',[])if r['ticket']==generation),None);syn=syns.get(ticket)
   if not release or not egress or not answer or not syn:errors.append(f'HTTP ticket{ticket} lacks positive DNS cleanup, actual answer, admission or joined SYN');continue
   if not release.start()<admission.start()<egress.start():errors.append(f'HTTP ticket{ticket} DNS release/admission/egress order is invalid')
   if not observed<=now<expires or not 0<expires-observed<=60000:errors.append(f'HTTP ticket{ticket} was admitted outside its bounded observed lifetime')
   if syn['length']!=int(egress[2])or syn['hash']!=egress[3]:errors.append(f'HTTP ticket{ticket} admission egress does not name its captured first SYN')
   if answer['packet']>=syn['packet']:errors.append(f'HTTP ticket{ticket} emitted SYN before its actual DNS answer')
   joins.append(dict(ticket=ticket,dns_generation=generation,observed_tick=observed,expires_tick=expires,admitted_tick=now,dns_release_log_offset=release.start(),admission_log_offset=admission.start(),egress_log_offset=egress.start(),egress_sequence=int(egress[1]),dns_answer_packet=answer['packet'],tcp_first_syn=syn))
 except(ValueError,IndexError,struct.error)as exc:errors.append(str(exc))
 return dict(ok=not errors,errors=errors,joins=joins,scope='Exact positive DNS RELEASE precedes retained BEGIN_STREAM_RESOLVED admission; bounded admission time is observed in serial. Subsequent egress hash/length joins the actual first TCP SYN, which follows the actual captured DNS answer. DNS observation is provenance data under held BOOT authority and fixed endpoint policy, not DNS authenticity or new authority.')

def stream_receipts(trace,tickets,contract,large_tickets=(),receiving_tickets=()):
 """Bounded custody samples qualify consumption, never stand in for PCAP."""
 errors=[];admitted=[];drains=[];admit_offsets={}
 expected=(contract['stream_ring'],contract['stream_max_wire'],contract['stream_total_ticks'],contract['stream_idle_ticks'])
 if expected!=(1024,13312,30000,5000):errors.append('source streaming limits differ from the explicit bounded fixture')
 for m in re.finditer(r'\[net.job.stream.admitted\] owner=(\d+) task=(\d+) ticket=(\d+) kind=(\d+) ring=(\d+) max_wire=(\d+) total_ticks=(\d+) idle_ticks=(\d+) authority=held_route',trace):
  owner,task,ticket,kind,ring,maximum,total,idle=map(int,m.groups());admitted.append(ticket);admit_offsets[ticket]=m.start()
  if(owner,task,kind)!=(15,15,9)or(ring,maximum,total,idle)!=expected:errors.append('stream admission differs from exact owner/route/kind/source limits')
 if admitted!=list(tickets)or len(admitted)!=trace.count('[net.job.stream.admitted]'):errors.append('stream admission ticket sequence missing/extra/malformed')
 for ticket in tickets:
  resolved=re.search(rf'\[net.job.resolved.admitted\] owner=15 task=15 ticket={ticket} dns_generation=\d+ ',trace)
  egress=re.search(rf'\[net.job.egress.submit\] ticket={ticket} sequence=\d+ length=\d+ hash=0x[0-9a-f]+ scope=dma_admission_not_delivery',trace)
  if not resolved or not egress or ticket not in admit_offsets or not resolved.start()<admit_offsets[ticket]<egress.start():errors.append(f'ticket{ticket} streaming admission does not join resolved authorization before actual egress')
 previous={};complete=set()
 for m in re.finditer(r'\[net.job.stream.drain\] owner=(\d+) task=(\d+) ticket=(\d+) produced=(\d+) consumed=(\d+) buffered=(\d+) ring=(\d+) state_before=(\w+) receipt=retained not_delivery=1',trace):
  owner,task,ticket,produced,consumed,buffered,ring=map(int,m.groups()[:7]);state=m[8]
  if(owner,task)!=(15,15)or ticket not in tickets or ring!=expected[0]or not 0<consumed<=produced<=expected[1]or buffered!=produced-consumed or buffered>ring:errors.append('stream drain violates exact owner/cursor/ring bounds')
  old=previous.get(ticket)
  if ticket not in admit_offsets or m.start()<=admit_offsets[ticket]:errors.append('stream drain precedes its exact kind9 admission')
  if old and(consumed<=old[1]or produced<old[0]):errors.append('sampled stream produced/consumed counters regressed or repeated consumption')
  if state not in('Receiving','Complete')or ticket in complete and state!='Complete':errors.append('stream drain state is invalid or regressed')
  if state=='Complete':complete.add(ticket)
  previous[ticket]=(produced,consumed);drains.append(dict(ticket=ticket,produced=produced,consumed=consumed,buffered=buffered,state_before=state))
 if len(drains)!=trace.count('[net.job.stream.drain]'):errors.append('malformed stream drain sample')
 for ticket in receiving_tickets:
  if not any(row['ticket']==ticket and row['state_before']=='Receiving'for row in drains):errors.append(f'ticket{ticket} lacks actual destructive READ while Receiving')
 for ticket in large_tickets:
  if not any(row['ticket']==ticket and row['consumed']>4096 for row in drains):errors.append(f'ticket{ticket} never actually consumed beyond legacy4096')
 return dict(ok=not errors,errors=errors,admissions=admitted,drains=drains,source_limits=dict(ring=expected[0],max_wire=expected[1],total_ticks=expected[2],idle_ticks=expected[3]),scope='Exact held-route kind9 admissions and sampled actual destructive READ custody; cumulative produced and strictly increasing consumed remain within the1024-byte ring. Complete endpoint bytes require separate exact PCAP reassembly; no exact max-wire drain sample or delivery claim.')

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--model',choices=['e1000','e1000e'],default='e1000e');p.add_argument('--mode',choices=MODES,default='echo');p.add_argument('--resolution',choices=['1280x800','1024x768'],default='1280x800');p.add_argument('--binding',type=Path);p.add_argument('--wheel',action='store_true');p.add_argument('--check',action='store_true');a=p.parse_args()
 if a.wheel and a.binding is None:p.error('--wheel requires --binding')
 if a.check:
  if not a.wheel:p.error('--check requires --wheel')
  print(json.dumps(verify_wheel_evidence(Path(__file__).resolve().parents[1],a.iso.resolve(strict=True),a.binding.resolve(strict=True),a.out.resolve(strict=True))));return
 if a.wheel and a.mode not in('reader','stream','stream-close'):p.error('--wheel requires --mode reader, stream or stream-close')
 if a.source_disk is None:p.error('native run requires --source-disk')
 root=Path(__file__).resolve().parents[1];bound=None
 if a.wheel:
  from network_contention_gate import frozen_binding
  bound=frozen_binding(root,a.iso.resolve(strict=True),a.binding.resolve(strict=True))
 iso=a.iso.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 contract=load_wheel_contract(root)if a.wheel else load_pixel_contract(root);request=contract['request'];response_body=STREAM_BODY if a.mode in STREAM_MODES else READER_BODY if a.mode=='reader'else BODY;overview=overview_preview(response_body)if a.mode in('reader',)+STREAM_MODES else BODY;disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';qmp=Path('/tmp')/f'sex-http-{os.getpid()}.sock'
 if a.mode in STREAM_MODES and (contract['wire_max_body']!=len(STREAM_BODY)or contract['reader_max_body']!=len(STREAM_BODY)):raise RuntimeError('source Client/reader stream body limit differs from fixture')
 count=1 if a.mode in DNS_MODES else 4 if a.mode=='switch'else 3 if a.mode in('cancel-retry','stream-cancel-retry')else 2
 dns_count=2 if a.mode in('switch','cancel-retry','stream-cancel-retry')else 1
 rows=[];attempts=[];dns_rows=[];dns_attempts=[];server_errors=[];connections=[];stopping=threading.Event();http_seen=threading.Event();dns_seen=threading.Event();reset_after_drain=threading.Event();partial_sent=threading.Event()
 listener=socket.socket();listener.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);listener.bind(('127.0.0.1',46078));listener.listen(4);listener.settimeout(.2)
 dns_peer=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);dns_peer.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);dns_peer.bind(('127.0.0.1',46079));dns_peer.settimeout(.2)
 def serve_dns():
  try:
   while not stopping.is_set():
    try:body,remote=dns_peer.recvfrom(4096)
    except socket.timeout:continue
    ticket=len(dns_rows)+1
    row=dict(ticket=ticket,remote=list(remote),request_hex=body.hex(),request_sha256=hashlib.sha256(body).hexdigest(),host_monotonic=time.monotonic(),response_hex=None);dns_rows.append(row)
    if ticket>dns_count or body!=dns_query(ticket):raise RuntimeError('unexpected, duplicate, or noncanonical actual H DNS query')
    dns_seen.set()
    if a.mode in('dns-silence','dns-cancel','dns-close'):continue
    kind=a.mode[4:]if a.mode in DNS_SUCCESS+('dns-cname-cycle','dns-cname-conflict','dns-bad-pointer','dns-cname-unrelated')else'cname'if a.mode in('dns-cname-disallowed','dns-cname-zero-ttl')else'malformed'if a.mode=='dns-malformed'else'answer'
    reply=bytearray(dns_response(ticket,kind))
    if a.mode in('dns-disallowed','dns-cname-disallowed'):reply[-4:]=bytes([10,0,2,99])
    elif a.mode in('dns-zero-ttl','dns-cname-zero-ttl'):reply[40:44]=b'\0'*4
    row['response_hex']=bytes(reply).hex();attempt=dict(ticket=ticket,kind=a.mode if a.mode in DNS_MODES else'answer',response_hex=row['response_hex'],response_sha256=hashlib.sha256(reply).hexdigest(),remote=list(remote),host_monotonic=time.monotonic())
    try:attempt['sent_bytes']=dns_peer.sendto(reply,remote)
    except OSError as exc:attempt['send_error']=repr(exc);raise
    finally:dns_attempts.append(attempt)
  except Exception as exc:
   if not stopping.is_set():server_errors.append(repr(exc))
 dns_thread=threading.Thread(target=serve_dns,daemon=True);dns_thread.start()
 def write(conn,ticket,data,late=False,early_rejection=False):
  sent=bytearray();error=None
  try:
   for start in range(0,len(data),13):
    conn.sendall(data[start:start+13]);sent.extend(data[start:start+13]);time.sleep(.002)
  except OSError as exc:error=repr(exc)
  finally:attempts.append(dict(ticket=ticket,late_cancelled_peer=late,early_rejection=early_rejection,attempted_hex=data.hex(),sent_hex=bytes(sent).hex(),sent_sha256=hashlib.sha256(sent).hexdigest(),error=error))
  if error and not(late or early_rejection):raise RuntimeError('response write: '+error)
  return bytes(sent)
 def serve():
  old=None
  try:
   for ticket in range(1,count+1):
    while not stopping.is_set():
     try:conn,remote=listener.accept();break
     except socket.timeout:continue
    else:return
    connections.append(conn);conn.settimeout(30);body=bytearray()
    while True:
     part=conn.recv(512)
     if not part:break
     body.extend(part)
     if len(body)>4096:raise RuntimeError('unbounded actual guest request')
    echo=ticket==1 or(a.mode=='switch'and ticket==3);wanted=ECHO if echo else request
    row=dict(ticket=ticket,kind='echo'if echo else'http',remote=list(remote),request_hex=bytes(body).hex(),request_sha256=hashlib.sha256(body).hexdigest(),response_hex='',aborted=False);rows.append(row)
    if bytes(body)!=wanted:raise RuntimeError('actual guest request differs from fixed production request')
    if echo:
     row['response_hex']=write(conn,ticket,ECHO).hex();conn.shutdown(socket.SHUT_WR);conn.close();continue
    http_seen.set()
    if ticket==2 and a.mode in('stream-reset','stream-cancel-retry','stream-close'):
     full=wire_response('stream',response_body,contract['wire_max_body']);header_end=full.index(b'\r\n\r\n')+4
     prefix=full[:header_end+(5120 if a.mode=='stream-reset'else 2048)]
     row.update(aborted=True,response_hex=full.hex(),prefix_bytes=len(prefix),host_sent_prefix_hex=write(conn,ticket,prefix).hex(),response_scope='Full planned response and actual sent prefix are separate; complete delivery is never inferred from sendall')
     partial_sent.set()
     if a.mode=='stream-reset':
      deadline=time.monotonic()+90
      while not stopping.is_set()and not reset_after_drain.wait(.1):
       if time.monotonic()>deadline:raise TimeoutError('actual >4096 stream drain before peer RST')
      if stopping.is_set():return
      conn.setsockopt(socket.SOL_SOCKET,socket.SO_LINGER,struct.pack('ii',1,0));conn.close();continue
     old=(conn,row)
     if a.mode=='stream-close':
      while not stopping.wait(.1):pass
      return
     continue
    silent=ticket==2 and a.mode in('silence','cancel-retry','close')
    if silent:
     row['aborted']=True;old=(conn,row)
     if a.mode!='cancel-retry':
      while not stopping.wait(.1):pass
      return
     continue
    if ticket==2 and a.mode=='reset':row['aborted']=True;conn.setsockopt(socket.SOL_SOCKET,socket.SO_LINGER,struct.pack('ii',1,0));conn.close();continue
    if old is not None:
     old_conn,old_row=old
     if a.mode=='stream-cancel-retry':
      remaining=bytes.fromhex(old_row['response_hex'])[old_row['prefix_bytes']:]
      old_row['host_sent_late_suffix_hex']=write(old_conn,2,remaining,late=True).hex();old_row['response_scope']='Planned complete response; initial actual prefix and attempted late remaining suffix retained separately, actual wire independently qualifies only its captured prefix'
     else:
      old_row['response_hex']=wire_response('echo').hex();old_row['host_sent_prefix_hex']=write(old_conn,2,wire_response('echo'),late=True).hex();old_row['response_scope']='attempted late bytes; actual captured prefix is independently qualified'
     old_conn.close();old=None
    reply=wire_response(a.mode if ticket==2 else'echo',response_body,contract['wire_max_body'])
    if a.mode=='oversized':
     row.update(aborted=True,response_hex=reply.hex(),host_sent_prefix_hex=write(conn,ticket,reply,early_rejection=True).hex(),response_scope='Exact source-bound oversized header/body attempt; early incremental rejection may end the wire at its checked prefix')
     try:conn.shutdown(socket.SHUT_WR)
     except OSError:pass
    else:row['response_hex']=write(conn,ticket,reply).hex();conn.shutdown(socket.SHUT_WR)
    conn.close()
  except Exception as exc:
   if not stopping.is_set():server_errors.append(repr(exc))
 thread=threading.Thread(target=serve,daemon=True);thread.start()
 cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-vga','none','-device','VGA,xres='+a.resolution.replace('x',',yres='),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{qmp},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}']
 authority_paths=('apps/kaleidoscope/src/dns_probe.rs','servers/sexnet/src/dns_jobs.rs','servers/sexnet/src/jobs.rs','servers/sexnet/src/main.rs','crates/sex-pdx/src/network_job.rs','crates/sex-net/src/dns_wire.rs')
 (out/'manifest.json').write_text(json.dumps(dict(iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),input_disk_sha256=hashlib.sha256(seed.read_bytes()).hexdigest(),command=cmd,mode=a.mode,model=a.model,resolution=a.resolution,qemu_version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True),pixel_source_sha256=contract['source_sha256'],authority_source_sha256={name:hashlib.sha256((Path(__file__).resolve().parents[1]/name).read_bytes()).hexdigest()for name in authority_paths},runner_sha256={name:hashlib.sha256((Path(__file__).parent/name).read_bytes()).hexdigest()for name in('http_content_gate.py','check_tcp_capture.py','check_dns_capture.py')},http_request_hex=request.hex(),http_body_hex=response_body.hex(),scope='Actual H now obtains a fixed-name UDP DNS observation, positively releases it, then requests BEGIN_STREAM_RESOLVED kind9 under the existing held BOOT route and immutable HTTP endpoint policy. Real J/R/V/navigation/F11 keys, source-bound reader pages with actual ZoomToggle/restore where selected, loopback UDP46079/TCP46078 fixtures and full shared PCAP. No DNS authenticity, TTL wall-clock accuracy, new capability authority, TLS, public routing, HTML, redirect following or cache claim.'),indent=2)+'\n')
 manifest_path=out/'manifest.json';manifest=json.loads(manifest_path.read_text());manifest.update(wheel=a.wheel,binding=bound);manifest_path.write_text(json.dumps(manifest,indent=2)+'\n')
 proc=subprocess.Popen(cmd,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);actions=[];pixel_results=[];reader_results=[];failure=None;wheel_actions=[];wheel_parking=[];wheel_parked=[];wheel_other=[];wheel_park_phase=0;wheel_close_start=None;wheel_need_erasure=False
 def text():return serial_text(serial)if serial.exists()else''
 def health():
  if server_errors:raise RuntimeError(str(server_errors))
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|sexnet.fatal|serial.record.overflow',text()):raise RuntimeError('guest failure')
 def wait(pattern,start=0,timeout=90):
  deadline=time.monotonic()+timeout
  while time.monotonic()<deadline:
   health();match=re.search(pattern,text()[start:])
   if match:return match
   time.sleep(.02)
  raise TimeoutError(pattern)
 def wait_stream_drain(ticket,minimum):
  deadline=time.monotonic()+90
  while time.monotonic()<deadline:
   health()
   samples=list(re.finditer(rf'\[net.job.stream.drain\] owner=15 task=15 ticket={ticket} produced=(\d+) consumed=(\d+) buffered=(\d+) ring=1024 state_before=Receiving receipt=retained not_delivery=1',text()))
   if any(int(m[2])>=minimum for m in samples):return
   time.sleep(.02)
  raise TimeoutError(f'actual Receiving drain ticket{ticket} consumed>={minimum}')
 def focus(q):
  for _ in range(8):
   current=re.findall(r'\[shell.focus.set\] id=(\d+)',text())
   if current and current[-1]=='300':return
   start=len(text());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':code}}}for code,down in [('alt',True),('tab',True),('tab',False),('alt',False)]]});wait(r'\[shell.app.switch\]',start)
  raise RuntimeError('actual AltTab did not focus Browser')
 def pixels(q,view,name):
  if a.wheel:wheel_park(q,name)
  result=wait_http_pixels(q,text,out,view,overview,contract,health,name,resolution=tuple(map(int,a.resolution.split('x'))));result['expected_view']=view;pixel_results.append(result)
  if a.wheel:wheel_parked.append(result)
  if not result['ok']:raise RuntimeError('actual HTTP pixels: '+'; '.join(result['errors']))
 def ready(q,ticket,start,name):
  wait(rf'\[browser.http.body\] ticket={ticket} status=200 length={len(response_body)} hash={hex(fingerprint(response_body))} controlled_peer=1 tls=0',start)
  wait(rf'\[browser.http.released\] ticket={ticket} view=Ready private_job=0',start)
  wait(rf'\[browser.view.applied\] bytes=\d+ http=2 body_bytes={len(response_body)} pixels=unconfirmed',start);pixels(q,'Ready',name)
 def resolution(ticket,generation,start):
  wait(rf'\[browser.dns.released\] ticket={generation} view=Answer\(\[10, 0, 2, 2\]\) private_job=0',start)
  wait(rf'\[net.job.resolved.admitted\] owner=15 task=15 ticket={ticket} dns_generation={generation} ',start)
 def wheel_park(q,name):
  nonlocal wheel_park_phase
  from launcher_workflow_gate import pointer_park
  pointer_park(q,tuple(map(int,a.resolution.split('x'))),text,wait,wheel_parking.append,wheel_park_phase);wheel_park_phase+=1;wheel_parking[-1].update(capture=name,serial_end=len(text()))
 def wheel_other_picture(q,name,kind):
  wheel_park(q,name);path=out/(name+'.ppm');deadline=time.monotonic()+25;error=None
  while time.monotonic()<deadline:
   health();q.cmd('screendump',{'filename':str(path)});snapshot=text()
   try:
    if kind=='launcher':
     from check_launcher_pixels import verify as check_launcher
     errors=check_launcher(snapshot,path.read_bytes(),contract['font'],tuple(map(int,a.resolution.split('x'))))
     if errors:raise ValueError(str(errors))
    else:
     width,height,_=ppm_rgb(path.read_bytes(),min_width=1,min_height=1)
     if(width,height)!=tuple(map(int,a.resolution.split('x'))):raise ValueError('closed resolution differs')
    frame=dict(kind=kind,requested_anchor=None,screenshot=path.name,screenshot_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),serial_end=len(snapshot),serial_sha256=hashlib.sha256(snapshot.encode()).hexdigest());wheel_other.append(frame);wheel_parked.append(frame);return frame
   except(ValueError,AssertionError)as exc:error=str(exc);time.sleep(.1)
  (out/'wheel-failed-frame.json').write_text(json.dumps(dict(name=name,error=error,serial_end=len(snapshot),serial_sha256=hashlib.sha256(snapshot.encode()).hexdigest(),screenshot_sha256=hashlib.sha256(path.read_bytes()).hexdigest()),indent=2)+'\n');raise RuntimeError(error)
 def wheel_send(q,name,wheel,target,before,x,y):
  from launcher_workflow_gate import pointer_move
  resolution=tuple(map(int,a.resolution.split('x')));pointer_start=len(text());pointer_move(q,x,y,resolution,text,wait,lambda row:None)
  after=wheel_move(reader_geometry(text(),response_body,contract,before),wheel)[0]if target=='body'else before
  event=dict(name=name,wheel=wheel,target=target,before=before,after=after,x=x,y=y,pointer_start=pointer_start,serial_start=len(text()),host_start=time.monotonic(),transport='input-send-event',events=wheel_events(wheel));wheel_actions.append(event)
  q.cmd('input-send-event',{'events':event['events']});event.update(serial_qmp_return=len(text()),host_qmp_return=time.monotonic());deadline=time.monotonic()+30;error='no wheel receipts'
  while time.monotonic()<deadline:
   health();snapshot=text();event.update(serial_end=len(snapshot),host_end=time.monotonic())
   try:check=inspect_wheel_action(snapshot,event,response_body,contract,resolution);return check['anchor']
   except(ValueError,AssertionError,KeyError)as exc:error=str(exc);time.sleep(.02)
  raise RuntimeError('real wheel action '+name+': '+error)
 def wheel_body_point():
  tile,_=wheel_viewport(text());return tile['x']+tile['w']-24,tile['y']+64
 def wheel_close(q):
  nonlocal wheel_close_start
  from clipboard_workflow_gate import hardware_keys
  from network_contention_gate import cleanup_receipts
  wheel_close_start=len(text());hardware_keys(q,('f11',));wait(r'\[browser.http.closed\] pending=0 terminal=1',wheel_close_start);wait(r'\[sext.app.pool.ready\] target=15 task=15 .*ok=1',wheel_close_start);cleanup_receipts(text(),15,15,wheel_close_start)
  x,y=wheel_body_point();wheel_send(q,'closed',-1,'closed',0,x,y);wheel_other_picture(q,'browser-wheel-closed','closed')
 def reader_workflow(q):
  nonlocal wheel_need_erasure
  contract['reader_require_position']=True
  anchor=0;start_network=len(text());original=None
  def paint(name):
   nonlocal anchor
   if a.wheel:wheel_park(q,name)
   extra=None
   if a.wheel and wheel_need_erasure:
    from launcher_workflow_gate import launcher_erased
    extra=lambda prefix,image:launcher_erased(prefix,image,contract['launcher'],tuple(map(int,a.resolution.split('x'))))
   result=wait_reader_pixels(q,text,out,name,response_body,contract,anchor,health,resolution=tuple(map(int,a.resolution.split('x'))),extra_check=extra);reader_results.append(result)
   if not result['ok']:raise RuntimeError('actual reader pixels: '+'; '.join(result['errors']))
   if a.wheel:
    if wheel_need_erasure:result['launcher_erasure']=True
    wheel_parked.append(result)
   return reader_geometry(text()[:result['serial_end']],response_body,contract,anchor)
  def navigation(key,code,name=None):
   nonlocal anchor
   prior=reader_geometry(text(),response_body,contract,anchor);anchor=reader_move(prior,key)
   start=len(text());q.key(key);wait(rf'\[shell.browser.input\] code={code} down=1 slot=43 accepted=1',start)
   frame=paint(name or'browser-reader-'+key);actions.append(dict(action='actual_reader_'+key,first=frame['first'],end=frame['end'],source_anchor=anchor));return frame
  def zoom(name):
   start=len(text());q.key('grave_accent');wait(r'\[shell.palette.item\] idx=0',start)
   for old,new in [(0,9),(9,8)]:
    start=len(text());q.key('k');wait(rf'\[shell.palette.select\] old={old} new={new}',start)
   start=len(text());q.key('ret');wait(r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1',start)
   wait(r'\[browser.viewport\] target=15 task=15 sequence=\d+ w=\d+ h=\d+ origin=held_shell',start)
   frame=paint(name);actions.append(dict(action='actual_palette_'+name,tile=frame['tile'],source_anchor=anchor));return frame
  start=len(text());q.key('v');wait(r'\[shell.browser.input\] code=47 down=1 slot=43 accepted=1',start)
  first=paint('browser-reader-initial');original=first['tile']
  if first['first']!=0 or first['source_bytes']!=len(response_body) or first['total_lines']<=first['body_rows']:raise RuntimeError('native reader fixture does not exercise full document pagination')
  if a.wheel:
   from clipboard_workflow_gate import hardware_keys
   from check_launcher_pixels import layout as launcher_layout
   x,y=wheel_body_point()
   for name,delta in [('top',1),('down',-1),('up',1),('burst',-3)]:anchor=wheel_send(q,name,delta,'body',anchor,x,y);paint('browser-wheel-'+name)
   navigation('end',107,'browser-wheel-end-entry');anchor=wheel_send(q,'end',-1,'body',anchor,x,y);paint('browser-wheel-end-clamp')
   tile,_=wheel_viewport(text());anchor=wheel_send(q,'outside',-1,'outside',anchor,int(a.resolution.split('x')[0])-20,10);paint('browser-wheel-outside')
   anchor=wheel_send(q,'title',-1,'title',anchor,tile['x']+tile['w']//2,tile['y']+14);paint('browser-wheel-title')
   start=len(text());hardware_keys(q,('grave_accent',));wait(r'\[shell.palette.item\] idx=0',start)
   deadline=time.monotonic()+15
   while True:
    try:modal,_=launcher_layout(text());break
    except ValueError:
     health()
     if time.monotonic()>deadline:raise
     time.sleep(.02)
   left=max(tile['x']+8,modal['x']+8);right=min(tile['x']+tile['w']-8,modal['x']+modal['w']-8);top=max(tile['y']+36,modal['y']+36);bottom=min(tile['y']+tile['h']-8,modal['y']+modal['h']-8)
   if left>=right or top>=bottom:raise RuntimeError('actual launcher does not overlap Browser body')
   anchor=wheel_send(q,'modal',-1,'modal',anchor,(left+right)//2,(top+bottom)//2);wheel_other_picture(q,'browser-wheel-modal','launcher')
   start=len(text());hardware_keys(q,('esc',));wait(r'\[command_palette.close\]',start);wheel_need_erasure=True;paint('browser-wheel-modal-erased');navigation('home',102,'browser-wheel-home')
  navigation('pgdn',109);navigation('down',108);navigation('pgup',104);navigation('home',102)
  end=navigation('end',107)
  if end['end']!=end['total_lines']or (STREAM_TAIL if a.mode in STREAM_MODES else READER_TAIL).decode()not in ''.join(reader_results[-1]['expected_rows'][2:-1]):raise RuntimeError('reader End lacks final full-document marker')
  navigation('up',103)
  expanded=zoom('browser-reader-expanded');restored=zoom('browser-reader-restored')
  if expanded['tile']['w']==original['w']and expanded['tile']['h']==original['h']:raise RuntimeError('real ZoomToggle did not change reader geometry')
  if(restored['tile']['w'],restored['tile']['h'])!=(original['w'],original['h']):raise RuntimeError('real ZoomToggle did not restore reader geometry')
  start=len(text());q.key('v');wait(rf'\[browser.view.applied\] bytes=\d+ http=2 body_bytes={len(response_body)} pixels=unconfirmed',start);pixels(q,'Ready','browser-reader-overview')
  retained=anchor;start=len(text());q.key('v');wait(r'\[shell.browser.input\] code=47 down=1 slot=43 accepted=1',start);again=paint('browser-reader-resumed')
  if again['anchor']!=retained:raise RuntimeError('same-document reader re-entry lost source anchor')
  start=len(text());q.key('j');wait(r'\[browser.view.applied\] bytes=\d+ http=4 body_bytes=0 pixels=unconfirmed',start);pixels(q,'Cancelled','browser-reader-erased')
  layouts=len(re.findall(r'\[browser.reader.layout\]',text()));start=len(text());q.key('v');wait(r'\[shell.browser.input\] code=47 down=1 slot=43 accepted=1',start);pixels(q,'Cancelled','browser-reader-cannot-reopen')
  if len(re.findall(r'\[browser.reader.layout\]',text()))!=layouts or re.search(r'\[browser.view.applied\] bytes=\d+ http=258 ',text()[start:]):raise RuntimeError('cancelled document reopened reader')
  if re.search(r'\[net\.(?:job|dns)\.egress\.submit\]|\[net\.job\.resolved\.admitted\]|\[browser.http.body\]|\[browser.dns.answer\]',text()[start_network:]):raise RuntimeError('reader navigation issued new owned network work')
  if len(rows)!=count or len(dns_rows)!=dns_count:raise RuntimeError('reader navigation changed actual peer exchange count')
  actions.append(dict(action='reader_local_navigation_only',http_connections=count,dns_queries=dns_count,cancelled_document_cannot_reopen=True))
  if a.wheel:wheel_close(q)
 def close_pending(q,start,kind):
  nonlocal wheel_close_start
  if a.wheel:wheel_close_start=start
  began=time.monotonic();q.key('f11');wait(rf'\[browser.{kind}.closed\] pending=1 terminal=1',start);wait(r'\[sext.app.pool.ready\] target=15 .*ok=1',start)
  trace=text();roles={int(n)for n in re.findall(r'\[app.service.ack\] target=15 .*role=(\d+) ',trace[start:])};ack=trace.rfind('[net.app.cleanup.ack] target=15')
  retired=re.search(r'\[net.dns.records.retired\] target=15 task=15 remaining=0 before_cleanup_ack=1',trace[start:])
  if roles!={1,2,4,8,16}or ack<start or not retired or retired.start()+start>=ack or re.search(r'\[net\.(?:job|dns)\.egress\.submit\]',trace[ack:]):raise RuntimeError('pending Close lacks zero DNS records before ACK and terminal owned-egress cleanup')
  wheel_other_picture(q,'browser-closed','closed')if a.wheel else q.cmd('screendump',{'filename':str(out/'browser-closed.ppm')});actions.append(dict(action='actual_f11_close_pending_'+kind,roles=sorted(roles),dns_records_remaining=0,seconds=time.monotonic()-began));pixel_results.append(dict(ok=True,skipped=True,reason='Browser terminally closed; no live HTTP status surface remains. All five cleanup roles, zero retained DNS records before network cleanup ACK, and wire evidence qualify Close.'))
 try:
  deadline=time.monotonic()+15
  while not qmp.exists():
   health()
   if time.monotonic()>deadline:raise RuntimeError('QMP unavailable')
   time.sleep(.02)
  q=QMP(qmp)
  if a.resolution=='1024x768':
   wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret');actions.append(dict(action='actual_compact_boot_menu',requested_resolution=[1024,768]))
  wait(r'\[app.surface.presented\] target=15 surface=300');wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0')
  focus(q);start=len(text());began=time.monotonic();q.key('h')
  deadline=time.monotonic()+60
  while not dns_seen.wait(.02):
   health()
   if time.monotonic()>deadline:raise RuntimeError('actual H did not emit real DNS query')
  actions.append(dict(action='actual_h_dns_then_resolved_fetch',dns_ticket=1,seconds=time.monotonic()-began))
  if a.mode in DNS_MODES:
   if a.mode=='dns-close':close_pending(q,len(text()),'dns')
   elif a.mode=='dns-cancel':
    start=len(text());began=time.monotonic();q.key('j');wait(r'\[browser.dns.released\] ticket=1 view=Cancelled private_job=0',start);wait(r'\[browser.view.applied\] bytes=\d+ http=4 body_bytes=0 ',start);pixels(q,'Cancelled','browser-http');actions.append(dict(action='actual_j_cancel_waiting_dns',seconds=time.monotonic()-began,http_job_allocated=False))
   else:
    dns_view=r'Answer\(\[10, 0, 2, 99\]\)'if a.mode in('dns-disallowed','dns-cname-disallowed')else r'Answer\(\[10, 0, 2, 2\]\)'if a.mode in('dns-zero-ttl','dns-cname-zero-ttl')else'Unavailable'
    wait(rf'\[browser.dns.released\] ticket=1 view={dns_view} private_job=0',start);wait(r'\[browser.http.view\] state=Unavailable tls=0',start);wait(r'\[browser.view.applied\] bytes=\d+ http=3 body_bytes=0 ',start);pixels(q,'Unavailable','browser-http')
   if any(int(n)>1 for n in re.findall(r'\[net.job.egress.submit\] ticket=(\d+) ',text()))or '[net.job.resolved.admitted]'in text():raise RuntimeError('DNS denial/cancellation/Close admitted new HTTP-owned egress')
  else:
   resolution(2,1,start)
   deadline=time.monotonic()+60
   while not http_seen.wait(.02):
    health()
    if time.monotonic()>deadline:raise RuntimeError('released DNS observation did not produce actual resolved HTTP request')
  if a.mode in('stream-reset','stream-cancel-retry','stream-close'):
   deadline=time.monotonic()+60
   while not partial_sent.wait(.02):
    health()
    if time.monotonic()>deadline:raise TimeoutError('controlled stream prefix send not completed')
   wait_stream_drain(2,5120 if a.mode=='stream-reset'else 1024)
   if a.mode=='stream-reset':reset_after_drain.set()
  if a.mode in('close','stream-close'):
   if a.wheel:
    x,y=wheel_body_point();wheel_send(q,'pending-burst',-3,'pending',0,x,y);pixels(q,'Pending','browser-wheel-pending')
   close_pending(q,len(text()),'http')
   if a.wheel:
    x,y=wheel_body_point();wheel_send(q,'closed',-1,'closed',0,x,y);wheel_other_picture(q,'browser-wheel-after-close','closed')
  elif a.mode in('cancel-retry','stream-cancel-retry'):
   start=len(text());began=time.monotonic();q.key('j');wait(r'\[browser.http.released\] ticket=2 view=Cancelled private_job=0',start);wait(r'\[browser.view.applied\] bytes=\d+ http=4 body_bytes=0 ',start);pixels(q,'Cancelled','browser-cancelled');actions.append(dict(action='actual_j_cancel',ticket=2,seconds=time.monotonic()-began));start=len(text());q.key('h');resolution(3,2,start);ready(q,3,start,'browser-http');actions.append(dict(action='actual_h_fresh_retry',ticket=3,dns_generation=2))
   if a.mode=='stream-cancel-retry':reader_workflow(q)
  elif a.mode in('echo','switch','reader','stream')+DNS_SUCCESS:
   ready(q,2,start,'browser-http-first'if a.mode=='switch'else'browser-http')
   if a.mode in('reader','stream'):reader_workflow(q)
   if a.mode=='switch':
    start=len(text());q.key('j');wait(r'\[browser.view.applied\] bytes=\d+ http=4 body_bytes=0 ',start);pixels(q,'Cancelled','browser-erased');actions.append(dict(action='actual_j_erase_previous_ready_body',job_allocated=False))
    start=len(text());q.key('r');wait(r'\[browser.net.job.released\] ticket=3 view=Verified private_job=0',start);actions.append(dict(action='actual_r_echo_same_replay_fence',ticket=3));start=len(text());q.key('h');resolution(4,2,start);ready(q,4,start,'browser-http');actions.append(dict(action='actual_h_after_echo',ticket=4,dns_generation=2))
  elif a.mode not in DNS_MODES:
   wait(r'\[browser.http.released\] ticket=2 view=Unavailable private_job=0',start);wait(r'\[browser.view.applied\] bytes=\d+ http=3 body_bytes=0 ',start);pixels(q,'Unavailable','browser-http')
  expected_bodies=[2,4]if a.mode=='switch'else[3]if a.mode in('cancel-retry','stream-cancel-retry')else[2]if a.mode in('echo','reader','stream')+DNS_SUCCESS else[]
  if [int(n)for n in re.findall(r'\[browser.http.body\] ticket=(\d+) ',text())]!=expected_bodies:raise RuntimeError('invalid, incomplete, cancelled or absent HTTP response became usable body')
  health()
 except Exception as exc:failure=repr(exc)
 finally:
  stopping.set()
  for conn in connections:
   try:conn.close()
   except OSError:pass
  listener.close();thread.join(timeout=2)
  dns_peer.close();dns_thread.join(timeout=2)
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=10)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
 (out/'endpoint-exchanges.json').write_text(json.dumps(dict(connections=rows,response_send_attempts=attempts),indent=2)+'\n')
 (out/'dns-endpoint-exchanges.json').write_text(json.dumps(dict(queries=dns_rows,response_send_attempts=dns_attempts),indent=2)+'\n')
 expected=[]
 for row in rows:
  item=dict(payload_hex=row['request_hex'],response_hex=row['response_hex'])
  if row['aborted']:item.update(aborted=True,allow_empty_reply=True)
  expected.append(item)
 raw_pcap=pcap.read_bytes()if pcap.exists()else b''
 validation=capture(raw_pcap,expected,guest_ports=range(48001,65536),initiator='guest')if raw_pcap else dict(ok=False,errors=['missing actual full PCAP'])
 dns_expected=[dict(ticket=row['ticket'],response_hex=row['response_hex'])for row in dns_rows]
 dns_validation=dns_capture(raw_pcap,'echo',dns_attempts,expected=dns_expected)if raw_pcap and dns_expected else dict(ok=False,errors=['missing actual H DNS query/capture'])
 admissions=[]if a.mode in DNS_MODES else[(2,1),(4,2)]if a.mode=='switch'else[(2,1),(3,2)]if a.mode in('cancel-retry','stream-cancel-retry')else[(2,1)]
 joined=resolution_joins(text(),raw_pcap,admissions,dns_validation)
 if a.mode in DNS_SUCCESS and joined['ok']:
  # The captured alias fixtures carry23s/7s CNAME TTLs and60s A data.
  # Authority/additional TTL1 is unrelated and must not shorten admission.
  lifetime=60000 if a.mode=='dns-compressed'else 7000
  if len(joined['joins'])!=1 or joined['joins'][0]['expires_tick']-joined['joins'][0]['observed_tick']!=lifetime:
   joined['ok']=False;joined['errors'].append('resolved provenance TTL differs from the exact captured CNAME chain/final A RRset')
 large=[3]if a.mode=='stream-cancel-retry'else[2]if a.mode in('stream','stream-short','stream-reset')else[]
 receiving=[2,3]if a.mode=='stream-cancel-retry'else[2]if a.mode in STREAM_MODES else[]
 streaming=stream_receipts(text(),[ticket for ticket,_ in admissions],contract,large,receiving)
 authority=nic(text(),1,True,True)
 if len(rows)!=count:failure=failure or'incomplete actual peer exchanges'
 if len(dns_rows)!=dns_count:failure=failure or'incomplete actual DNS exchanges'
 if a.mode in('reset','stream-reset')and validation.get('ok')and not validation['connections'][1]['rst_in']:failure=failure or'missing captured real peer reset'
 if validation.get('ok')and a.mode=='stream-reset':
  header_len=wire_response('stream',response_body,contract['wire_max_body']).index(b'\r\n\r\n')+4
  if validation['connections'][1]['incoming_bytes']<=header_len+4096:failure=failure or'RST did not follow actual captured >4096 body prefix'
 if validation.get('ok')and a.mode=='oversized':
  header_len=wire_response('oversized',BODY,contract['wire_max_body']).index(b'\r\n\r\n')+4
  if validation['connections'][1]['incoming_bytes']<header_len:failure=failure or'oversized denial lacks captured complete invalid Content-Length header'
 result=dict(ok=failure is None and validation['ok']and dns_validation['ok']and joined['ok']and streaming['ok']and authority['ok']and not server_errors,error=failure,server_errors=server_errors,capture=validation,dns_capture=dns_validation,resolution_order=joined,streaming=streaming,authority=authority,pixels=pixel_results,reader_pixels=reader_results,actions=actions,scope='Actual H-driven fixed-name DNS query/answer and positive cleanup precede generation-bound HTTP admission under existing held BOOT authority and fixed endpoint policy. Exact first TCP SYN joins the admitted egress frame hash/length and follows the captured DNS answer. Denial modes require only baseline Echo TCP and no new HTTP-owned egress. Full HTTP/body pixels or explicit terminal Close qualify presentation/cleanup. Reader/stream modes additionally require complete2048/12288-byte full-document pages, actual navigation and geometry changes, overview return/reset, cancellation erasure and no new owned network work. No injected native expiry/rollback, DNS authenticity, wall-clock TTL accuracy, new capabilities, TLS, public web, HTML, cache or physical/SMP isolation claim.')
 if a.wheel:
  evidence=dict(serial_offset_units=WHEEL_SERIAL_UNITS,serial_prefix_hash_encoding='UTF-8',actions=wheel_actions,parking=wheel_parking,parked_frames=wheel_parked,reader_frames=reader_results,status_frames=[r for r in pixel_results if r.get('screenshot')],other_frames=wheel_other,close_start=wheel_close_start)
  names={'serial.log','wire.pcap','endpoint-exchanges.json','dns-endpoint-exchanges.json'}|{r['screenshot']for r in evidence['reader_frames']+evidence['status_frames']+wheel_other};evidence['raw_sha256']={name:hashlib.sha256((out/name).read_bytes()).hexdigest()for name in names if(out/name).is_file()};(out/'wheel-evidence.json').write_text(json.dumps(evidence,indent=2)+'\n')
  if result['ok']:
   try:result['wheel']=verify_wheel_evidence(root,iso,a.binding.resolve(strict=True),out)
   except Exception as exc:result.update(ok=False,error='independent wheel replay: '+repr(exc))
 (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if not result['ok']:raise RuntimeError(json.dumps(result))
 print(json.dumps(dict(ok=True,mode=a.mode,model=a.model,connections=len(rows))))
if __name__=='__main__':main()
