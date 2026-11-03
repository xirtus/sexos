#!/usr/bin/env python3
"""Actual controlled HTTPS text/html page read as text in the Browser reader.

Same BOOT endpoint, trust profile and held authority as https_document_gate;
only the fixed resource differs (/page.html, Accept: text/html). The expected
reader text is produced on the host by compiling the same bounded converter
(apps/kaleidoscope/src/html_text.rs); its behaviour is independently pinned by
tests/browser_html_text.rs and tests/browser_script.rs. Inline scripts run only
through the bounded subset; no general JavaScript, CSS, layout or web claim.
"""
import argparse,hashlib,json,os,re,shutil,socket,subprocess,tempfile,threading,time
from pathlib import Path
import https_document_gate as doc
import tls_exchange_gate as tls
import http_content_gate as http
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from network_contention_gate import frozen_binding,MAX_PCAP
from check_nonunix_target import inspect as target_check

REQUEST=b'GET /page.html HTTP/1.1\r\nHost: probe.sexos.test:46080\r\nConnection: close\r\nAccept: text/html\r\n\r\n'
PAGE=(b'<!DOCTYPE html><html><head><title>Lab notes</title><style>body{color:red}</style>'
 b'<script>while(1){}</script></head><body>'
 b'<h1>SexOS field notes</h1><p>One shared address space supplies addresses;\n held capabilities supply authority.</p>'
 b'<p><script>var n = 6; document.write("Script says " + n*7, " / ", -(-(-((1+1)*3))))</script></p>'
 b"<p><script>var t = 'sexos'; if (t.length === 5) { document.write(t.toUpperCase(), ' ', Math.max(2, 7) > 5 ? 'ok' : 'no') } else document.write('bad')</script></p>"
 b'<p>Press <kbd>W</kbd> for this <strong>page</strong>, see <code>html_text.rs</code>.</p>'
 b"<p><script>function sq(x) { return x * x }\nvar s = 0; for (var i = 1; i <= 200; i++) s += i; var n = 3; while (n > 0) { document.write(n, ' '); n-- } document.write('sum ', s, ' sq ', sq(12))</script></p>"
 b"<p><script>function fact(n) { if (n <= 1) return 1; return n * fact(n - 1) }\ndocument.write('deep ', ((((((((((7)))))))))), ' ', fact(7))</script></p>"
 b"<p><script>var sq = []; for (var i = 1; i <= 6; i++) sq.push(i * i); document.write('squares ', sq.join(' '), ' last ', sq[sq.length - 1])</script></p>"
 b"<p><script>var parts = 'held, never ambient'.split(', '); document.write(parts.length, ' ', parts[1].toUpperCase(), ' at ', 'SexOS'.indexOf('OS'), ' ', parseInt('42px') + 1)</script></p>"
 b"<p><script>var odd = []; for (var i = 0; i < 20; i++) { if (i % 2 === 0) continue; if (i > 9) break; odd.push(i) } document.write(`odd ${odd.join('+')} = ${1 + 3 + 5 + 7 + 9}`)</script></p>"
 b"<p><script>var app = {name: 'Quil', saves: 2}; app.saves++; app['kind'] = 'editor'; document.write(JSON.stringify(app), ' keys ', Object.keys(app).length)</script></p>"
 b"<p><script>var price = 19.999; document.write(typeof price, ' ', price.toFixed(2), ' ', null == undefined, ' ', typeof missing)</script></p>"
 b"<p><script>var apps = ['spindle', 'quil', 'linen', 'browser']; var t = 0; do { t += 2 } while (t < 5); var kind = ''; switch (apps.length) { case 4: kind = 'four'; break; default: kind = '?' }\ndocument.write(apps.filter(a => a.includes('l')).map(a => a.toUpperCase().padEnd(8, '.')).sort().join(' '), ' ', apps.reduce((n, a) => n + a.length, 0), ' ', t, ' ', kind, ' ', apps.indexOf('quil'), ' ', 'a-b'.replaceAll('-', '+'))</script></p>"
 b'<form><input name=q placeholder="Search notes"> <input type=checkbox checked> exact <button>Find</button></form><p>x<sup>2</sup> <abbr title="Long Count">LC</abbr></p>'
 b"<p><script>var scores = [7, 3, 9]; var [best] = [...scores].sort((a, b) => b - a); var {kind, n} = {kind: 'hex', n: 255}; var cfg = {debug: 1, log: 2}; delete cfg.debug;\ndocument.write(best, ' ', Math.max(...scores, 4), ' ', kind, ' ', n.toString(16), ' ', Number.isInteger(2.0), ' ', 'log' in cfg, 'debug' in cfg, ' ', parseFloat('2.5em') * 2)</script></p>"
 b"<p><script>document.write('code ', 'S'.charCodeAt(0), ' ', String.fromCharCode(0x53, 101, 120), ' ', (0xff).toString(2).length)</script></p>"
 b'<h3>Notes</h3>'
 b'<h2>Rules</h2><ol><li>BOOT grants routes &amp; identities.<ul><li>held, never ambient</li></ul></li><li>Applications cannot mint them.</li><li>Only the bounded script subset runs.</li></ol>'
 b'<p>Read the <a href="/guide">guide</a>, the <a href=\'/ledger\'>ledger</a> or <a href="https://elsewhere.example/x">elsewhere</a>.<br>Entities: &lt;tag&gt; &#65;&#x42;C</p>'
 b'<table><tr><th>key</th><th>value</th></tr><tr><td>pages</td><td>1</td></tr></table>'
 b'<script>fetch("/x")</script><script src="/x.js"></script>'
 b'<blockquote>Quoted: authority is held, never ambient.</blockquote><hr>'
 b'<p><img src="/art.qoi" alt="SexOS mark"></p><!-- a comment that never renders --><p>END OF CONTROLLED HTML PAGE</p></body></html>')
GUIDE_REQUEST=b'GET /guide HTTP/1.1\r\nHost: probe.sexos.test:46080\r\nConnection: close\r\nAccept: text/html\r\n\r\n'
GUIDE=b'<html><body><h1>Guide</h1><p>Followed from the field notes on the same BOOT endpoint.</p><pre>\nkey   action\n1-9   follow link\nBksp  back\n</pre><p><a href="/page.html">Back to notes</a></p></body></html>'
# The page response is ~1.1 KB: one full 1024-byte TLS record plus a short
# one. Before F11 this size failed at guest connection close.
SCOPE='Actual fixed BOOT-controlled HTTPS text/html page through held policy/seed/UTC/DNS/duplex authority, converted by the bounded reader (inline scripts only through the bounded subset) and checked against reader glyph pixels. Public laboratory root only; no general JavaScript, CSS, layout, forms, arbitrary URL or public web trust claim.'

def response(mode,page=PAGE):
 return b'\r\n'.join([b'HTTP/1.1 200 OK',b'Content-Type: text/html; charset=utf-8',b'Content-Length: '+str(len(page)).encode(),b'Connection: close'])+b'\r\n\r\n'+page

class HtmlPeer(doc.ControlledHTTPSPeer):
 """Serves the fixed page, then exactly one followed page; nothing else."""
 EXCHANGES=((REQUEST,PAGE),(GUIDE_REQUEST,GUIDE),(REQUEST,PAGE))
 def __init__(self,root):
  super().__init__(root,'ready');self.served=[threading.Event()for _ in self.EXCHANGES]
 def serve_dns(self,sock):
  while not self.stop.is_set():
   try:data,remote=sock.recvfrom(513)
   except socket.timeout:continue
   ticket=int.from_bytes(data[:2],'big')
   if len(data)<2 or data!=tls.dns_query(ticket)or len(self.dns)>=len(self.EXCHANGES):raise ValueError('unexpected controlled DNS request')
   r=tls.dns_response(ticket);sent=sock.sendto(r,remote);self.dns.append(dict(ticket=ticket,response_hex=r.hex()));self.attempts.append(dict(ticket=ticket,kind='answer',response_hex=r.hex(),sent_bytes=sent))
 def serve_tls(self,listener,ctx):
  for index,(request,page)in enumerate(self.EXCHANGES):
   sock=self.accept(listener)
   if sock is None:return
   row=dict(index=index,request_hex=None,response_hex=None,unwrap_completed=False);self.rows.append(row)
   w=ctx.wrap_socket(sock,server_side=True,do_handshake_on_connect=False);self.live.append(w);w.do_handshake()
   if(w.version(),w.cipher()[0],getattr(w,'controlled_sni',None))!=('TLSv1.3',tls.CIPHER,tls.HOST):raise ValueError('foreign TLS profile')
   got=bytearray()
   while len(got)<len(request):
    chunk=w.recv(4096)
    if not chunk:raise ValueError('guest closed before exact request')
    got.extend(chunk)
   if bytes(got)!=request:raise ValueError(f'request {index} differs: {bytes(got)!r}')
   row['request_hex']=bytes(got).hex();wire=response('ready',page)
   # One TLS record per 1024 bytes: the guest negotiates max_fragment_length=2^10.
   for offset in range(0,len(wire),1024):w.sendall(wire[offset:offset+1024])
   row['response_hex']=wire.hex()
   w.unwrap().close();row['unwrap_completed']=True;self.served[index].set()
   if index==0:self.completed.set()
  while not self.stop.is_set():
   try:extra,_=listener.accept();extra.close();raise ValueError('extra HTTPS endpoint connection')
   except socket.timeout:pass

def expected_text(root,page=PAGE):
 with tempfile.TemporaryDirectory()as d:
  src=Path(d)/'oracle.rs';src.write_text('extern crate alloc;\n#[path='+json.dumps(str(root/'apps/kaleidoscope/src/script.rs'))+']mod script;\n#[path='+json.dumps(str(root/'apps/kaleidoscope/src/html_text.rs'))+']mod html_text;\nuse std::io::{Read,Write};fn main(){let mut i=Vec::new();std::io::stdin().read_to_end(&mut i).unwrap();let mut o=vec![0u8;12288];let c=html_text::convert(&i,&mut o);eprintln!("{} {}",c.links,c.truncated as u8);std::io::stdout().write_all(&o[..c.len]).unwrap();}\n')
  subprocess.run(['rustc','--edition=2021','-O',str(src),'-o',str(Path(d)/'oracle')],check=True,capture_output=True)
  r=subprocess.run([str(Path(d)/'oracle')],input=page,capture_output=True,check=True);links,cut=map(int,r.stderr.split())
  return r.stdout,links,cut

def geometry(trace,contract,text,request=REQUEST,path=b'/page.html'):
 rel=list(re.finditer(rf'\[browser.https.released\] ticket=(\d+) tx=\d+ rx=\d+ request_bytes={len(request)} body_bytes=(\d+) body_hash=(0x[0-9a-f]+) positive_release=1 authenticated=1 public_test_root=1',trace));assert rel,'missing positive authenticated HTML release'
 r=rel[-1];assert int(r[2])==len(text)and r[3]==hex(tls.fingerprint(text)),'released body is not the converted page text'
 tile,seq=doc.viewport(trace);frame=http.reader_frame(text,tile['w'],tile['h'],contract,0,path);frame.update(tile=tile,viewport=seq,document_ticket=int(r[1]))
 lay=list(re.finditer(r'\[browser.reader.layout\] first=(\d+) end=(\d+) body_rows=(\d+) rows=(\d+) cols=(\d+) total_lines=(\d+) source_bytes=(\d+) viewport=(\d+) origin=authenticated_body tls=1 public_test_root=1 pixels=unconfirmed',trace))
 want=tuple(frame[k]for k in('first','end','body_rows','rows','cols','total_lines','source_bytes','viewport'));assert lay and lay[-1].start()>r.end()and tuple(map(int,lay[-1].groups()))==want,f'reader layout differs {lay[-1].groups() if lay else None} {want}'
 return frame

def main():
 from quil_workflow_gate import palette_commands,palette_select
 p=argparse.ArgumentParser();[p.add_argument(f'--{n}',required=True,type=Path)for n in('iso','source-disk','binding','out')];p.add_argument('--model',choices=('e1000','e1000e'),default='e1000e');p.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');a=p.parse_args()
 root=Path(__file__).resolve().parents[1];out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh HTML evidence directory required')
 binding=frozen_binding(root,a.iso.resolve(strict=True),a.binding.resolve(strict=True));held=json.loads(a.binding.read_text())
 if held['runner_sha256'].get('scripts/https_html_gate.py')!=hashlib.sha256(Path(__file__).read_bytes()).hexdigest():raise RuntimeError('HTML checker absent from exact frozen binding')
 text,links,cut=expected_text(root);contract=doc.load_contract(root);contract.update(request=REQUEST,reader_path=b' /page.html');resolution=tuple(map(int,a.resolution.split('x')))
 doc.REQUEST=REQUEST;doc.response=response
 peer=HtmlPeer(root);peer.start();disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-html-{os.getpid()}.sock'
 command=['qemu-system-x86_64','-M','q35','-cpu','max,+pku,+rdseed','-rtc',doc.RTC_NORMAL,'-m','512M','-vga','none','-device','VGA,xres='+a.resolution.replace('x',',yres='),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(a.iso.resolve()),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}']
 manifest=dict(binding=binding,command=command,model=a.model,resolution=a.resolution,page_sha256=hashlib.sha256(PAGE).hexdigest(),page_bytes=len(PAGE),text_sha256=hashlib.sha256(text).hexdigest(),text_bytes=len(text),links=links,request_hex=REQUEST.hex());(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');(out/'expected-text.txt').write_bytes(text)
 proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic();failure=None;checks={}
 def trace():return serial.read_text(errors='replace')if serial.exists()else''
 def health():
  if time.monotonic()-began>300:raise TimeoutError('bounded 300 second HTML gate')
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if peer.errors:raise RuntimeError(str(peer.errors))
  if any(f.exists()and f.stat().st_size>MAX_PCAP for f in(serial,pcap)):raise RuntimeError('raw evidence exceeded bound')
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',trace()):raise RuntimeError('guest failure')
 def wait(pattern,start=0,timeout=90):
  deadline=time.monotonic()+timeout
  while time.monotonic()<deadline:
   health();m=re.search(pattern,trace()[start:])
   if m:return m
   time.sleep(.01)
  raise TimeoutError(pattern)
 def key(code,pattern):b=len(trace());q.key(code);return wait(pattern,b)
 try:
  deadline=time.monotonic()+15
  while not sock.exists():
   health()
   if time.monotonic()>deadline:raise TimeoutError('QMP unavailable')
   time.sleep(.05)
  q=QMP(sock)
  if a.resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0');focus_surface(300,trace,key);palette_select(key,'ZoomToggle',palette_commands(root))
  b=len(trace());q.key('ret');wait(r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1',b);wait(r'\[browser.viewport\] target=15 task=15 sequence=\d+ w=\d+ h=\d+ origin=held_shell',b)
  b=len(trace());q.key('w');wait(r'\[shell.browser.input\] code=17 down=1 slot=43 accepted=1',b);wait(r'\[browser.html.intent\] accepted=1 explicit=1 path=/page.html script=0 public_internet=0',b)
  wait(r'\[browser.https.view\] state=Verified controlled_profile=1 public_internet=0',b);doc_event=peer.completed.wait(30)
  if not doc_event:raise TimeoutError('peer close_notify')
  conv=wait(rf'\[browser.html.converted\] source_bytes={len(PAGE)} text_bytes={len(text)} links={links} truncated={cut} scripts_run=12 scripts_refused=2 ticks=\d+',b);checks['converted']=conv[0]
  hw=wait(r'\[browser.stack.high_water\] bytes=(\d+) mapped=65536 painted_at=\d+ untouched=\d+',b);checks['stack_high_water']=int(hw[1])
  if int(hw[1])>60*1024:raise RuntimeError(f'Browser stack high water {hw[1]} leaves under 4 KiB')
  def reader(name,text,request,path):
   shot=out/(name+'.ppm');result=dict(ok=False,errors=['no frame']);deadline=time.monotonic()+25
   while time.monotonic()<deadline:
    health();q.cmd('screendump',{'filename':str(shot)})
    try:result=doc.frame_pixels(shot.read_bytes(),geometry(trace(),contract,text,request,path),contract,resolution)
    except(AssertionError,ValueError,KeyError,IndexError,TypeError)as exc:result=dict(ok=False,errors=[str(exc)])
    if result['ok']:break
    time.sleep(.1)
   checks[name]=result
   if not result['ok']:raise RuntimeError(name+': '+str(result['errors']))
  reader('html-reader',text,REQUEST,b'/page.html')
  b=len(trace());q.key('3');wait(r'\[browser.html.link.refused\] index=3 listed=1 reason=not_endpoint_path network=0',b);time.sleep(1)
  if len(peer.dns)!=1 or len(peer.rows)!=1:raise RuntimeError('refused link caused network work')
  checks['refused_link']='index=3 https://elsewhere.example/x refused, no DNS/TLS'
  guide,glinks,gcut=expected_text(root,GUIDE)
  b=len(trace());q.key('1');wait(r'\[browser.html.link\] index=1 path=/guide same_endpoint=1 script=0 depth=1',b)
  if not peer.served[1].wait(60):raise TimeoutError('followed page not served')
  wait(r'\[browser.https.view\] state=Verified controlled_profile=1 public_internet=0',b)
  checks['followed']=wait(rf'\[browser.html.converted\] source_bytes={len(GUIDE)} text_bytes={len(guide)} links={glinks} truncated={gcut} scripts_run=0 scripts_refused=0 ticks=\d+',b)[0]
  reader('html-followed',guide,GUIDE_REQUEST,b'/guide');(out/'expected-guide.txt').write_bytes(guide)
  b=len(trace());q.key('backspace');wait(r'\[browser.html.back\] path=/page.html depth=0',b)
  if not peer.served[2].wait(60):raise TimeoutError('back page not served')
  wait(rf'\[browser.html.converted\] source_bytes={len(PAGE)} text_bytes={len(text)} links={links} truncated={cut} scripts_run=12 scripts_refused=2 ticks=\d+',b)
  reader('html-back',text,REQUEST,b'/page.html')
  b=len(trace());q.key('backspace');wait(r'\[browser.html.back\] depth=0 network=0',b);time.sleep(1)
  if len(peer.rows)!=3:raise RuntimeError('back at the first page caused network work')
  assert b'Script says 42' in text and b'SEXOS ok' in text and b'3 2 1 sum 20100 sq 144' in text and b'deep 7 5040' in text and b'squares 1 4 9 16 25 36 last 36' in text and b'2 NEVER AMBIENT at 3 43' in text and b'odd 1+3+5+7+9 = 25' in text and b'{"name":"Quil","saves":3,"kind":"editor"} keys 3' in text and b'number 20.00 true undefined' in text and b'LINEN... QUIL.... SPINDLE. 23 6 four 1 a+b' in text and b'[input: Search notes] [x] exact [button: Find]' in text and b'9 9 hex ff true truefalse 5' in text and b'code 83 Sex 8' in text and b'while' not in text and b'color:red' not in text and text.startswith(b'Title: Lab notes'),'head/script/style handling differs'
 except Exception as exc:failure=repr(exc)
 finally:
  peer.close()
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
 t=trace();request_ok=[r.get('request_hex')for r in peer.rows]==[REQUEST.hex(),GUIDE_REQUEST.hex(),REQUEST.hex()]
 res=dict(ok=failure is None and request_ok and not peer.errors,error=failure,request_exact=request_ok,peer=peer.rows,checks=checks,resolution=a.resolution,model=a.model,scope=SCOPE)
 (out/'result.json').write_text(json.dumps(res,indent=2,default=str)+'\n')
 if not res['ok']:raise RuntimeError(json.dumps(dict(error=failure,request_exact=request_ok,errors=peer.errors)))
 print(json.dumps(dict(ok=True,resolution=a.resolution,text_bytes=len(text),links=links)))

if __name__=='__main__':main()
