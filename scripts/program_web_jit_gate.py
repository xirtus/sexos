#!/usr/bin/env python3
"""Real QuickJS bytecode compilation in the browser, compared to forced interpreter.
Uses a sealed ISO built from this source head plus a separately hashed opt-in Web
binary. Requires actual runtime code seals and execution counters, not just output.
The generic program_jit_gate.py separately covers kernel W^X and fault isolation.
"""
import argparse, hashlib, http.server, json, os, re, shutil, threading, time
from pathlib import Path
from quil_autosave_gate import boot
from program_role_facts import assert_private_ordinary_launch
ROOT=Path(__file__).resolve().parents[1]

def main():
 p=argparse.ArgumentParser()
 for n in ('iso','source-disk','program','out'):p.add_argument('--'+n,required=True,type=Path)
 p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True)
 class Handler(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   path=a.program if self.path=='/web' else ROOT/'scripts/web_site/jit.html' if self.path=='/jit.html' else None
   if path is None:self.send_response(404);self.end_headers();return
   body=path.read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def log_message(self,*args):pass
 class Server(http.server.ThreadingHTTPServer):daemon_threads=True
 server=Server(('127.0.0.1',0),Handler);threading.Thread(target=server.serve_forever,daemon=True).start()
 disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 os.environ.setdefault('SEXOS_GATE_BOUND','900')
 g=boot(a.iso.resolve(),disk,a.out,'webjit',a.resolution);t=g['text'];q=g['q']
 def command(line):
  g['focus'](153);time.sleep(.3)
  for ch in line:
   if ch in ':+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon' if ch==':' else 'equal'}],'hold-time':80})
   else:q.key({' ':'spc','/':'slash','.':'dot','-':'minus'}.get(ch,ch))
   time.sleep(.04)
  q.key('ret')
 results=[]
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  base=f'http://10.0.2.2:{server.server_address[1]}';start=len(t())
  command(f'fetch {base}/web /bin/kaleidoscope')
  saved=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',start,240);assert saved[1].startswith('Saved'),saved[1]
  expected=f'{sum(((i+7)*3)^(i-7) for i in range(10000))}|2147483648|-2147483649|4294967296|true|a1|2|640,boolean,true,false,0,7,number,true,boolean,false,false,0,true,7'
  for enabled in (False,True):
   start=len(t());command(f'run +net kaleidoscope {base}/jit.html'+('' if enabled else ' --no-jit'))
   value=g['wait'](r'\[web.console\] level=0 jit-result ([^\r\n]+)',start,300);assert value[1]==expected,(value[1],expected)
   captured=g['wait'](r'\[web.console\] level=0 jit-captured ([^\r\n]+)',start,60)
   assert captured[1]==f'true|boolean|false|17|number|true|true|true|96|{sum((i+7)^3 for i in range(128))}',captured[1]
   strings=g['wait'](r'\[web.console\] level=0 jit-strings ([^\r\n]+)',start,60)
   assert strings[1]=='128|3606624|28,36,28,36,8,56,8,56|ab|left,right',strings[1]
   cache_value=g['wait'](r'\[web.console\] level=0 jit-object-cache ([^\r\n]+)',start,60)
   assert cache_value[1]=='124750|retained|replacement|prototype|getter|getter|2|own|right|right|right|2',cache_value[1]
   cache_stats=g['wait'](r'\[web.object-cache\] hits=(\d+) misses=(\d+) concat=(\d+) compare=(\d+)',start,60)
   cache_hits=int(cache_stats[1])
   if enabled:
    assert cache_hits>results[0]['object_cache_hits']+100,cache_hits
    assert int(cache_stats[3])>results[0]['string_concats']+50 and int(cache_stats[4])>results[0]['string_compares']+100,cache_stats.groups()
   identity=g['wait'](r'\[web.console\] level=0 jit-identity ([^\r\n]+)',start,60)
   assert identity[1]=='128 1',identity[1]
   heap_value=g['wait'](r'\[web.console\] level=0 jit-heap-values ([^\r\n]+)',start,60)
   assert heap_value[1]=='14426496|112|32',heap_value[1]
   heap=g['wait'](r'\[web.jit.heap\] templates=(\d+) own=(\d+) compare=(\d+) concat=(\d+)',start,60)
   heap_numbers=list(map(int,heap.groups()))
   if enabled: assert heap_numbers[0]==3 and min(heap_numbers[1:])>100,heap_numbers
   else: assert heap_numbers==[0,0,0,0],heap_numbers
   stats=g['wait'](r'\[web.jit\] enabled=(\d+) compiled=(\d+) executed=(\d+) bailouts=(\d+) refused=(\d+) ticks=(\d+) script_ticks=(\d+)',start,60)
   numbers=list(map(int,stats.groups()));assert numbers[0]==int(enabled),numbers
   ready=g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',start,120);target,task=ready[1],ready[2]
   launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',start,60)
   assert_private_ordinary_launch(ROOT,t(),int(target),int(task),int(launch[1]))
   if enabled:
    assert numbers[1]>=11 and numbers[2]>=12800 and numbers[3]>=3 and numbers[4]==0,numbers
    assert re.search(rf'\[app.program.code\] target={target} task={task} op=seal ',t()[start:]),'no kernel seal for this domain/run'
   else:assert numbers[1:5]==[0,0,0,0],numbers
   results.append(dict(native_heap=heap_numbers,native_heap_result=heap_value[1],string_result=strings[1],string_concats=int(cache_stats[3]),string_compares=int(cache_stats[4]),object_cache_hits=cache_hits,object_cache_result=cache_value[1],enabled=enabled,compiled=numbers[1],executed=numbers[2],bailouts=numbers[3],refused=numbers[4],ticks=numbers[5],script_ticks=numbers[6],result=value[1],target=int(target),task=int(task)))
   g['focus'](int(launch[1]));q.key('f11');g['wait'](rf'\[web.closed\] target={target} task={task}',start,60)
   g['wait'](rf'\[app.private.retired\] target={target} task={task} ',start,120)
  assert '[web.fatal]' not in t() and 'KERNEL PANIC' not in t()
  proof=dict(ok=True,iso_sha256=hashlib.sha256(a.iso.read_bytes()).hexdigest(),program_sha256=hashlib.sha256(a.program.read_bytes()).hexdigest(),runs=results)
  (a.out/'result.json').write_text(json.dumps(proof,indent=2)+'\n');print(proof)
 finally:g['stop']();server.shutdown();server.server_close()
if __name__=='__main__':main()
