#!/usr/bin/env python3
"""Host-side stand-ins for Needle's peers, standard library only:

- FakePrinter: an IPP Everywhere printer (HTTP POST application/ipp) answering
  Get-Printer-Attributes, Validate-Job, Print-Job, Get-Jobs, Get-Job-Attributes and
  Cancel-Job, keeping every document it is sent.
- FakeScanner: an eSCL (AirScan) scanner with one platen page (a PNG).
- FakeResponder: a DNS-SD responder on a UDP port, answering legacy unicast queries
  for _ipp._tcp / _uscan._tcp with PTR, SRV, TXT and A records.

Also an independent IPP and DNS encoder used to make the byte-exact goldens in
crates/sex-print/tests/fixtures (see make_fixtures there)."""
import http.server,socket,socketserver,struct,threading,zlib

# --- IPP (RFC 8010) -----------------------------------------------------------
def ipp_attr(tag,name,value):
 if isinstance(value,bool):v=bytes([value])
 elif isinstance(value,int):v=struct.pack('>i',value)
 elif isinstance(value,tuple):v=struct.pack('>iiB',*value) if len(value)==3 else struct.pack('>ii',*value)
 elif isinstance(value,str):v=value.encode()
 else:v=value
 n=name.encode()
 return bytes([tag])+struct.pack('>H',len(n))+n+struct.pack('>H',len(v))+v
def ipp_attrs(tag,name,values):
 return b''.join(ipp_attr(tag,name if i==0 else '',v) for i,v in enumerate(values))
def ipp_message(version,code,rid,groups,data=b''):
 """groups: [(group_tag,[(value_tag,name,[values])])]"""
 out=bytes(version)+struct.pack('>HI',code,rid)
 for g,attrs in groups:
  out+=bytes([g])+b''.join(ipp_attrs(t,n,vs) for t,n,vs in attrs)
 return out+b'\x03'+data
def ipp_parse(b):
 """(version,code,rid,[(group,{name:[(tag,raw)]})],data); collections are skipped over."""
 ver=(b[0],b[1]);code,rid=struct.unpack('>HI',b[2:8]);at=8;groups=[];last=None;depth=0
 while True:
  t=b[at];at+=1
  if t==3:break
  if t<0x10:groups.append((t,{}));continue
  nl=struct.unpack('>H',b[at:at+2])[0];name=b[at+2:at+2+nl].decode();at+=2+nl
  vl=struct.unpack('>H',b[at:at+2])[0];val=b[at+2:at+2+vl];at+=2+vl
  if t==0x34:depth+=1
  if depth:
   if t==0x37:depth-=1
   continue
  if name:last=name;groups[-1][1].setdefault(name,[])
  groups[-1][1][last].append((t,val))
 return ver,code,rid,groups,b[at:]

# --- PWG raster decoding (independent of sex-print) ---------------------------
def pwg_pages(doc):
 """[(header fields dict, pixel bytes)] from an image/pwg-raster document."""
 assert doc[:4]==b'RaS2',doc[:4];at=4;pages=[]
 while at<len(doc):
  h=doc[at:at+1796];at+=1796
  u=lambda o:struct.unpack('>I',h[o:o+4])[0]
  f=dict(dpi=u(276),width=u(372),height=u(376),bpp=u(388),bpl=u(392),cspace=u(400),total=u(452),media=h[1732:1796].rstrip(b'\0').decode(),page_pt=(u(352),u(356)))
  px=f['bpp']//8;rows=bytearray()
  while len(rows)<f['bpl']*f['height']:
   rep=doc[at]+1;at+=1;line=bytearray()
   while len(line)<f['bpl']:
    c=doc[at];at+=1
    if c==128:line+=b'\xff'*(f['bpl']-len(line))
    elif c>128:n=(257-c)*px;line+=doc[at:at+n];at+=n
    else:line+=doc[at:at+px]*(c+1);at+=px
   rows+=bytes(line)*rep
  pages.append((f,bytes(rows[:f['bpl']*f['height']])))
 return pages

PRINTER_ATTRS=lambda uri:[
 (0x42,'printer-name',['needle-test']),(0x41,'printer-info',['Needle gate printer']),
 (0x41,'printer-make-and-model',['SexOS Fake IPP Everywhere']),(0x23,'printer-state',[3]),
 (0x44,'printer-state-reasons',['none']),(0x45,'printer-uri-supported',[uri]),
 (0x45,'printer-uuid',['urn:uuid:4e65656c-6465-4000-8000-000000000001']),
 (0x49,'document-format-supported',['application/pdf','image/pwg-raster','image/urf','application/octet-stream']),
 (0x49,'document-format-default',['application/octet-stream']),
 (0x44,'media-supported',['iso_a4_210x297mm','na_letter_8.5x11in']),(0x44,'media-default',['iso_a4_210x297mm']),
 (0x44,'sides-supported',['one-sided','two-sided-long-edge','two-sided-short-edge']),
 (0x44,'print-color-mode-supported',['monochrome','color']),(0x22,'color-supported',[True]),
 (0x32,'printer-resolution-supported',[(300,300,3),(600,600,3)]),
 (0x32,'pwg-raster-document-resolution-supported',[(75,75,3),(300,300,3)]),
 (0x44,'pwg-raster-document-type-supported',['sgray_8','srgb_8']),(0x44,'pwg-raster-document-sheet-back',['normal']),
 (0x44,'urf-supported',['V1.4','CP1','W8','SRGB24','RS300']),(0x33,'copies-supported',[(1,99)]),
 (0x23,'operations-supported',[2,4,8,9,10,11]),(0x44,'ipp-versions-supported',['1.1','2.0']),
]

class FakePrinter:
 """IPP over HTTP on 127.0.0.1:port. `jobs` holds every received job."""
 def __init__(self,host_uri_host='10.0.2.2'):
  self.jobs=[];self.requests=[];me=self
  class H(http.server.BaseHTTPRequestHandler):
   def log_message(self,*a):pass
   def do_POST(self):
    body=self.rfile.read(int(self.headers.get('Content-Length','0')))
    reply=me.handle(body,self.path)
    self.send_response(200);self.send_header('Content-Type','application/ipp');self.send_header('Content-Length',str(len(reply)));self.end_headers();self.wfile.write(reply)
  self.server=socketserver.ThreadingTCPServer(('127.0.0.1',0),H);self.server.daemon_threads=True
  self.port=self.server.server_address[1];self.uri=f'ipp://{host_uri_host}:{self.port}/ipp/print'
  threading.Thread(target=self.server.serve_forever,daemon=True).start()
 def job_group(self,j):
  return (2,[(0x21,'job-id',[j['id']]),(0x45,'job-uri',[f"{self.uri}/{j['id']}"]),(0x23,'job-state',[j['state']]),(0x44,'job-state-reasons',['job-completed-successfully' if j['state']==9 else 'none']),(0x42,'job-name',[j['name']])])
 def handle(self,body,path):
  ver,op,rid,groups,data=ipp_parse(body);ops=groups[0][1] if groups else {}
  self.requests.append((op,ops))
  g0=(1,[(0x47,'attributes-charset',['utf-8']),(0x48,'attributes-natural-language',['en'])])
  def val(n):return ops.get(n,[(0,b'')])[0][1]
  if op==0x0B:return ipp_message(ver,0,rid,[g0,(4,PRINTER_ATTRS(self.uri))])
  if op==0x04:
   fmt=val('document-format').decode()
   ok=fmt in('application/pdf','image/pwg-raster','image/urf','application/octet-stream')
   return ipp_message(ver,0 if ok else 0x040A,rid,[g0])
  if op==0x02:
   j=dict(id=len(self.jobs)+1,state=9,name=val('job-name').decode(),format=val('document-format').decode(),data=data,job_attrs=groups[1][1] if len(groups)>1 else {})
   self.jobs.append(j);return ipp_message(ver,0,rid,[g0,self.job_group(j)])
  if op==0x0A:return ipp_message(ver,0,rid,[g0]+[self.job_group(j) for j in self.jobs])
  if op in(0x09,0x08):
   jid=struct.unpack('>i',val('job-id'))[0];js=[j for j in self.jobs if j['id']==jid]
   if not js:return ipp_message(ver,0x0406,rid,[g0])
   if op==0x08:
    if js[0]['state']>=7:return ipp_message(ver,0x0404,rid,[g0])
    js[0]['state']=7;return ipp_message(ver,0,rid,[g0])
   return ipp_message(ver,0,rid,[g0,self.job_group(js[0])])
  return ipp_message(ver,0x0501,rid,[g0])
 def stop(self):self.server.shutdown()

# --- eSCL ----------------------------------------------------------------------
def png_gray(w,h,pixel):
 raw=b''.join(b'\0'+bytes(pixel(x,y) for x in range(w)) for y in range(h))
 chunk=lambda t,d:struct.pack('>I',len(d))+t+d+struct.pack('>I',zlib.crc32(t+d))
 return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',w,h,8,0,0,0,0))+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b'')
SCAN_W,SCAN_H=64,48
def scan_page():
 # A dark frame with a diagonal: something a test can recognise after a round trip.
 return png_gray(SCAN_W,SCAN_H,lambda x,y:0 if x in(0,SCAN_W-1) or y in(0,SCAN_H-1) or x==y else 230)
CAPS='''<?xml version="1.0" encoding="UTF-8"?>
<scan:ScannerCapabilities xmlns:scan="http://schemas.hp.com/imaging/escl/2011/05/03" xmlns:pwg="http://www.pwg.org/schemas/2010/12/sm">
 <pwg:Version>2.63</pwg:Version>
 <pwg:MakeAndModel>SexOS Fake eSCL</pwg:MakeAndModel>
 <scan:UUID>4e65656c-6465-4000-8000-000000000002</scan:UUID>
 <scan:Platen>
  <scan:PlatenInputCaps>
   <scan:MinWidth>16</scan:MinWidth><scan:MaxWidth>2550</scan:MaxWidth>
   <scan:MinHeight>16</scan:MinHeight><scan:MaxHeight>3508</scan:MaxHeight>
   <scan:SettingProfiles><scan:SettingProfile>
    <scan:ColorModes><scan:ColorMode>BlackAndWhite1</scan:ColorMode><scan:ColorMode>Grayscale8</scan:ColorMode><scan:ColorMode>RGB24</scan:ColorMode></scan:ColorModes>
    <scan:DocumentFormats><pwg:DocumentFormat>image/jpeg</pwg:DocumentFormat><pwg:DocumentFormat>image/png</pwg:DocumentFormat><scan:DocumentFormatExt>application/pdf</scan:DocumentFormatExt></scan:DocumentFormats>
    <scan:SupportedResolutions><scan:DiscreteResolutions>
     <scan:DiscreteResolution><scan:XResolution>75</scan:XResolution><scan:YResolution>75</scan:YResolution></scan:DiscreteResolution>
     <scan:DiscreteResolution><scan:XResolution>300</scan:XResolution><scan:YResolution>300</scan:YResolution></scan:DiscreteResolution>
    </scan:DiscreteResolutions></scan:SupportedResolutions>
   </scan:SettingProfile></scan:SettingProfiles>
  </scan:PlatenInputCaps>
 </scan:Platen>
</scan:ScannerCapabilities>
'''
class FakeScanner:
 def __init__(self):
  self.settings=[];self.served=0;self.jobs={};me=self
  class H(http.server.BaseHTTPRequestHandler):
   def log_message(self,*a):pass
   def reply(self,code,body=b'',ctype='text/xml',extra=()):
    self.send_response(code)
    for k,v in extra:self.send_header(k,v)
    self.send_header('Content-Type',ctype);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
   def do_GET(self):
    if self.path=='/eSCL/ScannerCapabilities':return self.reply(200,CAPS.encode())
    if self.path=='/eSCL/ScannerStatus':return self.reply(200,b'<?xml version="1.0"?><scan:ScannerStatus xmlns:scan="x" xmlns:pwg="y"><pwg:State>Idle</pwg:State></scan:ScannerStatus>')
    for jid,left in me.jobs.items():
     if self.path==f'/eSCL/ScanJobs/{jid}/NextDocument':
      if not left:return self.reply(404)
      me.jobs[jid]=left-1;me.served+=1;return self.reply(200,scan_page(),'image/png')
    self.reply(404)
   def do_POST(self):
    body=self.rfile.read(int(self.headers.get('Content-Length','0')))
    if self.path!='/eSCL/ScanJobs':return self.reply(404)
    me.settings.append(body.decode());jid=f'job{len(me.jobs)+1}';me.jobs[jid]=1
    self.reply(201,extra=[('Location',f'http://{self.headers.get("Host","")}/eSCL/ScanJobs/{jid}')])
   def do_DELETE(self):self.reply(200)
  self.server=socketserver.ThreadingTCPServer(('127.0.0.1',0),H);self.server.daemon_threads=True
  self.port=self.server.server_address[1]
  threading.Thread(target=self.server.serve_forever,daemon=True).start()
 def stop(self):self.server.shutdown()

# --- DNS-SD --------------------------------------------------------------------
def dns_name(n):return b''.join(bytes([len(l)])+l.encode() for l in n.split('.'))+b'\0'
def dns_rr(name,t,data,ttl=120,flush=False):
 return dns_name(name)+struct.pack('>HHIH',t,1|(0x8000 if flush else 0),ttl,len(data))+data
def txt(items):return b''.join(bytes([len(i)])+i.encode() for i in items)
def dnssd_answer(qid,question,printer_port,scanner_port,addr=(10,0,2,2)):
 """A response to `question` (raw question section) for one printer and one scanner."""
 host='needle-test.local';P='Needle Test._ipp._tcp.local';S='Needle Test._uscan._tcp.local'
 an=[dns_rr('_ipp._tcp.local',12,dns_name(P)),dns_rr('_uscan._tcp.local',12,dns_name(S))]
 ad=[dns_rr(P,33,struct.pack('>HHH',0,0,printer_port)+dns_name(host),flush=True),
     dns_rr(P,16,txt(['txtvers=1','rp=ipp/print','pdl=application/pdf,image/pwg-raster,image/urf','URF=V1.4,CP1,W8,SRGB24,RS300','Color=T','Duplex=T','UUID=4e65656c-6465-4000-8000-000000000001','ty=SexOS Fake IPP Everywhere']),flush=True),
     dns_rr(S,33,struct.pack('>HHH',0,0,scanner_port)+dns_name(host),flush=True),
     dns_rr(S,16,txt(['txtvers=1','rs=eSCL','vers=2.63','cs=binary,grayscale,color','pdl=image/jpeg,image/png,application/pdf']),flush=True),
     dns_rr(host,1,bytes(addr),flush=True)]
 return struct.pack('>HHHHHH',qid,0x8400,1 if question else 0,len(an),0,len(ad))+question+b''.join(an+ad)
class FakeResponder:
 """Answers any DNS query on 127.0.0.1:port (legacy unicast DNS-SD)."""
 def __init__(self,printer_port,scanner_port):
  self.sock=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);self.sock.bind(('127.0.0.1',0));self.port=self.sock.getsockname()[1];self.queries=[]
  def run():
   while True:
    try:q,src=self.sock.recvfrom(2048)
    except OSError:return
    self.queries.append(q)
    # Echo the question section; a malformed query gets no answer.
    at=12
    try:
     for _ in range(struct.unpack('>H',q[4:6])[0]):
      while q[at]:at+=1+q[at]
      at+=5
    except IndexError:continue
    self.sock.sendto(self.reply(q,at),src)
  self.printer_port,self.scanner_port=printer_port,scanner_port
  threading.Thread(target=run,daemon=True).start()
 def reply(self,q,at):
  qid,=struct.unpack('>H',q[:2]);n,=struct.unpack('>H',q[4:6])
  r=dnssd_answer(qid,q[12:at],self.printer_port,self.scanner_port)
  return r[:4]+struct.pack('>H',n)+r[6:]
 def stop(self):self.sock.close()
