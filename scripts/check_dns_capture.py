#!/usr/bin/env python3
"""Independent Ethernet/IPv4/UDP evidence for the fixed-name app DNS probe."""
import hashlib,struct
from check_tcp_capture import frames,checksum

LOCAL=bytes([10,0,2,15]);PEER=bytes([10,0,2,2]);PORT=46079
QUESTION=b'\x05probe\x05sexos\x04test\0\0\x01\0\x01'
POSITIVE_MODES=('compressed','cname','cname-reordered','cname-sections')
ALIAS_FAILURES=('cname-cycle','cname-conflict','bad-pointer','cname-unrelated')
MODES=('echo','wrong-context','malformed','truncated','nxdomain','silence','cancel-retry','close')+POSITIVE_MODES+ALIAS_FAILURES

def alias_response(ticket,kind):
 """Explicit independent wire fixtures; no production DNS parser is imported."""
 packet=bytearray(struct.pack('!6H',ticket,0x8180,1,0,0,0)+QUESTION)
 def ptr(offset):return struct.pack('!H',0xc000|offset)
 def rr(owner,typ,ttl,data):
  start=len(packet);packet.extend(owner+struct.pack('!HHIH',typ,1,ttl,len(data)));rdata=len(packet);packet.extend(data);return start,rdata
 if kind=='compressed':
  rr(b'\x05PrObE\xc0\x12',1,60,PEER);answers=1
 elif kind=='cname-reordered':
  first,_=rr(b'\x04page\xc0\x12',1,60,PEER);rr(b'\xc0\x0c',5,7,ptr(first));answers=2
 elif kind=='cname-cycle':
  _,target=rr(b'\xc0\x0c',5,23,b'\x03mid\xc0\x12');rr(ptr(target),5,7,b'\xc0\x0c');answers=2
 elif kind=='cname-conflict':
  _,target=rr(b'\xc0\x0c',5,23,b'\x03mid\xc0\x12');rr(b'\xc0\x0c',5,7,b'\x04page\xc0\x12');rr(ptr(target),1,60,PEER);answers=3
 elif kind=='bad-pointer':rr(b'\xc0\x0c',5,7,b'\xc0\x04');answers=1
 elif kind=='cname-unrelated':
  rr(b'\xc0\x0c',5,23,b'\x03mid\xc0\x12');rr(b'\x05other\xc0\x12',1,60,PEER);answers=2
 elif kind in('cname','cname-sections'):
  _,mid=rr(b'\xc0\x0c',5,23,b'\x03mid\xc0\x12');_,page=rr(ptr(mid),5,7,b'\x04page\xc0\x12');rr(ptr(page),1,60,PEER);answers=3
  if kind=='cname-sections':
   _,ns=rr(b'\x05sexos\xc0\x18',2,1,b'\x02ns\xc0\x12');rr(ptr(ns),1,1,bytes([203,0,113,55]));packet[8:12]=struct.pack('!HH',1,1)
 else:raise ValueError('unknown bounded alias fixture')
 packet[6:8]=struct.pack('!H',answers)
 if len(packet)>512:raise ValueError('DNS fixture exceeds datagram bound')
 return bytes(packet)

def query(ticket):
 if not 1<=ticket<=17535:raise ValueError('finite nonzero DNS ticket required')
 return struct.pack('!6H',ticket,0x100,1,0,0,0)+QUESTION

def response(ticket,kind='answer'):
 """Independent peer fixtures, never a replacement for captured packets."""
 if kind in POSITIVE_MODES+ALIAS_FAILURES:return alias_response(ticket,kind)
 packet=bytearray(struct.pack('!6H',ticket,0x8180,1,1,0,0)+QUESTION+b'\xc0\x0c'+struct.pack('!HHIH',1,1,60,4)+PEER)
 if kind=='wrong-id':packet[:2]=struct.pack('!H',ticket+100)
 elif kind=='wrong-question':packet[13:18]=b'other'
 elif kind=='malformed':packet[36:38]=struct.pack('!H',28) # unsupported AAAA answer with four bytes
 elif kind=='truncated':packet[2]|=2
 elif kind=='nxdomain':packet=bytearray(struct.pack('!6H',ticket,0x8183,1,0,0,0)+QUESTION)
 elif kind!='answer':raise ValueError('unknown DNS fixture kind')
 return bytes(packet)

def inspect(data,mode,attempts=None,expected=None):
 """Require actual complete packets; recorded host send calls alone cannot pass."""
 if mode not in MODES:raise ValueError('explicit supported DNS evidence mode required')
 errors=[];packets=[];queries=[];responses=[];zero=0
 expected_tickets=[1,2]if mode=='cancel-retry'else[1]
 try:
  declared=None
  if expected is not None:
   if not isinstance(expected,list)or not expected:raise ValueError('explicit DNS exchanges must be a nonempty list')
   expected_tickets=[row['ticket']for row in expected]
   if any(type(t)is not int or not 1<=t<=17535 for t in expected_tickets)or expected_tickets!=sorted(set(expected_tickets)):raise ValueError('explicit DNS tickets must be finite, unique and increasing')
   declared={row['ticket']:bytes.fromhex(row['response_hex'])if row.get('response_hex')else None for row in expected}
   if any(p is not None and (len(p)<12 or len(p)>512 or int.from_bytes(p[:2],'big')!=t)for t,p in declared.items()):raise ValueError('explicit DNS response must bind its transaction ticket and datagram bounds')
  for index,frame in enumerate(frames(data)):
   if len(frame)<14 or frame[12:14]!=b'\x08\x00':continue
   ip=frame[14:]
   if len(ip)<20:raise ValueError('short captured IPv4 header')
   ihl=(ip[0]&15)*4;total=int.from_bytes(ip[2:4],'big')
   if ip[0]>>4!=4 or ihl<20 or total<ihl or len(ip)<total or len(ip)<ihl or checksum(ip[:ihl])!=0:raise ValueError('invalid captured IPv4 header')
   if ip[9]!=17:continue
   # Inspect only the fixed DNS endpoint; unrelated UDP is not a DNS receipt.
   udp=ip[ihl:total]
   if len(udp)<8:raise ValueError('short captured UDP header')
   sport,dport,length,check=struct.unpack_from('!4H',udp)
   outbound=ip[12:16]==LOCAL and ip[16:20]==PEER and dport==PORT and 48001<=sport<=65535
   inbound=ip[12:16]==PEER and ip[16:20]==LOCAL and sport==PORT and 48001<=dport<=65535
   if not(outbound or inbound):continue
   if int.from_bytes(ip[6:8],'big')&0x3fff:raise ValueError('fragmented captured DNS datagram')
   if length!=len(udp)or length<8 or length>520:raise ValueError('invalid captured DNS UDP length')
   if check==0:zero+=1 # IPv4 UDP explicitly permits an absent checksum.
   elif checksum(ip[12:20]+b'\0\x11'+length.to_bytes(2,'big')+udp)!=0:raise ValueError('invalid captured UDP checksum')
   payload=udp[8:];ticket=(sport if outbound else dport)-48000
   item=dict(packet=index,direction='outgoing'if outbound else'incoming',ticket=ticket,guest_port=48000+ticket,peer_port=PORT,payload_hex=payload.hex(),payload_sha256=hashlib.sha256(payload).hexdigest(),udp_checksum='absent_legal_ipv4'if check==0 else'verified')
   packets.append(item)
   if outbound:
    if payload!=query(ticket):raise ValueError('actual query differs from exact fixed-name A/IN ticket-bound query')
    queries.append(item)
   else:
    kind=('explicit'if declared.get(ticket)==payload else None)if declared is not None else next((kind for kind in('answer','wrong-id','wrong-question','malformed','truncated','nxdomain')+POSITIVE_MODES+ALIAS_FAILURES if payload==response(ticket,kind)),None)
    if kind is None:raise ValueError('captured DNS response differs from explicit peer fixtures')
    item['kind']=kind;responses.append(item)
  if [q['ticket']for q in queries]!=expected_tickets:errors.append('actual DNS queries are missing, repeated, reordered, or reused a ticket')
  by_ticket={ticket:[r for r in responses if r['ticket']==ticket]for ticket in expected_tickets}
  if any(r['ticket']not in expected_tickets for r in responses):errors.append('unexpected captured DNS ticket')
  wanted={'echo':['answer'],'wrong-context':['wrong-id','wrong-question','answer'],'malformed':['malformed'],'truncated':['truncated'],'nxdomain':['nxdomain'],'silence':[],'close':[]}
  wanted.update({kind:[kind]for kind in POSITIVE_MODES+ALIAS_FAILURES})
  if declared is not None:
   for ticket in expected_tickets:
    wanted_payload=[]if declared[ticket]is None else[declared[ticket].hex()]
    if [r['payload_hex']for r in by_ticket[ticket]]!=wanted_payload:errors.append('captured DNS response sequence differs from declared complete bytes')
  elif mode=='cancel-retry':
   if [r['kind']for r in by_ticket[2]]!=['answer']:errors.append('fresh retry lacks exactly one captured valid response')
   if any(r['kind']!='answer'for r in by_ticket[1]):errors.append('old cancelled tuple has an unexpected packet')
   if len(by_ticket[1])>1:errors.append('duplicate delayed old response')
   if by_ticket[1]and by_ticket[1][0]['packet']<queries[1]['packet']:errors.append('old response was not delayed until fresh retry')
  elif [r['kind']for r in by_ticket[1]]!=wanted[mode]:errors.append('captured response sequence does not qualify the requested DNS mode')
  for reply in responses:
   origin=next((q for q in queries if q['ticket']==reply['ticket']),None)
   if origin is None or reply['packet']<=origin['packet']:errors.append('DNS response precedes its captured query')
  # A sendto return is retained separately and joined to actual packets. An old
  # NAT mapping may already be gone after cancellation; do not claim delivery.
  joined=[]
  for attempt in attempts or []:
   matches=[r['packet']for r in responses if r['ticket']==attempt['ticket']and r['payload_hex']==attempt['response_hex']]
   joined.append(dict(ticket=attempt['ticket'],kind=attempt['kind'],captured_packets=matches,host_send_succeeded=attempt.get('sent_bytes')==len(bytes.fromhex(attempt['response_hex']))))
 except(ValueError,IndexError,KeyError,TypeError,struct.error)as exc:
  errors.append(str(exc));joined=[]
 return dict(ok=not errors,errors=errors,mode=mode,queries=queries,responses=responses,packets=packets,host_attempt_capture_join=joined,ipv4_udp_zero_checksums=zero,scope='Independent full captured Ethernet/IPv4/UDP lengths and checksums, exact query/response bytes, fixed endpoint and finite ticket/transaction binding. IPv4 UDP zero checksums are explicit. Host send attempts are distinct from packets; no DNS cache, public resolver, web, TLS, physical containment or pixel claim.')
