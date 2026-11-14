#!/usr/bin/env python3
"""A minimal HTTP BitTorrent tracker for the Swingers gates: remembers every peer that
announces an info-hash and answers with the others in compact form. `--host-ip` lets
the gate rewrite loopback addresses to what a QEMU guest can reach (10.0.2.2)."""
import argparse,ipaddress,socket,struct,threading,urllib.parse
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
PEERS={};LOCK=threading.Lock()
def bencode(v):
    if isinstance(v,int):return b'i%de'%v
    if isinstance(v,bytes):return b'%d:'%len(v)+v
    if isinstance(v,dict):return b'd'+b''.join(bencode(k)+bencode(v[k]) for k in sorted(v))+b'e'
    raise TypeError(v)
class H(BaseHTTPRequestHandler):
    def log_message(self,*a):pass
    def do_GET(self):
        u=urllib.parse.urlsplit(self.path);q=urllib.parse.parse_qs(u.query.encode('latin-1'),keep_blank_values=True)
        ih=q.get(b'info_hash',[b''])[0];port=int(q.get(b'port',[b'0'])[0]);ip=self.client_address[0]
        if ip.startswith('127.') and ARGS.host_ip:ip=ARGS.host_ip
        with LOCK:
            s=PEERS.setdefault(ih,{})
            if q.get(b'event',[b''])[0]==b'stopped':s.pop((ip,port),None)
            elif port:s[(ip,port)]=1
            others=[p for p in s if p!=(ip,port)]
        v4=b''.join(socket.inet_aton(i)+struct.pack('>H',p) for i,p in others if ipaddress.ip_address(i).version==4)
        body=bencode({b'interval':ARGS.interval,b'peers':v4,b'complete':len(others),b'incomplete':0})
        self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        print('announce',ih.hex(),ip,port,len(others),flush=True)
p=argparse.ArgumentParser();p.add_argument('--port',type=int,default=6969);p.add_argument('--host-ip');p.add_argument('--interval',type=int,default=5);ARGS=p.parse_args()
ThreadingHTTPServer(('0.0.0.0',ARGS.port),H).serve_forever()
