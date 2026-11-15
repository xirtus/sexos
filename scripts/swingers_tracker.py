#!/usr/bin/env python3
"""A minimal HTTP and UDP (BEP 15, same port number) BitTorrent tracker for the Swingers gates: remembers every peer that
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
def record(ih,ip,port,stopped):
    """Remembers (or forgets) a peer; returns the others and their compact v4 form."""
    if ip.startswith('127.') and ARGS.host_ip:ip=ARGS.host_ip
    with LOCK:
        s=PEERS.setdefault(ih,{})
        if stopped:s.pop((ip,port),None)
        elif port:s[(ip,port)]=1
        others=[p for p in s if p!=(ip,port)]
    return others,b''.join(socket.inet_aton(i)+struct.pack('>H',p) for i,p in others if ipaddress.ip_address(i).version==4)
def udp():
    u=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);u.bind(('0.0.0.0',ARGS.port))
    while True:
        b,a=u.recvfrom(2048)
        if len(b)>=16 and b[:8]==struct.pack('>Q',0x41727101980) and struct.unpack('>I',b[8:12])[0]==0:
            u.sendto(struct.pack('>IIQ',0,struct.unpack('>I',b[12:16])[0],0x1234),a)
        elif len(b)>=98 and struct.unpack('>I',b[8:12])[0]==1:
            tid=b[12:16];ih=b[16:36];event=struct.unpack('>I',b[80:84])[0];port=struct.unpack('>H',b[96:98])[0]
            others,v4=record(ih,a[0],port,event==3)
            u.sendto(struct.pack('>I',1)+tid+struct.pack('>III',ARGS.interval,0,len(others))+v4,a)
            print('udp announce',ih.hex(),a[0],port,len(others),flush=True)
class H(BaseHTTPRequestHandler):
    def log_message(self,*a):pass
    def do_GET(self):
        u=urllib.parse.urlsplit(self.path);q=urllib.parse.parse_qs(u.query.encode('latin-1'),keep_blank_values=True)
        ih=q.get(b'info_hash',[b''])[0];port=int(q.get(b'port',[b'0'])[0]);ip=self.client_address[0]
        others,v4=record(ih,ip,port,q.get(b'event',[b''])[0]==b'stopped')
        body=bencode({b'interval':ARGS.interval,b'peers':v4,b'complete':len(others),b'incomplete':0})
        self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        print('announce',ih.hex(),ip,port,len(others),flush=True)
p=argparse.ArgumentParser();p.add_argument('--port',type=int,default=6969);p.add_argument('--host-ip');p.add_argument('--interval',type=int,default=5);ARGS=p.parse_args()
threading.Thread(target=udp,daemon=True).start()
ThreadingHTTPServer(('0.0.0.0',ARGS.port),H).serve_forever()
