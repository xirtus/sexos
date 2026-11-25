#!/usr/bin/env python3
"""Valentine's native local mail view and draft recovery through a QEMU power cut."""
import argparse,http.server,os,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
MAIL=(b'From: =?UTF-8?Q?Ren=C3=A9e?= <renee@example.org>\r\n'
      b'Subject: Dinner on Friday\r\nDate: Tue, 24 Nov 2026 12:00:00 +0000\r\n'
      b'Content-Type: multipart/mixed; boundary=outer\r\n\r\n'
      b'--outer\r\nContent-Type: multipart/alternative; boundary=x\r\n\r\n'
      b'--x\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n'
      b'Meet at seven.\r\n--x\r\nContent-Type: text/html\r\n\r\n'
      b'<img src="https://tracker.example/p.gif">\r\n--x--\r\n'
      b'--outer\r\nContent-Type: application/pdf; name="menu.pdf"\r\n'
      b'Content-Disposition: attachment; filename="menu.pdf"\r\n'
      b'Content-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n'
      b'--outer--\r\n')
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path!='/inbox.eml':self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(MAIL)));self.end_headers();self.wfile.write(MAIL)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from install_program import install
from quil_autosave_gate import boot
from valentine_key_input import send_text

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 elf=(ROOT/'iso_root/programs/valentine').read_bytes();assert elf[:4]==b'\x7fELF'
 loom=(ROOT/'iso_root/programs/loom').read_bytes();assert loom[:4]==b'\x7fELF'
 install(disk,[('valentine',elf),('loom',loom)])
 g=boot(a.iso,disk,a.out,'valentine',a.resolution);t=g['text'];q=g['q']
 def keys(value):
  send_text(q,value)
 def ctrl(key):q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':key}],'hold-time':80})
 def ctrl_alt(key):q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'alt'},{'type':'qcode','data':key}],'hold-time':80})
 def shell(line,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(line);q.key('ret');return g['wait'](pattern,s,timeout)
 def launch():
  s=len(t());shell('run valentine',r'\[shell.program.launch\]')
  ready=g['wait'](r'\[valentine.ready\] target=(\d+) task=(\d+)',s,90)
  display=g['wait'](rf'\[shell.program.launch\] target={ready[1]} task={ready[2]} surface=(\d+)',s,60)
  g['focus'](int(display[1]));time.sleep(.7)
  return ready[1],ready[2],s
 def close(target,task):
  s=len(t());q.key('f11');g['wait'](rf'\[valentine.closed\] target={target} task={task}',s,60)
  g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,90)
 def shot(name):time.sleep(.8);q.cmd('screendump',{'filename':str(a.out/f'{name}.ppm')});time.sleep(.4)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(1)
  target,task,s=launch();g['wait'](r'\[valentine.loaded\] messages=0',s,60);close(target,task)
  shell('mkdir /data/valentine/Travel',r'\[spindle.fs.result\] kind=3 status=0')
  shell('mkdir /data/valentine/Travel/2026',r'\[spindle.fs.result\] kind=3 status=0')
  m=shell(f'fetch http://10.0.2.2:{PORT}/inbox.eml /data/valentine/inbox.eml',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240)
  assert m[1].startswith('Saved'),m[1]
  m=shell(f'fetch http://10.0.2.2:{PORT}/inbox.eml /data/valentine/Travel/trip.eml',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240)
  assert m[1].startswith('Saved'),m[1]
  m=shell(f'fetch http://10.0.2.2:{PORT}/inbox.eml /data/valentine/Travel/2026/nested.eml',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240)
  assert m[1].startswith('Saved'),m[1]
  target,task,s=launch()
  g['wait'](r'\[valentine.opened\] file=inbox.eml bytes=\d+ subject=Dinner on Friday',s,60)
  g['wait'](r'\[valentine.loaded\] messages=1',s,60)
  shot('valentine-inbox')
  pixels=(a.out/'valentine-inbox.ppm').read_bytes();assert pixels.startswith(b'P6\n') and len(pixels)>100000
  valentine_surface=int(g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,5)[1])
  started=len(t());files_launch=shell('files',r'\[shell.program.launch\] target=\d+ task=\d+ surface=(\d+)',90)
  files_surface=int(files_launch[1]);g['wait'](r'\[files.app.ready\] target=\d+ task=\d+ root=/ portal=1',started,60)
  g['focus'](valentine_surface);time.sleep(.4)
  s=len(t());ctrl_alt('a');g['wait'](r'\[valentine.attachment.selected\] index=1 total=1',s,30)
  s=len(t());ctrl_alt('s');g['wait'](r'\[files.pick.asked\] row=\d+ mode=1 ',s,60)
  g['wait'](r'\[files.app.question\] program=valentine mode=Save',s,60)
  g['focus'](files_surface);time.sleep(.4);ctrl('s');time.sleep(.4)
  shot('valentine-attachment-picker')
  s=len(t());q.key('ret');g['wait'](r'\[files.pick.granted\] row=\d+ path=/menu.pdf',s,60)
  g['wait'](r'\[valentine.attachment.saved\] path=@/menu.pdf bytes=9',s,60)
  g['focus'](valentine_surface);time.sleep(.4);shot('valentine-attachment-saved')
  s=len(t());ctrl_alt('s');g['wait'](r'\[files.pick.asked\] row=\d+ mode=1 ',s,60)
  g['focus'](files_surface);time.sleep(.4);ctrl('s');time.sleep(.4)
  s=len(t());q.key('ret');g['wait'](r'\[files.pick.granted\] row=\d+ path=/menu.pdf',s,60)
  g['wait'](r'\[valentine.attachment.exists\] path=@/menu.pdf',s,60)
  g['focus'](valentine_surface);time.sleep(.4);shot('valentine-attachment-existing-name')
  shell('cat /menu.pdf',r'\[spindle.fs.cat\] %PDF-1.4',90)
  g['focus'](valentine_surface);time.sleep(.4)
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'shift'},{'type':'qcode','data':'slash'}],'hold-time':80})
  g['wait'](r'\[valentine.shortcuts\] open=1',s,60);shot('valentine-shortcuts')
  help_pixels=(a.out/'valentine-shortcuts.ppm').read_bytes()
  assert help_pixels.startswith(b'P6\n') and help_pixels!=pixels,'Valentine shortcut reference not painted'
  s=len(t());q.key('esc');g['wait'](r'\[valentine.shortcuts\] open=0',s,60)
  s=len(t());keys('dinner');g['wait'](r'\[valentine.search\] query=dinner matches=1',s,60);shot('valentine-search')
  s=len(t());q.key('esc');g['wait'](r'\[valentine.search\] query= matches=1',s,60)
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'r'}],'hold-time':80})
  g['wait'](r'\[valentine.loaded\] messages=1',s,60)
  s=len(t());ctrl('n');keys('bob@example.org');q.key('ret');keys('plans');q.key('ret');keys('see you at seven.')
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'shift'},{'type':'qcode','data':'slash'}],'hold-time':80})
  g['wait'](r'\[valentine.shortcuts\] open=1',s,60)
  s=len(t());q.key('esc');g['wait'](r'\[valentine.shortcuts\] open=0',s,60)
  ctrl('s');g['wait'](r'\[valentine.draft.saved\] path=draft-[^ ]+\.eml bytes=\d+',s,60)
  shot('valentine-draft')
  s=len(t());ctrl('z');g['wait'](r'\[valentine.draft.undo\] changed=1 dirty=1',s,60)
  shot('valentine-draft-undo')
  s=len(t());ctrl('y');g['wait'](r'\[valentine.draft.redo\] changed=1 dirty=0',s,60)
  close(target,task)
  target,task,s=launch();g['wait'](r'\[valentine.loaded\] messages=2',s,60)
  surface=int(g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,5)[1])
  keys('plans');g['wait'](r'\[valentine.search\] query=plans matches=1',s,60)
  s=len(t());q.key('ret');g['wait'](r'\[valentine.opened\] file=draft-[^ ]+\.eml bytes=\d+ subject=plans',s,60)
  s=len(t());ctrl('e');time.sleep(.4);shot('valentine-draft-reopened');q.key('esc')
  s=len(t());ctrl('m');g['wait'](r'\[valentine.account.unavailable\] reason=multipass-service-not-booted',s,60)
  # The wire contract is present, but no boot-routed Multipass/TLS flow service
  # exists yet. The guest must continue to show the honest unavailable state.
  shot('valentine-account-status')
  s=len(t());ctrl('g');g['wait'](r'\[valentine.folder\] name=Travel messages=1',s,60)
  g['wait'](r'\[valentine.opened\] file=Travel/trip.eml bytes=\d+ subject=Dinner on Friday',s,60)
  shot('valentine-folder')
  s=len(t());ctrl('g');g['wait'](r'\[valentine.folder\] name=Travel/2026 messages=1',s,60)
  g['wait'](r'\[valentine.opened\] file=Travel/2026/nested.eml bytes=\d+ subject=Dinner on Friday',s,60)
  shot('valentine-nested-folder')
  nested_pixels=(a.out/'valentine-nested-folder.ppm').read_bytes()
  assert nested_pixels.startswith(b'P6\n') and nested_pixels!=(a.out/'valentine-folder.ppm').read_bytes()
  # Another writer takes the name after the new composer selected it. The
  # first save must choose a new name without replacing that writer's mail.
  s=len(t());ctrl('n')
  taken=g['wait'](r'\[valentine.draft.new\] path=(Travel/2026/draft-[^ ]+\.eml)',s,60)[1]
  m=shell(f'fetch http://10.0.2.2:{PORT}/inbox.eml /data/valentine/{taken}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240)
  assert m[1].startswith('Saved'),m[1]
  g['focus'](surface);time.sleep(.4)
  keys('bob@example.org');q.key('ret');keys('nested note');q.key('ret');keys('See the itinerary.')
  s=len(t());ctrl('s')
  saved_nested=g['wait'](r'\[valentine.draft.saved\] path=(Travel/2026/draft-[^ ]+\.eml) bytes=\d+',s,60)[1]
  assert saved_nested != taken,(saved_nested,taken)
  shot('valentine-draft-collision-saved')
  q.key('esc')
  shell(f'cat /data/valentine/{taken}',r'\[spindle.fs.cat\] .*Dinner on Friday',90)
  g['focus'](surface);time.sleep(.4)
  s=len(t());ctrl('g');g['wait'](r'\[valentine.folder\] name= messages=2',s,60)
  close(target,task)
  # Let the idle composer save a draft, then cut QEMU power with the window open.
  # No Close or Ctrl+S runs for this draft.
  target,task,s=launch()
  s=len(t());ctrl('n');keys('reboot@example.org');q.key('ret');keys('power cut');q.key('ret');keys('saved after crash')
  saved=g['wait'](r'\[valentine.draft.saved\] path=(draft-[^ ]+\.eml) bytes=\d+',s,90)
  shot('valentine-before-powercut')
  assert '[valentine.fatal]' not in t() and 'KERNEL PANIC' not in t()
  g['stop']()
  g=boot(a.iso,disk,a.out,'valentine-reboot',a.resolution);t=g['text'];q=g['q']
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  shell(f'cat /data/valentine/{saved[1]}',r'\[spindle.fs.cat\] saved after crash',90)
  target,task,s=launch();g['wait'](r'\[valentine.loaded\] messages=3',s,60)
  s=len(t());keys('power cut');g['wait'](r'\[valentine.search\] query=power cut matches=1',s,60)
  s=len(t());q.key('ret');g['wait'](r'\[valentine.opened\] file=draft-[^ ]+\.eml bytes=\d+ subject=power cut',s,60)
  shot('valentine-autosaved-reboot')
  close(target,task)
  assert '[valentine.fatal]' not in t() and 'KERNEL PANIC' not in t()
 finally:g['stop']();server.shutdown()
 print({'ok':True,'messages':1,'powercut_draft':True})
if __name__=='__main__':main()
