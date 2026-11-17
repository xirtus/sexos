#!/usr/bin/env python3
"""Qupid's Kodi add-ons (Q13b). The host serves a small Kodi repository the way real
ones are laid out: the repository addon zip, addons.xml with its .md5, and each addon's
zip under zips/<id>/ with a .sha256 beside it. Qupid, given the network:

1. `run +net qupid --add-repo URL` installs the repository addon zip;
2. `run +net qupid --install plugin.video.test` reads addons.xml (checked against its
   MD5), plans the install and fetches script.module.dep first, each zip checked against
   its SHA-256, into its own folder;
3. `run +net qupid --addons` lists the plugins with the installed mark (screenshot);
4. plugin.video.bad, whose zip does not match its SHA-256, is refused.

Needs Qupid built (scripts/build_play.sh), or SEXOS_PLAY."""
import argparse,hashlib,http.server,io,os,re,shutil,threading,time,zipfile
from pathlib import Path
from PIL import Image
from install_program import install
ROOT=Path(__file__).resolve().parents[1]

def repo(port):
 base=f'http://10.0.2.2:{port}'
 REPO=f'''<addon id="repository.test" name="Test Repo" version="1.0.0" provider-name="t">
  <extension point="xbmc.addon.repository" name="Test">
    <dir minversion="19.0.0">
      <info compressed="false">{base}/omega/addons.xml</info>
      <checksum>{base}/omega/addons.xml.md5</checksum>
      <datadir zip="true">{base}/omega/zips/</datadir>
      <hashes>sha256</hashes>
    </dir>
  </extension>
</addon>'''
 PLUGIN='''<addon id="plugin.video.test" name="Test Videos" version="2.0.0" provider-name="t">
  <requires><import addon="xbmc.python" version="3.0.0"/><import addon="script.module.dep" version="1.0.0"/></requires>
  <extension point="xbmc.python.pluginsource" library="main.py"><provides>video</provides></extension>
  <extension point="xbmc.addon.metadata"><summary>Videos for the gate</summary></extension>
</addon>'''
 DEP='''<addon id="script.module.dep" name="Dep" version="1.1.0" provider-name="t">
  <requires><import addon="xbmc.python" version="3.0.0"/></requires>
  <extension point="xbmc.python.module" library="lib"/>
</addon>'''
 def z(files):
  b=io.BytesIO()
  with zipfile.ZipFile(b,'w',zipfile.ZIP_DEFLATED) as f:
   for n,d in files:f.writestr(n,d)
  return b.getvalue()
 BAD=PLUGIN.replace('plugin.video.test','plugin.video.bad').replace('Test Videos','Bad Videos').replace('version="2.0.0"','version="1.0.0"')
 index=f'<?xml version="1.0" encoding="UTF-8"?>\n<addons>\n{REPO}\n{PLUGIN}\n{DEP}\n{BAD}\n</addons>\n'.encode()
 out={'/repository.test-1.0.0.zip':z([('repository.test/addon.xml',REPO)]),
      '/omega/addons.xml':index,'/omega/addons.xml.md5':hashlib.md5(index).hexdigest().encode()}
 for id,v,xml,extra in (('plugin.video.test','2.0.0',PLUGIN,('main.py','print("hi")')),('script.module.dep','1.1.0',DEP,('lib/dep.py','X = 1'))):
  data=z([(f'{id}/addon.xml',xml),(f'{id}/{extra[0]}',extra[1])])
  path=f'/omega/zips/{id}/{id}-{v}.zip';out[path]=data
  out[path+'.sha256']=f'{hashlib.sha256(data).hexdigest()}  {id}-{v}.zip\n'.encode()
  # plugin.video.bad: its zip is not the one its hash names.
 data=z([('plugin.video.bad/addon.xml',BAD)]);path='/omega/zips/plugin.video.bad/plugin.video.bad-1.0.0.zip'
 out[path]=data;out[path+'.sha256']=b'0'*64+b'  plugin.video.bad-1.0.0.zip\n'
 return out

class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','+':'plus'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 play=os.environ.get('SEXOS_PLAY',str(ROOT/'target/play/x86_64-sex-app/release/qupid'))
 install(disk,[('qupid',Path(play).read_bytes())])
 files={};served=[]
 class Handler(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   served.append(self.path);body=files.get(self.path)
   if body is None:self.send_response(404);self.send_header('Content-Length','0');self.end_headers();return
   self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def log_message(self,*a):pass
 server=Quiet(('0.0.0.0',0),Handler);port=server.server_address[1];files.update(repo(port))
 threading.Thread(target=server.serve_forever,daemon=True).start()
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,out,'kodi',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k=='@':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'2'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   elif k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def say(text,pattern,timeout=180):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def fetched(name):
  m=say(f'fetch http://10.0.2.2:{port}/{name} {name}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',600);assert m[1].startswith('Saved'),m[1]
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  s=len(t());say(f'run +net qupid --add-repo http://10.0.2.2:{port}/repository.test-1.0.0.zip',r'\[qupid.kodi.(?:repo|failed)\] [^\n]*\n',300)
  added=g['wait'](r'\[qupid.kodi.repo\] id=repository.test version=1.0.0 sources=1',s,5)
  s=len(t());say('run +net qupid --install plugin.video.test',r'\[qupid.kodi.(?:installed|failed)\] [^\n]*\n',300)
  g['wait'](r'\[qupid.kodi.refresh\] repository=repository.test addons=4',s,5)
  inst=g['wait'](r'\[qupid.kodi.installed\] id=plugin.video.test addons=2 list=script.module.dep 1.1.0,plugin.video.test 2.0.0',s,5)
  assert any(x.endswith('script.module.dep-1.1.0.zip.sha256') for x in served),served
  s=len(t());say('run +net qupid --addons',r'\[qupid.kodi.catalogue\] addons=4 plugins=2',300)
  launch=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  time.sleep(3)
  shot=out/'addons.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(.5);Image.open(shot).convert('RGB').save(out/'addons.png')
  # Again: nothing to do.
  s=len(t());say('run +net qupid --install plugin.video.test',r'\[qupid.kodi.(?:installed|failed)\] [^\n]*\n',300)
  g['wait'](r'\[qupid.kodi.installed\] id=plugin.video.test addons=0',s,5)
  # A zip that is not the one hashed is refused, and nothing of it is kept.
  s=len(t());say('run +net qupid --install plugin.video.bad',r'\[qupid.kodi.(?:installed|failed)\] [^\n]*\n',300)
  bad=g['wait'](r'\[qupid.kodi.failed\] (Checksum[^\n]*)',s,5)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'repo':added[0].strip(),'installed':inst[0].strip(),'tampered':bad[1][:80],'requests':len(served)})
 finally:g['stop']()
if __name__=='__main__':main()
