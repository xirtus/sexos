#!/usr/bin/env python3
"""Independent PCAP/checksum/sequence reassembly for the private TCP endpoint."""
import struct
from pathlib import Path
LOCAL=bytes([10,0,2,15]);PORT=46076

def checksum(data):
 if len(data)%2:data+=b'\0'
 total=sum(struct.unpack('!'+str(len(data)//2)+'H',data))
 while total>>16:total=(total&65535)+(total>>16)
 return (~total)&65535

def fingerprint(data):
 value=0xcbf29ce484222325
 for b in data:value=((value^b)*0x100000001b3)&0xffffffffffffffff
 return value

def frames(data):
 if len(data)<24:raise ValueError('short PCAP header')
 order={b'\xd4\xc3\xb2\xa1':'<',b'\xa1\xb2\xc3\xd4':'>'}.get(data[:4])
 if order is None:raise ValueError('unsupported PCAP magic')
 if struct.unpack_from(order+'I',data,20)[0]!=1:raise ValueError('capture is not Ethernet')
 offset=24
 while offset<len(data):
  if offset+16>len(data):raise ValueError('short PCAP record')
  sec,usec,size,original=struct.unpack_from(order+'IIII',data,offset);offset+=16
  if size!=original or offset+size>len(data):raise ValueError('truncated PCAP packet')
  yield data[offset:offset+size];offset+=size

def inspect(data,expected,guest_ports=None,initiator="peer"):
 ports=(PORT,)if guest_ports is None else tuple(guest_ports)
 if initiator not in("peer","guest")or not ports:raise ValueError("explicit TCP capture role/ports required")
 errors=[];connections={};packets=0
 try:
  for frame in frames(data):
   if len(frame)<14 or frame[12:14]!=b'\x08\x00':continue
   ip=frame[14:];ihl=(ip[0]&15)*4;length=int.from_bytes(ip[2:4],'big')
   if ip[0]>>4!=4 or ihl<20 or len(ip)<length or length<ihl or checksum(ip[:ihl])!=0:raise ValueError('invalid captured IPv4 header')
   if ip[9]!=6:continue
   if int.from_bytes(ip[6:8],'big')&0x3fff:raise ValueError('fragmented captured TCP')
   tcp=ip[ihl:length];pseudo=ip[12:20]+b'\0\x06'+len(tcp).to_bytes(2,'big')
   if len(tcp)<20 or checksum(pseudo+tcp)!=0:raise ValueError('invalid captured TCP checksum')
   header=(tcp[12]>>4)*4
   if header<20 or header>len(tcp):raise ValueError('bad captured TCP header length')
   sport,dport=struct.unpack_from('!HH',tcp);seq,ack=struct.unpack_from('!II',tcp,4);flags=tcp[13];payload=tcp[header:]
   outbound=ip[12:16]==LOCAL and sport in ports;inbound=ip[16:20]==LOCAL and dport in ports
   if not(outbound or inbound):continue
   packets+=1;key=(ip[16:20].hex(),dport,sport)if outbound else(ip[12:16].hex(),sport,dport)
   item=connections.setdefault(key,dict(guest_port=key[2],incoming={},outgoing={},client_isn=None,server_isn=None,handshake=False,fin_in=0,fin_out=0,rst_in=0,rst_out=0))
   side='outgoing'if outbound else'incoming';isn='server_isn'if outbound else'client_isn'
   if flags&2:
    if item[isn]not in[None,seq]:raise ValueError('same tuple reused without a separately identified connection')
    item[isn]=seq
    responder=outbound if initiator=='peer'else inbound
    other='client_isn'if outbound else'server_isn'
    if responder and(item[other]is None or not flags&0x10 or ack!=(item[other]+1)&0xffffffff):raise ValueError('invalid SYN/ACK acknowledgment')
    if not responder and flags&0x10:raise ValueError('initiator SYN has unexpected ACK')
   final_ack=inbound if initiator=='peer'else outbound
   other='client_isn'if outbound else'server_isn'
   if final_ack and flags&0x10 and not flags&2 and item[other]is not None and item[isn]is not None and seq==(item[isn]+1)&0xffffffff and ack==(item[other]+1)&0xffffffff:item['handshake']=True
   if payload:
    if item[isn]is None:raise ValueError('payload precedes captured SYN')
    start=(seq-item[isn]-1)&0xffffffff
    if start>1024*1024 or start+len(payload)>1024*1024:raise ValueError('captured stream exceeds qualification bound')
    for n,b in enumerate(payload,start):
     if n in item[side]and item[side][n]!=b:raise ValueError('conflicting retransmitted byte')
     item[side][n]=b
   item['fin_out'if outbound else'fin_in']+=bool(flags&1);item['rst_out'if outbound else'rst_in']+=bool(flags&4)
  if len(connections)!=len(expected):errors.append('wrong actual TCP connection count')
  results=[]
  for item,wanted in zip(connections.values(),expected):
   request=bytes.fromhex(wanted['payload_hex']);received=bytes(item['incoming'].get(n,0)for n in range(len(item['incoming'])));sent=bytes(item['outgoing'].get(n,0)for n in range(len(item['outgoing'])))
   if set(item['incoming'])!=set(range(len(item['incoming'])))or set(item['outgoing'])!=set(range(len(item['outgoing']))):errors.append('hole in captured stream')
   if item['client_isn']is None or item['server_isn']is None or not item['handshake']:errors.append('missing real three-way handshake acknowledgment')
   if wanted.get('aborted'):
    if initiator=='guest':
     if not sent or not request.startswith(sent)or not sent.startswith(received)or(not received and not wanted.get('allow_empty_reply')):errors.append('aborted outbound connection differs from explicit prefix/no-response policy')
    elif not received or not sent or not request.startswith(received)or not received.startswith(sent):errors.append('aborted connection bytes differ from explicit observed prefix')
   elif received!=request or sent!=request or not item['fin_in']or not item['fin_out']:errors.append('full captured stream/FIN differs from actual endpoint payload')
   results.append(dict(guest_port=item['guest_port'],incoming_bytes=len(received),outgoing_bytes=len(sent),incoming_hash=hex(fingerprint(received)),outgoing_hash=hex(fingerprint(sent)),fin_in=item['fin_in'],fin_out=item['fin_out'],rst_in=item['rst_in'],rst_out=item['rst_out']))
 except(ValueError,IndexError,struct.error)as error:errors.append(str(error));results=[]
 return dict(ok=not errors,errors=errors,tcp_packets=packets,connections=results,scope='Independent full PCAP checksums and TCP sequence reassembly join real endpoint bytes; aborted prefix is distinguished from completed streams. No app authorization, DNS/HTTP/TLS or hostile/physical containment proof.')
