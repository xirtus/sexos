#!/usr/bin/env python3
"""IPv6 and program sockets: `run +net talk check ADDRESS 7` on a network that has only
IPv6. The machine takes a global address by SLAAC from a router advertisement, then
the program connects by TCP to an echo server on the link and gets its bytes back,
does a UDP echo round trip, and takes a TCP connection on port 7070 and echoes it. Then `run +net peek http://[ADDRESS]/`
loads a page over IPv6 through the same HTTP exchanges the browser uses.

The far side is this script: a peer on QEMU's socket network backend that speaks
IPv6 frames itself (router, neighbor discovery, a small TCP and UDP echo server, and
a TCP client), because the build hosts have no IPv6 for QEMU's user network to use.

The network has no IPv4 at all, so names resolve only through what IPv6 says: the
router advertisement carries the resolver (RDNSS) and sets the O flag, so the machine
also asks stateless DHCPv6 (an Information-Request answered here). Then
`run +net peek http://six.test/` must look the name up over IPv6 and load the page."""
import argparse,os,re,shutil,socket,struct,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]

PEER_MAC=bytes.fromhex('020000000001');GUEST_MAC=bytes.fromhex('525400123456')
PREFIX=bytes.fromhex('fd00005e00000001')
PEER_LL=bytes.fromhex('fe80'+'00'*13+'01');PEER=PREFIX+bytes(7)+b'\1'
def eui64(mac):return bytes([mac[0]^2])+mac[1:3]+b'\xff\xfe'+mac[3:]
GUEST=PREFIX+eui64(GUEST_MAC);GUEST_LL=bytes.fromhex('fe80000000000000')+eui64(GUEST_MAC)
PAGE=b'<html><body><h1>Hello over IPv6</h1><p>Fetched by a program, by address.</p></body></html>'
ALL_NODES=bytes.fromhex('ff02'+'00'*13+'01')
ALL_DHCP=bytes.fromhex('ff02'+'00'*11+'010002')
NAME=b'six.test'
def text6(a):return socket.inet_ntop(socket.AF_INET6,a) if hasattr(socket,'inet_ntop') else a.hex()

def checksum(data):
 if len(data)%2:data+=b'\0'
 s=sum(struct.unpack('!%dH'%(len(data)//2),data))
 while s>>16:s=(s&0xffff)+(s>>16)
 return ~s&0xffff
def ip6(src,dst,nh,payload,hop=64):
 # The upper-layer checksum covers a pseudo-header; its field sits at a known offset.
 at={58:2,6:16,17:6}[nh];payload=bytearray(payload);payload[at:at+2]=b'\0\0'
 payload[at:at+2]=struct.pack('!H',checksum(src+dst+struct.pack('!IxxxB',len(payload),nh)+bytes(payload)))
 return struct.pack('!IHBB',6<<28,len(payload),nh,hop)+src+dst+bytes(payload)
def ether(dst,payload):return dst+PEER_MAC+b'\x86\xdd'+payload
def mcast_mac(a):return b'\x33\x33'+a[12:]

class Peer:
 """One end of QEMU's socket backend: length-prefixed Ethernet frames."""
 def __init__(self):
  self.server=socket.socket();self.server.bind(('127.0.0.1',0));self.server.listen(1);self.port=self.server.getsockname()[1]
  self.conn=None;self.lock=threading.Lock();self.log=[];self.tcp={};self.client=None;self.udp=[];self.ra_sent=0;self.rs_seen=0;self.ns_seen=0;self.dhcp6=[];self.dns=[]
  threading.Thread(target=self.serve,daemon=True).start()
 def send(self,frame):
  frame=frame.ljust(60,b'\0')
  with self.lock:self.conn.sendall(struct.pack('!I',len(frame))+frame)
 def ra(self):
  options=b'\x01\x01'+PEER_MAC+struct.pack('!BBBBIII',3,4,64,0xC0,86400,14400,0)+PREFIX+bytes(8)
  # RDNSS (RFC 8106): this peer resolves names. O flag: ask DHCPv6 for the rest.
  options+=struct.pack('!BBHI',25,3,0,600)+PEER
  body=struct.pack('!BBHBBHII',134,0,0,64,0x40,1800,0,0)+options
  self.send(ether(mcast_mac(ALL_NODES),ip6(PEER_LL,ALL_NODES,58,body,255)));self.ra_sent+=1
 def serve(self):
  self.conn,_=self.server.accept();buf=b''
  # Unsolicited advertisements too, as routers send them, in case a solicitation is missed.
  def beacon():
   while True:
    time.sleep(3)
    try:self.ra()
    except OSError:return
  threading.Thread(target=beacon,daemon=True).start()
  while True:
   data=self.conn.recv(65536)
   if not data:return
   buf+=data
   while len(buf)>=4 and len(buf)>=4+struct.unpack('!I',buf[:4])[0]:
    n=struct.unpack('!I',buf[:4])[0];frame=buf[4:4+n];buf=buf[4+n:]
    try:self.frame(frame)
    except Exception as e:self.log.append('error '+repr(e))
 def frame(self,f):
  if f[12:14]!=b'\x86\xdd':return
  ip=f[14:];length=struct.unpack('!H',ip[4:6])[0];nh=ip[6];hop=ip[7];src=ip[8:24];dst=ip[24:40];body=ip[40:40+length]
  if nh==58:
   kind=body[0]
   if kind==133:self.rs_seen+=1;self.ra()
   elif kind==135 and body[8:24] in(PEER,PEER_LL) and src!=bytes(16):
    self.ns_seen+=1
    na=struct.pack('!BBHI',136,0,0,0xE0000000)+body[8:24]+b'\x02\x01'+PEER_MAC
    self.send(ether(f[6:12],ip6(body[8:24],src,58,na,255)))
   elif kind==128 and dst==PEER:self.send(ether(f[6:12],ip6(PEER,src,58,b'\x81'+body[1:])))
  elif nh==17 and dst==ALL_DHCP and struct.unpack('!H',body[2:4])[0]==547:self.dhcpv6(f[6:12],src,body[8:])
  elif nh==17 and dst==PEER and struct.unpack('!H',body[2:4])[0]==53:self.resolve(f[6:12],src,struct.unpack('!H',body[:2])[0],body[8:])
  elif nh==17 and dst==PEER:
   sport,dport=struct.unpack('!HH',body[:4]);payload=body[8:]
   self.udp.append((text6(src),sport,payload))
   if dport==7:self.send(ether(f[6:12],ip6(PEER,src,17,struct.pack('!HHHH',7,sport,8+len(payload),0)+payload)))
  elif nh==6 and dst==PEER:self.segment(f[6:12],src,body)
 def dhcpv6(self,mac,src,m):
  # Stateless DHCPv6: answer an Information-Request with this peer as the resolver.
  if m[0]!=11:return
  self.dhcp6.append(m)
  client=b'';at=4
  while at+4<=len(m):
   code,n=struct.unpack('!HH',m[at:at+4])
   if code==1:client=m[at:at+4+n]
   at+=4+n
  reply=bytes([7])+m[1:4]+client+struct.pack('!HHHH',2,10,3,1)+PEER_MAC+struct.pack('!HH',23,16)+PEER
  self.send(ether(mac,ip6(PEER_LL,src,17,struct.pack('!HHHH',547,546,8+len(reply),0)+reply)))
 def resolve(self,mac,src,sport,q):
  # One name: AAAA gives this peer, A has no data (there is no IPv4 here).
  name=[];at=12
  while q[at]:name.append(q[at+1:at+1+q[at]]);at+=1+q[at]
  kind=struct.unpack('!H',q[at+1:at+3])[0];question=q[12:at+5];self.dns.append((b'.'.join(name),kind))
  known=b'.'.join(name).lower()==NAME
  answer=struct.pack('!HHHIH',0xc00c,28,1,60,16)+PEER if known and kind==28 else b''
  head=q[:2]+struct.pack('!HHHHH',0x8180 if known else 0x8183,1,1 if answer else 0,0,0)
  r=head+question+answer
  self.send(ether(mac,ip6(PEER,src,17,struct.pack('!HHHH',53,sport,8+len(r),0)+r)))
 def tcp_out(self,mac,src,dst,sport,dport,seq,ack,flags,payload=b''):
  head=struct.pack('!HHIIBBHHH',sport,dport,seq,ack,5<<4,flags,65535,0,0)
  self.send(ether(mac,ip6(src,dst,6,head+payload)))
 def segment(self,mac,src,body):
  sport,dport,seq,ack,off,flags=struct.unpack('!HHIIBB',body[:14]);payload=body[(off>>4)*4:]
  SYN,ACK,FIN,RST,PSH=2,16,1,4,8
  if self.client and dport==self.client['port'] and sport==self.client['remote']:return self.client_segment(mac,src,seq,ack,flags,payload)
  if dport not in(7,80):return
  key=(src,sport,dport);c=self.tcp.get(key)
  if flags&RST:
   if c:c['reset']=True
   return
  if flags&SYN:
   c=self.tcp[key]={'rcv':seq+1,'snd':7000,'got':b'','fin':False,'port':dport}
   self.tcp_out(mac,PEER,src,dport,sport,c['snd'],c['rcv'],SYN|ACK);c['snd']+=1;return
  if not c:return
  def out(flags,data=b''):self.tcp_out(mac,PEER,src,dport,sport,c['snd'],c['rcv'],flags,data);c['snd']+=len(data)+(1 if flags&FIN else 0)
  fresh=payload and seq==c['rcv']
  if fresh:c['rcv']+=len(payload);c['got']+=payload
  if dport==7 and fresh:out(ACK|PSH,payload)
  elif dport==80 and fresh and b'\r\n\r\n' in c['got'] and not c['fin']:
   # One small page, then this side closes (HTTP/1.0).
   body=PAGE;c['fin']=True
   out(ACK|PSH,b'HTTP/1.0 200 OK\r\nContent-Type: text/html\r\nContent-Length: %d\r\n\r\n'%len(body)+body);out(ACK|FIN)
  elif payload:out(ACK)
  if flags&FIN and seq+len(payload)==c['rcv']:
   c['rcv']+=1
   if c['fin']:out(ACK)
   else:c['fin']=True;out(ACK|FIN)
 def connect(self,port,data):
  """Opens a connection to the guest's `port`, sends `data` and closes once it is echoed."""
  self.client={'port':41000,'remote':port,'snd':9000,'rcv':None,'got':b'','data':data,'state':'syn','mac':GUEST_MAC}
  self.tcp_out(GUEST_MAC,PEER,GUEST,41000,port,9000,0,2);self.client['snd']+=1
 def client_segment(self,mac,src,seq,ack,flags,payload):
  c=self.client;SYN,ACK,FIN,RST=2,16,1,4
  if flags&RST:c['state']='reset';return
  if c['state']=='syn' and flags&SYN and flags&ACK:
   c['rcv']=seq+1;c['state']='open'
   self.tcp_out(mac,PEER,GUEST,c['port'],c['remote'],c['snd'],c['rcv'],ACK|8,c['data']);c['snd']+=len(c['data']);return
  if payload and seq==c['rcv']:c['rcv']+=len(payload);c['got']+=payload
  if c['state']=='open' and len(c['got'])>=len(c['data']):
   c['state']='closing';self.tcp_out(mac,PEER,GUEST,c['port'],c['remote'],c['snd'],c['rcv'],ACK|FIN);c['snd']+=1;return
  if flags&FIN and seq+len(payload)==c['rcv']:
   c['rcv']+=1;c['state']='closed'
  if payload or flags&FIN:self.tcp_out(mac,PEER,GUEST,c['port'],c['remote'],c['snd'],c['rcv'],ACK)

peer=Peer()
os.environ['SEXOS_GATE_NIC']=f'-netdev socket,id=n0,connect=127.0.0.1:{peer.port} -device e1000e,netdev=n0,mac=52:54:00:12:34:56'
from install_program import install
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','[':'bracket_left',']':'bracket_right'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 programs=[(n,(ROOT/'iso_root/programs'/n).read_bytes()) for n in ('talk','peek')];assert all(e[:4]==b'\x7fELF' for _,e in programs)
 install(disk,programs)
 g=boot(a.iso,disk,a.out,'ipv6',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t())
  keys(f'run +net talk check {text6(PEER)} 7');q.key('ret')
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,120)
  address=g['wait'](r'\[talk.address\] (fd00:5e:0:1:[0-9a-f:]+)/64',s,120)[1]
  assert address==text6(GUEST),f'SLAAC address {address}, wanted {text6(GUEST)}'
  r=g['wait'](r'\[talk.(tcp\] [^\n]*|failed\] [^\n]*)',s,120)[0];assert 'match=1' in r and 'echoed=30' in r,r
  r=g['wait'](r'\[talk.(udp\] [^\n]*|failed\] [^\n]*)',s,120)[0];assert 'match=1' in r and f'peer={text6(PEER)} port=7' in r,r
  g['wait'](r'\[talk.listening\] port=7070',s,60);time.sleep(.5)
  data=b'a connection the program took, over IPv6'
  peer.connect(7070,data)
  r=g['wait'](r'\[talk.(accepted\] [^\n]*|failed\] [^\n]*)',s,120)[0];assert f'echoed={len(data)}' in r and f'peer={text6(PEER)} port=41000' in r,r
  assert peer.client['got']==data,peer.client
  g['wait'](r'\[talk.done\]',s,60)
  assert re.search(r'\[net.ipv6.slaac\] address='+re.escape(text6(GUEST)),t()),'sexnet did not report its SLAAC address'
  assert any(c['got']==b'hello over the network, by TCP' for c in peer.tcp.values()),peer.tcp
  q.cmd('screendump',{'filename':str(a.out/'talk.ppm')});time.sleep(.5)
  # The HTTP path programs and the browser use (sex_app::net's exchanges), to a bracketed IPv6 URL.
  g['focus'](int(m[3]));time.sleep(.3);s=len(t());q.key('f11');g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s,90)
  g['focus'](153);time.sleep(.5);s=len(t())
  keys(f'run +net peek http://[{text6(PEER)}]/page.html');q.key('ret')
  r=g['wait'](r'\[peek.(got\] [^\n]*|refused\] [^\n]*)',s,120)[0];assert r.startswith('[peek.got] status=200 bytes='+str(len(PAGE))),r
  assert any(c['port']==80 and c['got'].startswith(b'GET /page.html HTTP/1.0\r\nHost: ['+text6(PEER).encode()+b']\r\n') for c in peer.tcp.values()),[c['got'][:80] for c in peer.tcp.values()]
  assert peer.udp and peer.udp[0][2]==b'hello by UDP',peer.udp
  q.cmd('screendump',{'filename':str(a.out/'peek.ppm')});time.sleep(.5)
  # By name, on a network with no IPv4: the resolver came from IPv6 (RDNSS, DHCPv6).
  assert peer.dhcp6,'no DHCPv6 Information-Request'
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,30)
  g['focus'](int(m[3]));time.sleep(.3);s=len(t());q.key('f11');g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s,90)
  g['focus'](153);time.sleep(.5);s=len(t())
  keys('run +net peek http://six.test/page.html');q.key('ret')
  r=g['wait'](r'\[peek.(got\] [^\n]*|refused\] [^\n]*)',s,120)[0];assert r.startswith('[peek.got] status=200 bytes='+str(len(PAGE))),(r,peer.dns)
  assert (NAME,28) in peer.dns,peer.dns
  assert any(c['got'].startswith(b'GET /page.html HTTP/1.0\r\nHost: six.test\r\n') for c in peer.tcp.values()),[c['got'][:80] for c in peer.tcp.values()]
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'address':address,'router_solicitations':peer.rs_seen,'neighbor_solicitations':peer.ns_seen,'dhcpv6_requests':len(peer.dhcp6),'dns_questions':len(peer.dns),'peer_errors':[l for l in peer.log if l.startswith('error')]})
 finally:g['stop']()
if __name__=='__main__':main()
