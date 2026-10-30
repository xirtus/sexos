#!/usr/bin/env python3
"""Native register-only Browser→Quil clipboard qualification; never builds.

Fixtures are a finite loopback endpoint. Guest authority remains its held BOOT
routes in one GLOBAL_VAS. Synthetic tests exercise this checker, not the guest.
"""
import argparse,ast,gzip,hashlib,json,os,re,shutil,socket,subprocess,threading,time
from pathlib import Path
import http_content_gate as http
import https_import_gate as storage
import tls_responsiveness_gate as desktop
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from network_contention_gate import frozen_binding,cleanup_receipts,MAX_PCAP
from tls_exchange_gate import reassemble
from check_document_disk import recover,fnv
from check_desktop_pixels import final_view,glyph,text_cells
from check_quil_ui_pixels import view as quil_view,verify as quil_pixels
from check_disk_persistence import ppm_rgb
from check_nonunix_target import inspect as target_check

BODY=http.STREAM_BODY
NAME='clipnote'
def utf8_fixture():
 tail=b'\nUTF8 CLIPBOARD BYTE TAIL 105 END'
 seed=b'finite UTF8 byte storage and scalar cells\n'
 body=bytearray((seed*((12288+len(seed)-1)//len(seed)))[:12288])
 prefix='Aé€🙂漢\t\n'.encode('utf8');body[:len(prefix)]=prefix
 scalars=[c.encode('utf8')for c in ('é','€','🙂','漢','𐍈')]
 for index,boundary in enumerate(range(64,12288-len(tail)-16,16)):
  scalar=scalars[index%len(scalars)];split=1+index%(len(scalar)-1);start=boundary-split
  body[start:start+len(scalar)]=scalar;body[boundary+7]=9;body[boundary+11]=10
 body[-len(tail):]=tail
 return bytes(body)
UTF8_BODY=utf8_fixture()
TEXT_PROFILES={'ascii':BODY,'utf8':UTF8_BODY}
def profile_body(name):
 if not isinstance(name,str)or name not in TEXT_PROFILES:raise ValueError('unknown source-defined text profile')
 return TEXT_PROFILES[name]
def fixture_response(body=BODY):
 if body not in TEXT_PROFILES.values():raise ValueError('response must use an exact source-defined text fixture')
 charset=b'utf-8'if body==UTF8_BODY else b'us-ascii'
 return b'HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset='+charset+b'\r\nContent-Length: 12288\r\nConnection: close\r\n\r\n'+body

def manifest_profile(manifest,requested=None):
 profile=manifest.get('text_profile')
 if not isinstance(profile,str)or profile not in TEXT_PROFILES or requested is not None and requested!=profile:raise ValueError('recorded/requested text profile differs')
 body=profile_body(profile)
 if manifest.get('body_sha256')!=sha(body)or manifest.get('body_bytes')!=len(body):raise ValueError('recorded text profile body differs')
 return profile,body
SCOPE=('Two actual boots of one sealed ISO: finite captured DNS/plain HTTP12288, real keyboard reverse/Shift selection and reflow, '
       'whole-document Browser Copy, Quil atomic selected replacement, SaveAs, exact independently decoded QJ02 bytes and cold user reopen. '
       'An explicit source-defined ASCII(default)/UTF8 profile binds both boots, raw bytes, scalar cells and frame-byte budgets. '
       'Whole source-bound Browser text/highlight cells and complete Quil admitted text are checked against actual PPM pixels. '
       'No public routing/TLS, process VAS, POSIX clipboard, crash/power-loss, physical hardware or SMP claim. '
       'Native capacity rejection, completed handoff surviving producer Close and empty post-retirement publication are included. '
       'Producer-close-before-Finish, cancellation and Full remain host model qualifications unless separately recorded.')


def sha(data):return hashlib.sha256(data).hexdigest()
def name_keyboard(name,quil_source):
 """Preflight every intended name byte against the actual unshifted editor.

 QMP qcodes name physical keys. Name mode deliberately calls the production
 scancode translator with shift=false, so unsupported glyphs must fail before
 a boot instead of silently saving a different name.
 """
 if not isinstance(name,str)or not name.isascii()or not name:raise ValueError('bounded ASCII document name required')
 limit=re.search(r'QUIL_NAME_LEN\s*<\s*(\d+)',quil_source)
 if not limit or len(name)>int(limit[1]):raise ValueError('document name exceeds actual name-mode bound')
 header='fn scancode_to_char(scancode: u64, shift: bool) -> Option<u8> {'
 if header not in quil_source or 'scancode_to_char(scancode, false)'not in quil_source:raise ValueError('actual unshifted filename keyboard source binding changed')
 body=quil_source.split(header,1)[1].split('\nfn ',1)[0]
 pattern=r"^\s*(0x[0-9A-Fa-f]+)\s*=>\s*Some\(if shift \{ b'(?:\\.|[^'\\\r\n])*' \} else \{ b'((?:\\.|[^'\\\r\n])*)' \}\),"
 decoded={int(code,16):ast.literal_eval("b'"+value+"'")for code,value in re.findall(pattern,body,re.M)}
 physical={code:char for base,row in((0x10,'qwertyuiop'),(0x1e,'asdfghjkl'),(0x2c,'zxcvbnm'),(0x02,'1234567890'))for code,char in enumerate(row,base)}
 physical.update({0x0c:'minus',0x0d:'equal',0x27:'semicolon',0x28:'apostrophe',0x33:'comma',0x34:'dot',0x35:'slash'})
 keys=[]
 for byte in name.encode('ascii'):
  candidates=[physical[code]for code,value in decoded.items()if value==bytes([byte])and code in physical]
  if byte in(32,10)or len(candidates)!=1:raise ValueError(f'unsupported actual filename keyboard byte={byte}')
  keys.append(candidates[0])
 return tuple(keys)


def source_contract(root,text_profile='ascii'):
 root=Path(root);c=http.load_pixel_contract(root);c['text_profile']=text_profile;c['body']=profile_body(text_profile)
 paths=('crates/sex-pdx/src/clipboard.rs','crates/sex-pdx/src/clipboard_client.rs','crates/sex-pdx/src/text_batch.rs',
        'servers/quil/src/main.rs','servers/quil/src/document_exchange.rs','servers/quil/src/view.rs',
        'servers/silk-shell/src/clipboard.rs','servers/silk-shell/src/linen_text_delivery.rs','servers/silk-shell/src/main.rs',
        'scripts/network_contention_gate.py',
        'scripts/clipboard_workflow_gate.py','scripts/tests/test_clipboard_workflow_gate.py',
        'scripts/check_document_disk.py','scripts/https_import_gate.py','scripts/check_quil_ui_pixels.py','crates/sex-pdx/src/text_encoding.rs','apps/kaleidoscope/src/document_reader.rs','scripts/check_desktop_pixels.py')
 source={p:(root/p).read_text()for p in paths}
 app=(root/'apps/kaleidoscope/src/main.rs').read_text();renderer=(root/'servers/sexdisplay/src/main.rs').read_text()
 if 'self.reader.render_path_selected(self.http.document(),self.http.document_path(),self.selection.range())'not in app or 'self.ui.queue_selected(&text.bytes[..text.len]'not in app or 'visible_selection=f.selection'not in app:raise ValueError('actual selected reader/outbox binding changed')
 if 'if sender != SHELL'not in source[paths[1]]or 'pub const SHELL:Subject=Subject{pd:3,task:3}'not in source[paths[0]]:raise ValueError('exact authenticated Shell PD/task client check changed')
 if 'Phase::Commit => {\n                if v as usize != self.len'not in source['servers/quil/src/document_exchange.rs']or 'Phase::End => {\n                if v != 0'not in source['servers/quil/src/document_exchange.rs']:raise ValueError('positive exact COMMIT/END source binding changed')
 labels={name:ast.literal_eval(value)for name,value in re.findall(r'clipboard_client::View::(\w+)=>(b"[^"\n]*")',app)}
 selected=re.search(r'hi>lo\)\{(b"[^"\n]*")\}',app)
 if not selected or not all(name in labels for name in('Copied','Pending')):raise ValueError('source-bound clipboard reader status labels changed')
 c['clipboard_labels']=labels;c['selection_label']=ast.literal_eval(selected[1])
 quil=source['servers/quil/src/main.rs']
 c['name_keys']=name_keyboard(NAME,quil)
 capacity=re.search(r'if reason=="capacity"\{(b"[^"\n]*")\}',quil)
 empty=re.search(r'View::Empty=>(b"[^"\n]*")',quil)
 if not capacity or not empty or 'CLIPBOARD.retire(subject);'not in source['servers/silk-shell/src/main.rs']:raise ValueError('negative UI/terminal publication source binding changed')
 c['negative_labels']=dict(Capacity=ast.literal_eval(capacity[1]).decode('ascii'),Empty=ast.literal_eval(empty[1]).decode('ascii'))
 bg=re.search(r'let background = if selected \{ Some\((0x[0-9a-f]+)\)',renderer)
 if not bg:raise ValueError('actual scalar selection background changed')
 c['selection_color']=int(bg[1],16)&0xffffff
 boundary=re.search(r'^const BAR_BG_H: usize = (\d+);$',renderer,re.M)
 if not boundary:raise ValueError('canonical display desktop boundary changed')
 c['desktop_top']=int(boundary[1])
 reader=source['apps/kaleidoscope/src/document_reader.rs']
 if text_profile=='utf8'and not all(x in reader for x in ('encoded_row_cost','source_geometry','text::cell(body,offset)','replace_footer','CAPACITY-(3*cols+2)')):raise ValueError('scalar reader byte-budget/source binding changed')
 c['source_sha256'].update({p:sha(value.encode())for p,value in source.items()})
 return c


def selected_frame_ascii(body,w,h,c,anchor,selection,label):
 """Independent raw-source→rendered-cell model, without invoking Rust state."""
 f=http.reader_frame(body,w,h,c,anchor,b'/welcome');lo,hi=sorted(selection)
 if not 0<=lo<=hi<=len(body):raise ValueError('invalid expected raw source selection')
 trimmed=body.rstrip(b' \n');first=last=None
 for visible,index in enumerate(range(f['first'],f['end'])):
  start=f['starts'][index];end=start
  while end<len(trimmed)and end-start<f['cols']and trimmed[end]!=10:end+=1
  a,b=max(lo,start),min(hi,end)
  if a<b:
   if first is None:first=(visible+2)*(f['cols']+1)+a-start
   last=(visible+2)*(f['cols']+1)+b-start
 f['selection']=(0,0)if first is None else(first,last)
 if label:f['lines'][-1]=label[:f['cols']].ljust(f['cols'],b' ')
 return f


def utf8_boundaries(body):
 text=body.decode('utf8',errors='strict')
 if any(ord(c)<32 and c not in '\t\n'or 127<=ord(c)<=159 for c in text):raise ValueError('expected fixture contains unsupported controls')
 return {0,len(body)}|{end for _,_,end in text_cells(body)}

def utf8_reader_frame(body,width,height,c,anchor=0,selection=(0,0),label=b''):
 """Independent scalar rows and serialized frame-byte accounting, UTF8 only."""
 boundaries=utf8_boundaries(body)
 if len(selection)!=2 or any(type(value)is not int for value in selection):raise ValueError('expected integer source selection endpoints')
 lo,hi=sorted(selection)
 if len(body)>c['reader_max_body']or type(anchor)is not int or anchor not in boundaries or lo not in boundaries or hi not in boundaries:raise ValueError('source anchor/selection is not a whole bounded scalar range')
 scale=2 if width>=c['scale_threshold']else 1;cols=min(c['reader_max_columns'],max(0,(width-2*c['inset'])//(c['advance']*scale)-1));available=height-c['top']-c['glyph_height']*scale
 base_rows=min(c['max_rows'],c['reader_capacity']//(cols+1),1+available//(c['line']*scale))if cols and available>=0 else 0
 if base_rows<4:raise ValueError('actual reader tile has no fully visible body row')
 trimmed=body.rstrip(b' \t\n');cells=text_cells(trimmed);wrapped=[];index=0
 while index<len(cells):
  start=cells[index][1];row=[]
  while index<len(cells)and len(row)<cols and cells[index][0]!='\n':row.append(cells[index]);index+=1
  if index<len(cells)and cells[index][0]=='\n':index+=1
  wrapped.append((start,row))
 def shown(char):return ' 'if char=='\t'else char
 largest=max([cols+1]+[cols+1+sum(len(shown(char).encode('utf8'))-1 for char,_,_ in row)for _,row in wrapped])
 visible=min(base_rows-3,(c['reader_capacity']-(3*cols+2))//largest);rows=visible+3
 if not visible:raise ValueError('source serialized scalar rows leave no complete body row')
 starts=[start for start,_ in wrapped];anchored=max((i for i,start in enumerate(starts)if start<=anchor),default=0);first=min(anchored,max(0,len(wrapped)-visible));end=min(first+visible,len(wrapped))
 host=re.search(br'\r\nHost: ([^\r\n]+)\r\n',c['request'])
 if not host:raise ValueError('missing actual fixed request hostname')
 path=http.reader_path(b'/welcome',c);progress=f'{first+1 if end>first else 0}-{end}/{len(wrapped)}'.encode()
 controls=next((pair[1]for pair in c['reader_controls']if len(pair[1])+1+len(progress)<=cols),None)
 footer=controls.ljust(cols-len(progress),b' ')+progress if controls is not None else c['reader_footers'][0 if cols>=25 else 1 if cols>=6 else 2]
 lines=[c['reader_header'][:cols].ljust(cols,b' '),(host[1]+b' '+path)[:cols].ljust(cols,b' ')];selected_first=selected_last=None
 for visible_row in range(visible):
  row=wrapped[first+visible_row][1]if first+visible_row<len(wrapped)else[];encoded=bytearray();row_at=sum(len(x)+1 for x in lines)
  for char,start,stop in row:
   value=shown(char).encode('utf8')
   if lo<hi and lo<=start and stop<=hi:
    if selected_first is None:selected_first=row_at+len(encoded)
    selected_last=row_at+len(encoded)+len(value)
   encoded.extend(value)
  encoded.extend(b' '*(cols-len(row)));lines.append(bytes(encoded))
 if label:footer=label
 lines.append(footer[:cols].ljust(cols,b' '));offsets=[];at=0
 for line in lines:offsets.append(at);at+=len(line)+1
 size=at-1
 if size>c['reader_capacity']:raise ValueError('independent scalar frame exceeds actual byte transaction')
 return dict(path=path.decode(),cols=cols,rows=rows,body_rows=visible,first=first,end=end,total_lines=len(wrapped),source_bytes=len(body),scale=scale,anchor=anchor,source_anchor=anchor,first_byte=starts[first]if starts else 0,progress=progress.decode()if controls is not None else None,starts=starts,lines=lines,row_offsets=offsets,bytes=size,selection=(0,0)if selected_first is None else(selected_first,selected_last),max_row_cost=largest)

def selected_frame(body,w,h,c,anchor,selection,label):
 return utf8_reader_frame(body,w,h,c,anchor,selection,label)if c.get('text_profile','ascii')=='utf8'else selected_frame_ascii(body,w,h,c,anchor,selection,label)

def reader_geometry(trace,body,c,anchor=0):
 if c.get('text_profile','ascii')=='ascii':return http.reader_geometry(trace,body,c,anchor,b'/welcome')
 queued=re.findall(r'\[shell.document.viewport\] target=15 task=15 surface=300 sequence=(\d+) x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) enqueued=1',trace);notices=re.findall(r'\[browser.viewport\] target=15 task=15 sequence=(\d+) w=(\d+) h=(\d+) origin=held_shell',trace)
 if not queued or not notices:raise ValueError('missing exact scalar reader viewport')
 sequence,nw,nh=map(int,notices[-1]);matching=[tuple(map(int,row))for row in queued if int(row[0])==sequence]
 if len(matching)!=1:raise ValueError('scalar viewport lacks exact queued geometry')
 _,x,y,w,h=matching[0]
 if(w,h)!=(nw,nh):raise ValueError('scalar queued/accepted viewport differs')
 path,ticket=http.released_reader_document(trace,body,c,b'/welcome');f=utf8_reader_frame(body,w,h,c,anchor);f.update(tile=dict(x=x,y=y,w=w,h=h),viewport=sequence,document_ticket=ticket);return f

def verify_browser(trace,ppm,body,c,anchor,selection,status='Selected',resolution=None):
 try:
  current=reader_geometry(trace,body,c,anchor);tile=current['tile']
  label=c['selection_label']if status=='Selected'else c['clipboard_labels'][status]if status else b''
  f=selected_frame(body,tile['w'],tile['h'],c,anchor,selection,label)
  f.update(tile=tile,viewport=current['viewport'])
  layouts=list(re.finditer(r'\[browser.reader.layout\] first=(\d+) end=(\d+) body_rows=(\d+) rows=(\d+) cols=(\d+) total_lines=(\d+) source_bytes=(\d+) viewport=(\d+) origin=validated_body tls=0 pixels=unconfirmed',trace))
  wanted=tuple(f[k]for k in('first','end','body_rows','rows','cols','total_lines','source_bytes','viewport'))
  if not layouts or tuple(map(int,layouts[-1].groups()))!=wanted:raise ValueError('current reader layout differs from independent source/caret model')
  http.reader_position(trace,layouts[-1].end(),f,0,True)
  if not re.search(rf'\[browser.view.applied\] bytes={f["bytes"]} http=258 body_bytes={len(body)} pixels=unconfirmed',trace[layouts[-1].end():]):raise ValueError('current atomic reader Applied missing')
  width,height,pixels=ppm_rgb(ppm,min_width=1,min_height=1)
  if resolution and(width,height)!=tuple(resolution):raise ValueError('actual framebuffer resolution differs')
  if tile['x']<0 or tile['y']<c['desktop_top'] or tile['x']+tile['w']>width or tile['y']+tile['h']>height:raise ValueError('actual reader tile outside framebuffer')
  factor=f['scale'];fg=c['color'].to_bytes(3,'big');bg=c['selection_color'].to_bytes(3,'big');a,b=f['selection'];mismatch=ink=highlight=checked=0
  columns=min(80,max(0,(tile['w']-2*c['inset'])//(c['advance']*factor)))
  rows=min(c['max_rows'],max(0,(tile['h']-c['top']+c['line']*factor-1)//(c['line']*factor)))
  for row in range(rows):
   line=f['lines'][row]if row<f['rows']else b''
   row_cells=text_cells(line);row_start=f.get('row_offsets',[i*(f['cols']+1)for i in range(f['rows'])])[row]if row<f['rows']else 0
   for col in range(columns):
    char,start,stop=row_cells[col]if col<len(row_cells)else(' ',len(line),len(line));bitmap=glyph(c['font'],char);index=row_start+start
    selected=row<f['rows']and col<f['cols']and col<len(row_cells)and a<=index<row_start+stop<=b
    for gy in range(c['line']):
     for gx in range(c['advance']):
      on=gy<c['glyph_height']and gx<5 and bool(bitmap[gy]&(1<<(4-gx)))
      for dy in range(factor):
       y=tile['y']+c['top']+(row*c['line']+gy)*factor+dy
       if y>=tile['y']+tile['h']:continue
       for dx in range(factor):
        x=tile['x']+c['inset']+(col*c['advance']+gx)*factor+dx
        if x>=tile['x']+tile['w']:continue
        value=pixels[(y*width+x)*3:(y*width+x)*3+3];checked+=1;ink+=on;highlight+=selected and not on
        mismatch+=(value!=fg)if on else(value!=bg)if selected else(value in(fg,bg))
  if mismatch:raise ValueError(f'whole Browser text/highlight/stale-band pixel mismatches={mismatch}')
  if not ink or a!=b and not highlight:raise ValueError('missing checked text/highlight pixels')
  return dict(ok=True,errors=[],checked_pixels=checked,highlight_pixels=highlight,selection=list(f['selection']),source_selection=list(selection),expected_rows=[line.decode('utf8')for line in f['lines']],geometry={k:v for k,v in f.items()if k not in('starts','lines')})
 except(ValueError,KeyError,IndexError,OverflowError,UnicodeError)as exc:return dict(ok=False,errors=[str(exc)])


def reverse_selection(body):
 boundaries=[0]+[end for _,_,end in text_cells(body)]
 if len(boundaries)<9:raise ValueError('fixture lacks explicit finite movement prefix')
 return boundaries[3],boundaries[5]

def complete_prefix(raw):
 """Accept a concurrently written UTF-8 file only through its last full scalar.

 Raw evidence is never changed. Only an otherwise valid incomplete trailing
 scalar (at most three bytes) can wait for the writer; malformed bytes fail.
 """
 try:raw.decode('utf-8',errors='strict');return raw
 except UnicodeDecodeError as exc:
  if exc.reason!='unexpected end of data' or exc.end!=len(raw) or not 1<=len(raw)-exc.start<=3:raise
  prefix=raw[:exc.start];prefix.decode('utf-8',errors='strict');return prefix


def failure_summary(exc):
 """Bound diagnostics without exposing UnicodeDecodeError's whole raw object."""
 if isinstance(exc,UnicodeDecodeError):
  return f'UnicodeDecodeError: encoding={exc.encoding} reason={exc.reason} start={exc.start} end={exc.end} input_bytes={len(exc.object)}'
 detail=str(exc).replace('\r','\\r').replace('\n','\\n')
 return type(exc).__name__+': '+detail[:512]+(' [truncated]'if len(detail)>512 else'')


def hardware_keys(q,keys):
 """One explicit hardware down/up packet; never QMP's deferred hold timer.

 Preserve modifier release before the next action. The caller waits for actual
 Shell input admission when exact per-key Browser movement is required.
 """
 keys=tuple(keys)
 if not 1<=len(keys)<=3 or len(set(keys))!=len(keys)or any(not isinstance(key,str)or not key for key in keys):raise ValueError('one bounded key/chord required')
 events=[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':code}}}for down,codes in((True,keys),(False,reversed(keys)))for code in codes]
 q.cmd('input-send-event',{'events':events})


def browser_movement(q,keys,text,wait,record):
 """One movement, followed by a fresh actual Shell endpoint admission barrier."""
 keys=tuple(keys)
 if keys not in(('right',),('left',),('shift','right'),('shift','left')):raise ValueError('bounded Browser horizontal qualification movement required')
 start=len(text());record(keys);hardware_keys(q,keys)
 code=106 if keys[-1]=='right'else 105
 return wait(rf'\[shell.browser.input\] code={code} down=1 slot=43 accepted=1',start)


def qualified_key(q,code,pattern,text,wait,record):
 """Desktop focus traversal qualifies real Alt+Tab, with exact action records."""
 start=len(text());keys=('alt','tab')if code=='tab'else(code,)
 record(keys);hardware_keys(q,keys);return wait(pattern,start)


def browser_viewport_sequence(trace):
 notices=list(re.finditer(r'\[browser.viewport\] target=15 task=15 sequence=(\d+) w=\d+ h=\d+ origin=held_shell',trace))
 if not notices:raise ValueError('missing current accepted Browser viewport')
 return int(notices[-1][1])


def viewport_barrier(trace,previous,after,contract,anchor=0,body=None):
 """Fresh accepted geometry, matching source layout and its positive Applied.

 Completion is a paint-state barrier. The following screenshot still has to
 independently prove all source-bound text and current selection pixels.
 """
 if body is None:body=contract.get('body',BODY)
 frame=reader_geometry(trace,body,contract,anchor);sequence=frame['viewport']
 if sequence<=previous:raise ValueError('waiting for a strictly newer accepted viewport')
 notices=list(re.finditer(rf'\[browser.viewport\] target=15 task=15 sequence={sequence} w=(\d+) h=(\d+) origin=held_shell',trace))
 if len(notices)!=1 or notices[0].start()<after:raise ValueError('new viewport did not follow actual Zoom action')
 notice=notices[0]
 zoom=re.search(r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1',trace[after:notice.start()])
 if not zoom:raise ValueError('fresh viewport lacks actual prior successful Zoom action')
 queued=list(re.finditer(rf'\[shell.document.viewport\] target=15 task=15 surface=300 sequence={sequence} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) enqueued=1',trace))
 if len(queued)!=1 or queued[0].start()>=notice.start()or tuple(map(int,queued[0].groups()[2:]))!=tuple(map(int,notice.groups())):raise ValueError('new viewport lacks exact prior Shell geometry')
 layouts=list(re.finditer(r'\[browser.reader.layout\] first=(\d+) end=(\d+) body_rows=(\d+) rows=(\d+) cols=(\d+) total_lines=(\d+) source_bytes=(\d+) viewport=(\d+) origin=validated_body tls=0 pixels=unconfirmed',trace))
 wanted=tuple(frame[k]for k in('first','end','body_rows','rows','cols','total_lines','source_bytes','viewport'))
 if not layouts or layouts[-1].start()<=notice.end()or tuple(map(int,layouts[-1].groups()))!=wanted:raise ValueError('new viewport lacks matching current source layout')
 layout=layouts[-1];http.reader_position(trace,layout.end(),frame,0,True)
 applied=re.search(rf'\[browser.view.applied\] bytes={frame["bytes"]} http=258 body_bytes=12288 pixels=unconfirmed',trace[layout.end():])
 if not applied:raise ValueError('new viewport layout lacks subsequent positive Applied')
 return dict(previous=previous,sequence=sequence,zoom_offset=after+zoom.start(),queued_offset=queued[0].start(),notice_offset=notice.start(),layout_offset=layout.start(),applied_offset=layout.end()+applied.start(),tile=frame['tile'])


def capture_prefix(raw,frame):
 size=frame.get('serial_bytes');digest=frame.get('serial_sha256')
 if type(size)is not int or not 0<size<=len(raw)or not isinstance(digest,str)or sha(raw[:size])!=digest:raise ValueError('immutable raw serial prefix missing or changed')
 return raw[:size].decode('utf-8',errors='strict')


def clipboard_join(trace,body=BODY):
 """Each exact transfer joins actual producer, publication and applied consumer."""
 value=hex(fnv(body));copy=list(re.finditer(r'\[browser.clipboard.copy\] ticket=(\d+) generation=(\d+) revision=(\d+) start=(\d+) end=(\d+) bytes=(\d+) hash=(0x[0-9a-f]+) accepted=1',trace))
 if len(copy)!=1:raise ValueError('missing/duplicate accepted actual Browser copy')
 cp=copy[0];ticket,generation,revision,start,end,length=map(int,cp.groups()[:6])
 if(ticket==0 or generation==0 or(start,end,length,cp[7])!=(0,len(body),len(body),value)):raise ValueError('Browser copied foreign/truncated source bytes')
 published=list(re.finditer(r'\[browser.clipboard.published\] ticket=(\d+) bytes=(\d+) generation=(\d+) hash=(0x[0-9a-f]+) positive_ack=1',trace))
 if len(published)!=1:raise ValueError('missing/duplicate positive publication')
 pub=published[0];pt,pl,pg=map(int,pub.groups()[:3])
 if(pt,pl,pub[4])!=(ticket,len(body),value)or pg==0 or pub.start()<=cp.end():raise ValueError('publication lacks exact prior whole-body copy')
 paste=list(re.finditer(r'\[quil.clipboard.paste\] ticket=(\d+) bytes=(\d+) generation=(\d+) hash=(0x[0-9a-f]+) applied=1 atomic=1(?: reason=\w+)?',trace))
 if len(paste)!=1:raise ValueError('missing/duplicate actual atomic Quil replacement')
 dest=paste[0];dt,dl,dg=map(int,dest.groups()[:3])
 if dt<=ticket or(dl,dg,dest[4])!=(len(body),pg,value)or dest.start()<=pub.end():raise ValueError('paste is foreign, partial or precedes publication')
 for pd,t,opcode in((15,ticket,0x281),(15,ticket,0x28a),(9,dt,0x283),(9,dt,0x285)):
  if not re.search(rf'\[shell.clipboard.request\] pd={pd} task={pd} ticket={t} opcode={hex(opcode)} result=Ok\(\(\)\)',trace):raise ValueError('missing actual authenticated subject broker admission')
 intent=re.search(rf'\[quil.clipboard.intent\] ticket={dt} action=paste generation=\d+ revision=\d+ start=(\d+) end=(\d+)',trace)
 selections=list(re.finditer(r'\[quil.selection\] generation=(\d+) revision=(\d+) bytes=(\d+) anchor=(\d+) caret=(\d+)',trace[:intent.start()if intent else 0]))
 if not intent or not selections:raise ValueError('paste lacks exact selected replacement snapshot')
 switches=list(re.finditer(r'\[shell.app.switch\] old=\d+ new=201 live=1 shortcut=alt_tab',trace[pub.end():intent.start()]))
 if not switches:raise ValueError('cross-app paste lacks actual live Alt+Tab switch to Quil after publication')
 switch_offset=pub.end()+switches[-1].start()
 old=selections[-1];oldlen,anchor,caret=map(int,old.groups()[2:])
 if oldlen==0 or sorted((anchor,caret))!=[0,oldlen]or tuple(map(int,intent.groups()))!=(0,oldlen):raise ValueError('Quil paste did not atomically replace whole existing buffer')
 return dict(producer=dict(pd=15,task=15,ticket=ticket,source_generation=generation,revision=revision),consumer=dict(pd=9,task=9,ticket=dt,replaced_bytes=oldlen),publication_generation=pg,alt_tab_offset=switch_offset,bytes=len(body),hash=value,copy_offset=cp.start(),published_offset=pub.start(),paste_offset=dest.start(),receiver_shell_subject=dict(pd=3,task=3,proof='positive workflow plus sealed exact Client::receive sender check'))


def unchanged_quil(before,after):
 """Status can change; admitted source rows, caret and viewport cannot."""
 fields=('x','y','w','h','top','cols','rows','cursor','view')
 if any(before[k]!=after[k]for k in fields)or before['text'].split('\n')[1:]!=after['text'].split('\n')[1:]:raise ValueError('rejected paste changed visible body, caret or viewport')
 return dict(ok=True,cursor=after['cursor'],body_text_sha256=sha('\n'.join(after['text'].split('\n')[1:]).encode()),scope='Complete current admitted body rows/caret; subsequent exact raw Save independently proves the whole12288-byte document')


def unchanged_document(before,after,document,body=BODY):
 a,ia=storage.object_bytes(before,document['object']);b,ib=storage.object_bytes(after,document['object'])
 old=[r for r in storage.workspace(before)['rows']if r['object']==document['object']]
 new=[r for r in storage.workspace(after)['rows']if r['object']==document['object']]
 if a!=b or ia!=ib or ia!=document['identity']or old!=new or recover(after,document['object'])['payload']!=body:raise ValueError('rejected/empty paste rewrote durable document or its identity')
 return dict(ok=True,object=document['object'],identity=ia,raw_object_sha256=sha(a))


def negative_joins(trace,positive,body=BODY):
 value=hex(fnv(body));pub=positive['publication_generation']
 failures=list(re.finditer(r'\[quil.clipboard.paste\] ticket=(\d+) bytes=12288 generation=(\d+) hash=(0x[0-9a-f]+) applied=0 atomic=1 reason=capacity',trace))
 if len(failures)!=1:raise ValueError('missing/duplicate actual capacity rejection')
 fail=failures[0];ticket,generation=int(fail[1]),int(fail[2])
 if ticket<=positive['consumer']['ticket']or generation!=pub or fail[3]!=value or fail.start()<=positive['paste_offset']:raise ValueError('capacity rejection is stale, partial or foreign')
 intent=re.search(rf'\[quil.clipboard.intent\] ticket={ticket} action=paste generation=(\d+) revision=(\d+) start=12288 end=12288',trace[:fail.start()])
 if not intent:raise ValueError('capacity probe lacks empty EOF destination snapshot')
 selections=list(re.finditer(r'\[quil.selection\] generation=(\d+) revision=(\d+) bytes=12288 anchor=12288 caret=12288',trace[:intent.start()]))
 if not selections or selections[-1].groups()!=intent.groups():raise ValueError('capacity probe lacks exact current full-buffer caret/source')
 for opcode in(0x283,0x285):
  if not re.search(rf'\[shell.clipboard.request\] pd=9 task=9 ticket={ticket} opcode={hex(opcode)} result=Ok\(\(\)\)',trace[intent.end():fail.start()]):raise ValueError('capacity payload lacks completed authorized read')
 close=re.search(r'\[shell.app.close.request\] target=15 task=15 surface=300',trace[fail.end():])
 if not close:raise ValueError('actual producer Close missing after capacity check')
 start=fail.end()+close.start();suffix=trace[start:]
 if not re.search(r'\[browser.import.closed\] target=15 task=15 pending=0 accepted=0 terminal=1 owned_storage=erased',suffix):raise ValueError('producer did not terminate exact idle incarnation')
 shell=re.search(r'\[shell.app.cleanup.ack\] target=15 task=15 surface=300 ok=1',suffix)
 pool=re.search(r'\[sext.app.pool.ready\] target=15 task=15 bytes=(\d+) zeroed=(\d+) workspace_records=(\d+) reused=1 zero_owner=sext ok=1',suffix)
 if not shell or not pool or shell.start()>=pool.start()or int(pool[1])==0 or pool[1]!=pool[2]:raise ValueError('producer lacks exact Shell ACK followed by complete pool zeroing')
 cleanup=cleanup_receipts(trace,15,15,start)
 empty=list(re.finditer(r'\[quil.clipboard.stopped\] ticket=(\d+) state=Empty payload_erased=1',trace[start+pool.end():]))
 if len(empty)!=1 or int(empty[0][1])<=ticket:raise ValueError('new consumer paste lacks empty post-retirement publication')
 empty_offset=start+pool.end()+empty[0].start();empty_ticket=int(empty[0][1])
 empty_intent=re.search(rf'\[quil.clipboard.intent\] ticket={empty_ticket} action=paste generation=\d+ revision=\d+ start=12288 end=12288',trace[start+pool.end():empty_offset])
 if not empty_intent or not re.search(rf'\[shell.clipboard.request\] pd=9 task=9 ticket={empty_ticket} opcode=0x283 result=Ok\(\(\)\)',trace[start+pool.end():empty_offset]):raise ValueError('empty result lacks new exact consumer intent/read')
 if re.search(r'\[quil.document.edit\]',trace[positive['paste_offset']:empty_offset]):raise ValueError('rejected/empty paste changed document revision')
 if re.search(rf'\[quil.clipboard.paste\] ticket={empty_ticket} ',trace)or re.search(rf'\[shell.clipboard.request\] pd=9 task=9 ticket={empty_ticket} opcode=0x285',trace):raise ValueError('empty publication fabricated a completed payload')
 return dict(ok=True,capacity=dict(ticket=ticket,bytes=12288,source_generation=int(intent[1]),revision=int(intent[2]),offset=fail.start()),producer_close=dict(start=start,shell_ack=start+shell.start(),pool_ready=start+pool.start(),zeroed_bytes=int(pool[2]),cleanup=cleanup),empty=dict(ticket=empty_ticket,offset=empty_offset),scope='Actual second authorized full read rejected atomically at EOF; completed receiver survives exact producer retirement; new read returns Empty after publication custody cleanup')


class Peer:
 def __init__(self,request,recovery=False,body=BODY):self.request=request;self.recovery=recovery;self.body=body;self.stop=threading.Event();self.errors=[];self.rows=[];self.dns=[];self.attempts=[];self.sockets=[];self.threads=[];self.live=[]
 def start(self):
  tcp=socket.socket();self.sockets=[tcp];tcp.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);tcp.bind(('127.0.0.1',46078));tcp.listen(3);tcp.settimeout(.2)
  udp=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);self.sockets.append(udp)
  try:udp.bind(('127.0.0.1',46079))
  except Exception:self.close();raise
  udp.settimeout(.2)
  def dns():
   while not self.stop.is_set():
    try:data,addr=udp.recvfrom(513)
    except socket.timeout:continue
    if self.recovery or self.dns or data!=http.dns_query(1):raise ValueError('unexpected actual DNS request')
    reply=http.dns_response(1);sent=udp.sendto(reply,addr);row=dict(ticket=1,response_hex=reply.hex());self.dns.append(row);self.attempts.append(dict(row,kind='answer',sent_bytes=sent))
  def serve():
   for ticket in range(1,2 if self.recovery else 3):
    while not self.stop.is_set():
     try:conn,_=tcp.accept();break
     except socket.timeout:continue
    else:return
    self.live.append(conn);conn.settimeout(45);data=bytearray()
    while True:
     part=conn.recv(4096)
     if not part:break
     data.extend(part)
     if len(data)>4096:raise ValueError('actual guest request exceeds finite endpoint bound')
    expected=http.ECHO if ticket==1 else self.request
    if bytes(data)!=expected:raise ValueError('actual guest request differs')
    response=http.ECHO if ticket==1 else fixture_response(self.body)
    for start in range(0,len(response),1024):conn.sendall(response[start:start+1024])
    self.rows.append(dict(ticket=ticket,request_hex=bytes(data).hex(),response_hex=response.hex()));conn.shutdown(socket.SHUT_WR);conn.close()
   while not self.stop.is_set():
    try:conn,_=tcp.accept();conn.close();raise ValueError('unrequested actual TCP flow')
    except socket.timeout:pass
  def guarded(fn):
   try:fn()
   except Exception as exc:
    if not self.stop.is_set():self.errors.append(failure_summary(exc))
  for fn in(dns,serve):thread=threading.Thread(target=guarded,args=(fn,),daemon=True);thread.start();self.threads.append(thread)
 def close(self):
  self.stop.set()
  for sock in self.sockets+self.live:
   try:sock.close()
   except OSError:pass
  for thread in self.threads:thread.join(timeout=1)


def wire_check(raw,peer,recovery=False,body=BODY):
 flows,packets=reassemble(raw);flows=sorted(flows,key=lambda f:f['guest_port']);wanted=1 if recovery else 2
 if len(flows)!=wanted or len(peer['rows'])!=wanted:raise ValueError('actual captured TCP flow count differs')
 for index,(flow,row)in enumerate(zip(flows,peer['rows']),1):
  response=http.ECHO if index==1 else fixture_response(body);request=http.ECHO if index==1 else http.load_pixel_contract(Path(__file__).resolve().parents[1])['request']
  if(flow['peer_port'],flow['guest_port'],flow['outgoing'],flow['incoming'])!=(46078,48000+index,request,response)or row['request_hex']!=request.hex()or row['response_hex']!=response.hex():raise ValueError('captured full HTTP/Echo bytes differ from independent endpoint profile')
  if not(flow['fin_in']and flow['fin_out'])or flow['rst_in']or flow['rst_out']:raise ValueError('actual flow lacks complete clean transfer')
 dns=dict(ok=True,errors=[])if recovery else http.dns_capture(raw,'echo',attempts=peer['attempts'],expected=peer['dns'])
 if not dns['ok']:raise ValueError(str(dns['errors']))
 return dict(ok=True,flows=wanted,tcp_packets=packets,dns=dns,body_sha256=sha(body),body_bytes=len(body),scope='Independent checksum/handshake/TCP reassembly plus exact complete endpoint bytes; host send/serial alone cannot pass')


def disk_check(before,after,body=BODY):
 result=storage.import_disk(before,after,NAME,body)
 return result


def disk_bytes(path):
 p=path/'nvme.img';return p.read_bytes()if p.exists()else gzip.decompress((path/'nvme.img.gz').read_bytes())


def replay_boot(out,contract):
 manifest=json.loads((out/'manifest.json').read_text());profile,body=manifest_profile(manifest,contract.get('text_profile','ascii'))
 if manifest.get('error')or manifest.get('source_sha256')!=contract['source_sha256']:raise ValueError('failed native run or changed sealed source/checker contract')
 root=Path(__file__).resolve().parents[1]
 actual_binding=frozen_binding(root,Path(manifest['iso_path']),Path(manifest['binding']['path']))
 if actual_binding!=manifest['binding']:raise ValueError('replayed ISO/source/runner binding differs')
 raw=(out/'serial.log').read_bytes();trace=complete_prefix(raw).decode('utf-8',errors='strict');resolution=tuple(map(int,manifest['resolution'].split('x')));checks=[]
 for row in json.loads((out/'frames.json').read_text()):
  image=(out/row['screenshot']).read_bytes()
  if sha(image)!=row['screenshot_sha256']:raise ValueError('actual immutable screenshot changed')
  prefix=capture_prefix(raw,row)
  if row['kind']=='browser':
   check=verify_browser(prefix,image,body,contract,row['anchor'],tuple(row['source_selection']),row['status'],resolution)
   if check['ok']and check['geometry']['viewport']<row.get('minimum_viewport',0):raise ValueError('screenshot predates its explicit new viewport barrier')
  else:
   errors=quil_pixels(prefix,image,contract['font'],target=9,resolution=resolution);v=quil_view(prefix)
   if not body[-28:].decode()in ''.join(v['text'].split('\n')):errors.append('whole-body unique tail missing from actual Quil view')
   if row.get('notice')and contract['negative_labels'][row['notice']]not in v['text'].split('\n')[0]:errors.append('source-bound negative status not in current actual Quil header')
   check=dict(ok=not errors,errors=errors,geometry=v)
  if not check['ok']:raise ValueError('pixel replay: '+str(check['errors']))
  checks.append(dict(screenshot=row['screenshot'],**check))
 if not checks or manifest['recovery']and not any(c['screenshot'].startswith('quil-cold')for c in checks):raise ValueError('missing required actual frames')
 if not manifest['recovery']and not all(any(c['screenshot'].startswith(name)for c in checks)for name in('browser-reverse','browser-reflow','browser-all','browser-copied','quil-pasted','quil-capacity-rejected','quil-after-producer-close','quil-empty-after-producer-close','quil-reopened')):raise ValueError('missing complete selection/transfer/reopen frame set')
 if not manifest['recovery']:
  browser={row['screenshot']:row['geometry']for row in checks if row['screenshot'].startswith('browser-')}
  original=browser['browser-reverse.ppm']['viewport'];reflow=browser['browser-reflow.ppm']['viewport']
  if reflow<=original or browser['browser-all.ppm']['viewport']<=reflow:raise ValueError('reflow/restore screenshots did not prove separate strictly newer accepted viewports')
 peer=json.loads((out/'peer-exchanges.json').read_text())
 if peer.get('errors'):raise ValueError('actual finite endpoint failed')
 wire=wire_check((out/'wire.pcap').read_bytes(),peer,manifest['recovery'],body);disk=disk_bytes(out)
 if sha(disk)!=manifest['final_disk_sha256']:raise ValueError('terminal raw disk changed')
 if manifest['recovery']:
  document=manifest['expected_document'];actual=recover(disk,document['object'])
  if actual['payload']!=body or actual['identity']!=document['identity']:raise ValueError('cold raw document bytes/identity differ')
  join=None;negative=None
 else:
  document=disk_check(gzip.decompress((out/'before-save.img.gz').read_bytes()),disk,body);join=clipboard_join(trace,body);negative=negative_joins(trace,join,body)
  frames={row['screenshot']:row for row in checks}
  negative['capacity_pixels']=unchanged_quil(frames['quil-pasted.ppm']['geometry'],frames['quil-capacity-rejected.ppm']['geometry'])
  negative['empty_pixels']=unchanged_quil(frames['quil-after-producer-close.ppm']['geometry'],frames['quil-empty-after-producer-close.ppm']['geometry'])
  negative['durable_unchanged']=unchanged_document(gzip.decompress((out/'before-producer-close.img.gz').read_bytes()),disk,document,body)
  saved=re.search(rf'\[quil.persist.save.ok\] bytes=12288 hash={hex(fnv(body))} doc={document["object"]}(?:\s|$)',trace[join['paste_offset']:])
  if not saved:raise ValueError('missing exact positive Save receipt after COMMIT+END')
  if join['paste_offset']+saved.start()<=negative['capacity']['offset']or join['paste_offset']+saved.end()>=negative['producer_close']['start']:raise ValueError('exact Save must follow capacity rejection and precede producer Close')
  saved_end=join['paste_offset']+saved.end()
  if not re.search(r'\[quil.document.saved\] saved_revision=(\d+) revision=\1 dirty=0',trace[saved_end:]):raise ValueError('saved snapshot differs from current editor revision')
  if not re.search(r'\[quil.name.mode\] on=1 save_as=1',trace[join['paste_offset']:saved_end]):raise ValueError('new durable object lacks actual SaveAs interaction')
 if not re.search(rf'\[quil.persist.load.ok\] bytes=12288 hash={hex(fnv(body))} doc={document["object"]}(?:\s|$)',trace):raise ValueError('missing positive exact user reopen')
 return dict(ok=True,text_profile=profile,body_sha256=sha(body),pixels=checks,wire=wire,clipboard=join,negative=negative,document=document,scope=SCOPE)


def run_boot(root,iso,binding,seed,out,contract,model,resolution_text,recovery=False,expected=None):
 out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise ValueError('fresh clipboard boot evidence directory required')
 body=contract.get('body',BODY);profile=contract.get('text_profile','ascii')
 bound=frozen_binding(root,iso,binding);held=json.loads(binding.read_text())
 for name,digest in contract['source_sha256'].items():
  if held.get('sources',{}).get(name,held.get('runner_sha256',{}).get(name))!=digest:raise ValueError('clipboard source/checker absent from sealed binding: '+name)
 version=subprocess.check_output(['qemu-system-x86_64','--version'],text=True)
 peer=Peer(contract['request'],recovery,body);disk=out/'nvme.img';shutil.copyfile(seed,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-clipboard-{os.getpid()}.sock';resolution=tuple(map(int,resolution_text.split('x')));command=desktop.native_command(iso,disk,serial,sock,pcap,model,resolution_text)
 manifest=dict(text_profile=profile,body_sha256=sha(body),body_bytes=len(body),binding=bound,iso_path=str(iso),command=command,resolution=resolution_text,recovery=recovery,expected_document=expected,input_disk_sha256=sha(seed.read_bytes()),source_sha256=contract['source_sha256'],qemu_version=version,scope=SCOPE);(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
 peer.start()
 try:proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT)
 except Exception:peer.close();raise
 started=time.monotonic();frames=[];actions=[];failure=None;browser_closed=False;reader_min_viewport=0
 def raw():return serial.read_bytes()if serial.exists()else b''
 def text():return complete_prefix(raw()).decode('utf-8',errors='strict')
 def health():
  if time.monotonic()-started>600:raise TimeoutError('bounded600second clipboard workflow')
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if peer.errors:raise RuntimeError(str(peer.errors))
  if any(p.exists()and p.stat().st_size>MAX_PCAP for p in(serial,pcap)):raise ValueError('bounded clipboard evidence exceeded')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',text()):raise ValueError('guest failure')
 def wait(pattern,start=0,timeout=90):
  end=time.monotonic()+timeout
  while time.monotonic()<end:
   health();match=re.search(pattern,text()[start:])
   if match:return match
   time.sleep(.01)
  raise TimeoutError(pattern)
 def key(code,pattern):
  return qualified_key(q,code,pattern,text,wait,lambda keys:actions.append(dict(keys=list(keys),serial_start=len(raw()))))
 def chord(*keys):
  actions.append(dict(keys=list(keys),serial_start=len(raw())));hardware_keys(q,keys)
 def press(code):chord(code)
 def movement(*keys):
  return browser_movement(q,keys,text,wait,lambda keys:actions.append(dict(keys=list(keys),serial_start=len(raw()),admission_barrier=True)))
 def zoom():
  nonlocal reader_min_viewport
  previous=browser_viewport_sequence(text())
  key('grave_accent',r'\[shell.palette.item\] idx=0')
  for old,new in((0,9),(9,8)):key('k',rf'\[shell.palette.select\] old={old} new={new}')
  after=len(text());key('ret',r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1')
  deadline=time.monotonic()+90;last='no fresh viewport'
  while time.monotonic()<deadline:
   health()
   try:receipt=viewport_barrier(text(),previous,after,contract,body=body);break
   except ValueError as exc:last=str(exc);time.sleep(.02)
  else:raise TimeoutError('new Zoom viewport/layout/Applied: '+last)
  reader_min_viewport=receipt['sequence'];actions.append(dict(action='viewport_barrier',**receipt))
 def picture(name,kind='browser',selection=(0,0),status='',anchor=0,notice=None):
  path=out/(name+'.ppm');deadline=time.monotonic()+30;check=dict(ok=False,errors=['no checked frame'])
  while time.monotonic()<deadline:
   health();q.cmd('screendump',{'filename':str(path)});snapshot=complete_prefix(raw());trace=snapshot.decode('utf-8',errors='strict');image=path.read_bytes()
   if kind=='browser':
    check=verify_browser(trace,image,body,contract,anchor,selection,status,resolution)
    if check['ok']and check['geometry']['viewport']<reader_min_viewport:check=dict(ok=False,errors=['image predates new viewport barrier'])
   else:
    errors=quil_pixels(trace,image,contract['font'],target=9,resolution=resolution)
    try:
     v=quil_view(trace)
     if body[-28:].decode()not in ''.join(v['text'].split('\n')):errors.append('actual complete body tail not visible')
     if notice and contract['negative_labels'][notice]not in v['text'].split('\n')[0]:errors.append('source-bound negative status not in current actual Quil header')
    except ValueError as exc:errors.append(str(exc))
    check=dict(ok=not errors,errors=errors)
   if check['ok']:break
   time.sleep(.05)
  row=dict(kind=kind,screenshot=path.name,screenshot_sha256=sha(path.read_bytes()),serial_bytes=len(snapshot),serial_sha256=sha(snapshot),anchor=anchor,source_selection=list(selection),status=status,notice=notice,minimum_viewport=reader_min_viewport,result=check);frames.append(row);(out/'frames.json').write_text(json.dumps(frames,indent=2)+'\n')
  if not check['ok']:raise RuntimeError(str(check['errors']))
 def open_linen():
  if browser_closed:
   event=key('f8',r'\[(?:shell.linen.toggle.minimize|linen.open.nonblocking)\]')
   if 'shell.linen.toggle.minimize'in event[0]:key('f8',r'\[linen.open.nonblocking\]')
  else:focus_surface(300,text,key);key('o',r'\[shell.document.open\] target=15 linen=200')
  focus_surface(200,text,key)
 def reopen(object_id):
  open_linen()
  for _ in range(40):
   try:
    v,labels=final_view(text())
    if any(name==NAME and identity==v['selected']for identity,_,name in labels):break
   except ValueError:pass
   key('j',r'\[shell.action.select_next_linen\] id=\d+')
  else:raise ValueError('saved document never selected in actual Linen')
  start=len(text());press('ret');wait(rf'\[quil.persist.load.ok\] bytes=12288 hash={hex(fnv(body))} doc={object_id}(?:\s|$)',start);focus_surface(201,text,key)
 try:
  deadline=time.monotonic()+20
  while not sock.exists():
   health()
   if time.monotonic()>deadline:raise TimeoutError('QMP unavailable')
   time.sleep(.02)
  q=QMP(sock)
  if resolution_text=='1024x768':wait(r'SexOS compact 1024x768',timeout=15);press('down');press('ret')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0');wait(r'\[linen.disk.publish.done\]')
  start=len(text());q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':30000}},{'type':'abs','data':{'axis':'y','value':30000}}]});wait(r'\[shell.abs.normalize\] raw_x=30000 raw_y=30000 .*reason=(?:ok|duplicate_sample)',start)
  if recovery:reopen(expected['object']);picture('quil-cold-reopened','quil')
  else:
   focus_surface(300,text,key);key('h',r'\[browser.http.document\] ticket=2 path=/welcome status=200 private_bytes=12288 positive_release=1');press('v');picture('browser-reader')
   for _ in range(5):movement('right')
   for _ in range(3):movement('shift','right')
   for _ in range(5):movement('shift','left')
   picture('browser-reverse',selection=reverse_selection(body),status='Selected');zoom();picture('browser-reflow',selection=reverse_selection(body),status='Selected');zoom()
   chord('ctrl','a');picture('browser-all',selection=(0,len(body)),status='Selected');start=len(text());chord('ctrl','c');wait(r'\[browser.clipboard.published\] ticket=\d+ bytes=12288 generation=\d+ hash='+hex(fnv(body))+r' positive_ack=1',start);picture('browser-copied',selection=(0,len(body)),status='Copied')
   focus_surface(201,text,key);start=len(text());chord('ctrl','n');wait(r'\[quil.new.ok\] bytes=0',start);start=len(text());[press(letter)for letter in 'draft'];wait(r'\[quil.document.edit\] revision=\d+ pending=0 bytes=5',start);start=len(text());chord('ctrl','a');wait(r'\[quil.selection\] generation=\d+ revision=\d+ bytes=\d+ anchor=0 caret=\d+',start);start=len(text());chord('ctrl','v');wait(r'\[quil.clipboard.paste\] ticket=\d+ bytes=12288 generation=\d+ hash='+hex(fnv(body))+r' applied=1 atomic=1',start);picture('quil-pasted','quil')
   start=len(text());chord('ctrl','v');wait(r'\[quil.clipboard.paste\] ticket=\d+ bytes=12288 generation=\d+ hash='+hex(fnv(body))+r' applied=0 atomic=1 reason=capacity',start);picture('quil-capacity-rejected','quil',notice='Capacity')
   before=disk.read_bytes();(out/'before-save.img.gz').write_bytes(gzip.compress(before,mtime=0));start=len(text());chord('ctrl','shift','s');wait(r'\[quil.name.mode\] on=1 save_as=1',start)
   for code in contract['name_keys']:press(code)
   saved=key('ret',r'\[quil.persist.save.ok\] bytes=12288 hash='+hex(fnv(body))+r' doc=(\d+)(?:\s|$)');wait(r'\[quil.document.saved\] saved_revision=(\d+) revision=\1 dirty=0');document=disk_check(before,disk.read_bytes(),body);assert document['object']==int(saved[1])
   (out/'saved-raw-document.json.gz').write_bytes(gzip.compress((json.dumps(storage.raw_document(disk.read_bytes(),document['object']),indent=2)+'\n').encode(),mtime=0));(out/'before-producer-close.img.gz').write_bytes(gzip.compress(disk.read_bytes(),mtime=0))
   focus_surface(300,text,key);start=len(text());press('f11');wait(r'\[browser.import.closed\] target=15 task=15 pending=0 accepted=0 terminal=1 owned_storage=erased',start);wait(r'\[shell.app.cleanup.ack\] target=15 task=15 surface=300 ok=1',start);wait(r'\[sext.app.pool.ready\] target=15 task=15 .*ok=1',start);cleanup_receipts(text(),15,15,start);browser_closed=True
   focus_surface(201,text,key);picture('quil-after-producer-close','quil');start=len(text());chord('ctrl','v');wait(r'\[quil.clipboard.stopped\] ticket=\d+ state=Empty payload_erased=1',start);picture('quil-empty-after-producer-close','quil',notice='Empty')
   unchanged_document(gzip.decompress((out/'before-producer-close.img.gz').read_bytes()),disk.read_bytes(),document,body);reopen(document['object']);picture('quil-reopened','quil')
 except Exception as exc:failure=failure_summary(exc)
 finally:
  peer.close()
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True);(out/'nvme.img.gz').write_bytes(gzip.compress(disk.read_bytes(),mtime=0));manifest['final_disk_sha256']=sha(disk.read_bytes());manifest['error']=failure;(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');(out/'peer-exchanges.json').write_text(json.dumps(dict(rows=peer.rows,dns=peer.dns,attempts=peer.attempts,errors=peer.errors),indent=2)+'\n');(out/'actions.json').write_text(json.dumps(actions,indent=2)+'\n');(out/'frames.json').write_text(json.dumps(frames,indent=2)+'\n')
 if failure or peer.errors:raise RuntimeError(str(dict(error=failure,peer_errors=peer.errors)))
 result=replay_boot(out,contract);result['binding']=bound;(out/'result.json').write_text(json.dumps(result,indent=2)+'\n');return result


def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',type=Path);p.add_argument('--binding',type=Path);p.add_argument('--source-disk',type=Path);p.add_argument('--out',type=Path);p.add_argument('--check',type=Path);p.add_argument('--text-profile',choices=tuple(TEXT_PROFILES));p.add_argument('--model',choices=('e1000','e1000e'),default='e1000e');p.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');a=p.parse_args();root=Path(__file__).resolve().parents[1]
 if a.check:
  first=json.loads((a.check/'write/manifest.json').read_text());profile,body=manifest_profile(first,a.text_profile);contract=source_contract(root,profile)
  result={name:replay_boot(a.check/name,contract)for name in('write','recovery')};first=json.loads((a.check/'write/manifest.json').read_text());second=json.loads((a.check/'recovery/manifest.json').read_text())
  if manifest_profile(second,profile)[0]!=profile or first['binding']!=second['binding']or first['final_disk_sha256']!=second['input_disk_sha256']:raise ValueError('cold replay did not use exact same ISO/terminal disk')
  print(json.dumps(dict(ok=True,boots=2,results=result)));return
 if not all((a.iso,a.binding,a.source_disk,a.out)):p.error('native run requires --iso --binding --source-disk --out')
 contract=source_contract(root,a.text_profile or 'ascii')
 if not target_check(root)['ok']:raise ValueError('non-Unix single GLOBAL_VAS target check failed')
 iso=a.iso.resolve(strict=True);binding=a.binding.resolve(strict=True);seed=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise ValueError('fresh clipboard cohort directory required')
 first=run_boot(root,iso,binding,seed,out/'write',contract,a.model,a.resolution);second=run_boot(root,iso,binding,out/'write/nvme.img',out/'recovery',contract,a.model,a.resolution,True,first['document'])
 if first['binding']!=second['binding']or json.loads((out/'write/manifest.json').read_text())['final_disk_sha256']!=json.loads((out/'recovery/manifest.json').read_text())['input_disk_sha256']:raise ValueError('cold boot source/image/disk binding differs')
 result=dict(ok=first['ok']and second['ok'],text_profile=contract['text_profile'],body_sha256=sha(contract['body']),boots=2,binding=first['binding'],document=first['document'],scope=SCOPE);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))

if __name__=='__main__':main()
