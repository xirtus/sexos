#!/usr/bin/env python3
"""A small CalDAV/CardDAV server for gates, on the host (QEMU's user network reaches it
as 10.0.2.2). It keeps one collection of resources with etags, a ctag and RFC 6578
sync-tokens; answers PROPFIND (Depth 0 and 1), REPORT calendar-query,
addressbook-query and sync-collection, GET, PUT with If-Match / If-None-Match and
DELETE; and refuses requests without the right Basic credentials. Every request is
logged so a gate can check what the guest sent."""
import base64,http.server,threading,re
from xml.sax.saxutils import escape

class Dav:
 def __init__(self,base,collection,kind,user='alice',password='wonderland',name='Home'):
  self.base,self.collection,self.kind,self.name=base,collection,kind,name
  self.auth='Basic '+base64.b64encode(f'{user}:{password}'.encode()).decode()
  self.items={};self.log=[];self.version=1;self.requests=[];self.lock=threading.Lock();self.files={}
 def put(self,name,data):
  with self.lock:
   self.version+=1;path=self.collection+name;self.items[path]=(self.version,data);self.log.append((self.version,path,False))
 def delete(self,name):
  with self.lock:
   self.version+=1;path=self.collection+name;self.items.pop(path,None);self.log.append((self.version,path,True))
 def tag(self,n):return f'"e{n}"'
 def ctype(self):return 'text/calendar; charset=utf-8' if self.kind=='cal' else 'text/vcard; charset=utf-8'
 def data_prop(self,n,d):
  el='C:calendar-data' if self.kind=='cal' else 'R:address-data'
  return f'<D:getetag>{escape(self.tag(n))}</D:getetag><{el}>{escape(d)}</{el}>'
 def response(self,path,props):return f'<D:response><D:href>{escape(path)}</D:href><D:propstat><D:prop>{props}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>'
 def ms(self,inner,token=False):
  t=f'<D:sync-token>http://fixture/sync/{self.version}</D:sync-token>' if token else ''
  return ('<?xml version="1.0" encoding="utf-8"?>\n<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:R="urn:ietf:params:xml:ns:carddav" xmlns:CS="http://calendarserver.org/ns/" xmlns:A="http://apple.com/ns/ical/">'+inner+t+'</D:multistatus>').encode()
 def handle(self,method,path,headers,body):
  """(status, headers dict, body bytes)"""
  self.requests.append((method,path,dict(headers),body))
  if path in self.files and method=='GET':return 200,{'Content-Type':'text/plain'},self.files[path]
  if headers.get('Authorization')!=self.auth:return 401,{'WWW-Authenticate':'Basic realm="fixture"'},b''
  with self.lock:
   text=body.decode('utf-8','replace')
   if method=='PROPFIND':
    if headers.get('Depth')=='1':
     rt='<C:calendar/>' if self.kind=='cal' else '<R:addressbook/>'
     inner=self.response(self.base,'<D:resourcetype><D:collection/></D:resourcetype>')+self.response(self.collection,f'<D:resourcetype><D:collection/>{rt}</D:resourcetype><D:displayname>{escape(self.name)}</D:displayname><A:calendar-color>#8BD17CFF</A:calendar-color>')
     return 207,{},self.ms(inner)
    return 207,{},self.ms(self.response(self.collection,f'<CS:getctag>c{self.version}</CS:getctag><D:sync-token>http://fixture/sync/{self.version}</D:sync-token>'))
   if method=='REPORT':
    if 'sync-collection' in text:
     m=re.search(r'sync/(\d+)',text);since=int(m[1]) if m else 0
     inner='';done=set()
     for v,p,removed in reversed(self.log):
      if v<=since or p in done:continue
      done.add(p)
      if removed:inner+=f'<D:response><D:href>{escape(p)}</D:href><D:status>HTTP/1.1 404 Not Found</D:status></D:response>'
      elif p in self.items:inner+=self.response(p,self.data_prop(*self.items[p]))
     return 207,{},self.ms(inner,True)
    return 207,{},self.ms(''.join(self.response(p,self.data_prop(*it)) for p,it in self.items.items()))
   if method=='GET':
    it=self.items.get(path)
    return (200,{'ETag':self.tag(it[0]),'Content-Type':self.ctype()},it[1].encode()) if it else (404,{},b'')
   if method=='PUT':
    cur=self.items.get(path)
    if headers.get('If-None-Match')=='*' and cur:return 412,{},b''
    if headers.get('If-Match') and (not cur or self.tag(cur[0])!=headers.get('If-Match')):return 412,{},b''
    if not path.startswith(self.collection):return 403,{},b''
    self.version+=1;self.items[path]=(self.version,text);self.log.append((self.version,path,False))
    return (204 if cur else 201),{'ETag':self.tag(self.version)},b''
   if method=='DELETE':
    cur=self.items.get(path)
    if not cur:return 404,{},b''
    if headers.get('If-Match') and self.tag(cur[0])!=headers.get('If-Match'):return 412,{},b''
    self.version+=1;self.items.pop(path);self.log.append((self.version,path,True))
    return 204,{},b''
  return 405,{},b''

def serve(dav):
 """Starts the server; returns its port."""
 class Handler(http.server.BaseHTTPRequestHandler):
  def go(self):
   n=int(self.headers.get('Content-Length') or 0);body=self.rfile.read(n) if n else b''
   status,hdrs,out=dav.handle(self.command,self.path,self.headers,body)
   self.send_response(status)
   for k,v in hdrs.items():self.send_header(k,v)
   if status==207:self.send_header('Content-Type','application/xml; charset=utf-8')
   self.send_header('Content-Length',str(len(out)));self.end_headers();self.wfile.write(out)
  do_GET=do_PUT=do_DELETE=do_PROPFIND=do_REPORT=do_OPTIONS=go
  def log_message(self,*a):pass
 class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
 server=Quiet(('127.0.0.1',0),Handler)
 threading.Thread(target=server.serve_forever,daemon=True).start()
 return server.server_address[1]
