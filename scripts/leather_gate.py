#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Real Weft SDK sample windows: greet; edit/save/sync/reload; decode picture;
fetch an exact HTTP fixture through a held network grant; paint and close each."""
import argparse,http.server,json,os,re,shutil,threading,time
from pathlib import Path
from install_program import install,fnv
from program_egui_gate import colours
ROOT=Path(__file__).resolve().parents[1]
BODY=b'Leather reached the network.\n'
class Handler(http.server.BaseHTTPRequestHandler):
    requests=0
    def do_GET(self):
        assert self.path=='/leather',self.path
        Handler.requests+=1
        self.send_response(200);self.send_header('Content-Length',str(len(BODY)));self.end_headers();self.wfile.write(BODY)
    def log_message(self,*args):pass

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ('iso','source-disk','out'):p.add_argument('--'+name,required=True,type=Path)
    p.add_argument('--resolution',default='1280x800');a=p.parse_args()
    server=http.server.ThreadingHTTPServer(('0.0.0.0',0),Handler)
    threading.Thread(target=server.serve_forever,daemon=True).start()
    os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
    os.environ.setdefault('SEXOS_GATE_BOUND','1200')
    from quil_autosave_gate import boot
    out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
    names=['hello','stitch','lantern','postcard']
    install(disk,[('leather-'+name,(ROOT/'iso_root/programs'/('leather-'+name)).read_bytes())for name in names])
    g=boot(a.iso,disk,out,'leather',a.resolution);q=g['q'];t=g['text'];rw,rh=map(int,a.resolution.split('x'))
    def chord(*keys):
        q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':key}}}for down,part in ((True,keys),(False,tuple(reversed(keys))))for key in part]})
    def type_text(text):
        for c in text:
            if c in '+:':chord('shift','equal' if c=='+' else 'semicolon')
            else:q.key({' ':'spc','-':'minus','.':'dot','/':'slash'}.get(c,c))
            time.sleep(.06)
    def launch(name,args='',network=False):
        g['focus'](153);time.sleep(.3);start=len(t());type_text('run '+('+net 'if network else '')+'leather-'+name+(' '+args if args else ''));q.key('ret')
        m=g['wait'](rf'\[leather.ready\] app={name} target=(\d+) task=(\d+) frames=([1-9]\d*)',start,300)
        target,task=int(m[1]),int(m[2]);launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',start,60)
        surface=int(launch[1]);g['focus'](surface);time.sleep(1)
        pos={n:(int(x),int(y))for n,x,y in re.findall(rf'\[leather.control\] app={name} name=(\w+) x=(-?\d+) y=(-?\d+)',t()[start:])}
        tile=list(map(int,re.findall(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t())[-1]))
        def click(control):
            x,y=pos[control];x+=tile[0]+8;y+=tile[1]+36
            q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.2)
            for down in (True,False):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.2)
        def close():
            time.sleep(.5);path=out/(name+'.ppm');q.cmd('screendump',{'filename':str(path)});time.sleep(.3)
            n=len(colours(path.read_bytes(),tile));assert n>8,(name,n)
            start=len(t());g['focus'](surface);q.key('f11');g['wait'](rf'\[app.private.retired\] target={target} task={task} ',start,120)
        return start,click,close
    try:
        g['wait'](r'\[app.private.retired\] target=41 ',0,180)
        start,click,close=launch('hello');click('primary');g['wait'](r'\[leather.greet\] ok=1',start);close()
        start,click,close=launch('stitch');click('content');chord('ctrl','a');type_text('copied leather');click('add');type_text('second line');click('primary')
        text=b'copied leather\nsecond line';g['wait'](rf'\[leather.saved\] bytes={len(text)} synced=1',start)
        click('load');g['wait'](rf'\[leather.loaded\] bytes={len(text)} lines=2 hash={fnv(text):#x}',start)
        # Saving to a different existing path must not inherit authorization.
        click('path');chord('ctrl','a');type_text('copy.txt');copy_start=len(t());click('primary');g['wait'](rf'\[leather.saved\] bytes={len(text)} synced=1',copy_start)
        click('path');chord('ctrl','a');type_text('draft.txt');click('content');chord('ctrl','a');type_text('do not overwrite')
        guard=len(t());click('primary');g['wait'](r'\[leather.error\] app=stitch what=Load an existing document before replacing it',guard)
        click('load');g['wait'](rf'\[leather.loaded\] bytes={len(text)} lines=2 hash={fnv(text):#x}',guard);close()
        start,click,close=launch('stitch','draft.txt');g['wait'](rf'\[leather.loaded\] bytes={len(text)} lines=2 hash={fnv(text):#x} startup=1',start);close()
        start,click,close=launch('lantern');g['wait'](r'\[leather.image\] width=96 height=64',0);close()
        start,click,close=launch('postcard',f'http://10.0.2.2:{server.server_port}/leather',True);click('primary')
        g['wait'](rf'\[leather.fetched\] status=200 bytes={len(BODY)} hash={fnv(BODY):#x}',start,120);assert Handler.requests==1;close()
        assert '[leather.fatal]'not in t()
        result=dict(ok=True,apps=names,text_bytes=len(text),http_bytes=len(BODY));(out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(result)
    finally:g['stop']();server.shutdown();server.server_close()
if __name__=='__main__':main()
