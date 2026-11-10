#!/usr/bin/env python3
"""Classic scripts execute as private resources; fetch/XHR remain CORS restricted.
Two real HTTP origins check public fetch/eval hooks, MIME/nosniff/CORP, changed or
removed elements, navigation cancellation and actual Origin/Referer headers.
"""
import argparse,hashlib,http.server,json,os,re,shutil,threading,time
from pathlib import Path
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]

def main():
 p=argparse.ArgumentParser()
 for n in ('iso','source-disk','out'):p.add_argument('--'+n,required=True,type=Path)
 p.add_argument('--program',type=Path,default=ROOT/'iso_root/programs/web');p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);seen=[];servers=[]
 class Handler(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   seen.append(dict(port=self.server.server_address[1],path=self.path,origin=self.headers.get('Origin'),referer=self.headers.get('Referer')))
   if self.path=='/web':body=a.program.read_bytes();mime='application/octet-stream';extra=[]
   elif self.path=='/page':
    body=('''<!doctype html><title>Classic resources</title><h1>Classic resources</h1><script>
     const readableFetch=fetch;let done=0;
     function finish(){if(++done===8)console.log('classic cases done');}
     globalThis.fetch=()=>{console.log('FAIL fetch hook');throw Error('fetch hook');};
     globalThis.eval=()=>{console.log('FAIL eval hook');throw Error('eval hook');};
     window.onerror=(message,source,line,column,error)=>{if(message==='same error')console.log('same error callback',error===sameError,error instanceof TypeError);else if(message==='Script error.')console.log('opaque error callback',error===null);else console.log('FAIL unmuted error',message);};
     window.addEventListener('error',e=>console.log('error event',e.message,e.message==='Script error.'?e.error===null:e.error===sameError));
     function bad(path,name,attr){const s=document.createElement('script');s.src='''+json.dumps(bbase)+'''+path;if(attr)s.setAttribute('crossorigin',attr);s.onload=()=>console.log('FAIL '+name+' executed');s.onerror=()=>{console.log(name+' blocked');finish();};document.head.appendChild(s);}
     const good=document.createElement('script');good.id='good';good.src='''+json.dumps(bbase)+'''+'/good.js';
     good.onload=()=>{console.log('classic loaded',classicRuns,good.textContent==='');finish();readableFetch('''+json.dumps(bbase)+'''+'/good.js').then(()=>console.log('FAIL fetch leaked'),()=>{console.log('fetch blocked');finish();});};
     good.onerror=()=>console.log('FAIL classic resource');document.head.appendChild(good);
     bad('/html.js','html');bad('/nosniff.js','nosniff');bad('/corp.js','corp');bad('/cors.js','cors','anonymous');
     for(const pair of [['''+json.dumps(abase)+'''+'/same-error.js','same'],['''+json.dumps(bbase)+'''+'/opaque-error.js','opaque']]){const s=document.createElement('script');s.src=pair[0];s.onload=()=>{console.log(pair[1]+' error resource loaded');finish();};s.onerror=()=>console.log('FAIL throwing script load');document.head.appendChild(s);}
     const removed=document.createElement('script');removed.src='''+json.dumps(bbase)+'''+'/removed.js';document.head.appendChild(removed);removed.remove();
     const changed=document.createElement('script');changed.src='''+json.dumps(bbase)+'''+'/stale.js';document.head.appendChild(changed);changed.src='''+json.dumps(bbase)+'''+'/changed.js';
     const inline=document.createElement('script');inline.text="console.log('inline intrinsic')";document.head.appendChild(inline);
     </script>''').encode();mime='text/html';extra=[]
   elif self.path=='/cancel':body=('''<!doctype html><script>const s=document.createElement('script');s.src='''+json.dumps(bbase)+'''+'/old-page.js';document.head.appendChild(s);location.href='/next';</script>''').encode();mime='text/html';extra=[]
   elif self.path=='/next':body=b"<!doctype html><h1>New page</h1><script>console.log('navigation clean',typeof classicRuns);</script>";mime='text/html';extra=[]
   elif self.path=='/good.js':body=b"globalThis.classicRuns=(globalThis.classicRuns||0)+1; document.body.setAttribute('data-classic','ran'); console.log('classic body',document.currentScript.id);";mime='application/javascript';extra=[('X-Content-Type-Options','nosniff')]
   elif self.path=='/same-error.js':body=b"globalThis.sameError=new TypeError('same error');throw sameError;";mime='application/javascript';extra=[]
   elif self.path=='/opaque-error.js':body=b"throw new TypeError('private cross-origin detail');";mime='application/javascript';extra=[]
   elif self.path in ('/html.js','/nosniff.js','/corp.js','/cors.js'):
    body=b"console.log('FAIL rejected source executed');";mime='text/html' if self.path=='/html.js' else 'text/plain' if self.path=='/nosniff.js' else 'application/javascript';extra=[('X-Content-Type-Options','nosniff')] if self.path=='/nosniff.js' else [('Cross-Origin-Resource-Policy','same-origin')] if self.path=='/corp.js' else []
   else:self.send_response(404);self.end_headers();return
   self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Content-Length',str(len(body)))
   for k,v in extra:self.send_header(k,v)
   self.end_headers();self.wfile.write(body)
  def log_message(self,*args):pass
 class Server(http.server.ThreadingHTTPServer):daemon_threads=True
 for _ in range(2):
  server=Server(('127.0.0.1',0),Handler);servers.append(server);threading.Thread(target=server.serve_forever,daemon=True).start()
 abase,bbase=[f'http://10.0.2.2:{server.server_address[1]}' for server in servers]
 disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0';os.environ.setdefault('SEXOS_GATE_BOUND','900')
 g=boot(a.iso.resolve(),disk,a.out,'classic',a.resolution);q=g['q'];t=g['text']
 def command(line):
  g['focus'](153);time.sleep(.3)
  for ch in line:
   if ch in ':_+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':{':':'semicolon','_':'minus','+':'equal'}[ch]}],'hold-time':80})
   else:q.key({' ':'spc','/':'slash','.':'dot','-':'minus'}.get(ch,ch))
   time.sleep(.04)
  q.key('ret')
 def close(start):
  ready=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)\r?\n',start,120);target,task=ready[1],ready[2]
  surface=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)\r?\n',start,60)[1]
  g['focus'](int(surface));q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}\r?\n',start,60)
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',start,120)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);start=len(t());command(f'fetch {abase}/web /bin/web')
  saved=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',start,240);assert saved[1].startswith('Saved'),saved[1]
  start=len(t());command(f'run +net web {abase}/page')
  g['wait'](r'\[web.console\] level=0 classic cases done\r?\n',start,300)
  for mark in ('classic body good','classic loaded 1 true','fetch blocked','html blocked','nosniff blocked','corp blocked','cors blocked','inline intrinsic','same error callback true true','opaque error callback true','error event same error true','error event Script error. true','same error resource loaded','opaque error resource loaded'):
   assert '[web.console] level=0 '+mark in t()[start:],mark
  assert '[web.console] level=0 FAIL' not in t()[start:]
  good=[r for r in seen if r['path']=='/good.js'];assert len(good)==2,good
  assert {r['origin'] for r in good}=={None,abase},good
  assert all(r['referer']==abase+'/' for r in good),good
  assert not any(r['path'] in ('/removed.js','/stale.js','/changed.js') for r in seen),seen
  close(start);start=len(t());command(f'run +net web {abase}/cancel')
  g['wait'](r'\[web.console\] level=0 navigation clean undefined\r?\n',start,300)
  assert not any(r['path']=='/old-page.js' for r in seen),seen
  close(start)
  proof=dict(ok=True,iso_sha256=hashlib.sha256(a.iso.read_bytes()).hexdigest(),program_sha256=hashlib.sha256(a.program.read_bytes()).hexdigest(),requests=seen)
  (a.out/'result.json').write_text(json.dumps(proof,indent=2)+'\n');print(proof)
 finally:
  g['stop']()
  for server in servers:server.shutdown();server.server_close()
if __name__=='__main__':main()
