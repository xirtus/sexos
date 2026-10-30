"""Adverse synthetic PCAP tests; native evidence is produced separately."""
import struct,sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_dns_capture import inspect,query,response,MODES

def checksum(b):
 if len(b)%2:b+=b'\0'
 n=sum(int.from_bytes(b[i:i+2],'big')for i in range(0,len(b),2))
 while n>>16:n=(n&65535)+(n>>16)
 return (~n)&65535

def packet(ticket,body,incoming=False,zero=False,peer_port=46079):
 local=b'\x0a\0\x02\x0f';peer=b'\x0a\0\x02\x02';src,dst=(peer,local)if incoming else(local,peer)
 sport,dport=(peer_port,48000+ticket)if incoming else(48000+ticket,peer_port)
 udp=bytearray(struct.pack('!4H',sport,dport,len(body)+8,0)+body)
 if not zero:udp[6:8]=(checksum(src+dst+b'\0\x11'+len(udp).to_bytes(2,'big')+udp) or 65535).to_bytes(2,'big')
 ip=bytearray(b'\x45\0'+(20+len(udp)).to_bytes(2,'big')+b'\0\0\x40\0\x40\x11\0\0'+src+dst);ip[10:12]=checksum(ip).to_bytes(2,'big')
 return b'\x52\x54\0\x12\x34\x56\x02\0\0\0\0\x01\x08\0'+ip+udp

def pcap(packets):
 out=struct.pack('<IHHIIII',0xa1b2c3d4,2,4,0,0,65535,1)
 for n,p in enumerate(packets):out+=struct.pack('<4I',1,n,len(p),len(p))+p
 return out

def fixture(mode):
 out=[packet(1,query(1))]
 if mode=='cancel-retry':out.extend([packet(2,query(2)),packet(1,response(1),True),packet(2,response(2),True)])
 elif mode=='wrong-context':out.extend(packet(1,response(1,k),True)for k in ['wrong-id','wrong-question','answer'])
 elif mode not in ['silence','close']:out.append(packet(1,response(1,'answer'if mode=='echo'else mode),True))
 return out

class DnsCapture(unittest.TestCase):
 def test_all_explicit_modes_require_complete_wire_contract(self):
  for mode in MODES:
   with self.subTest(mode=mode):r=inspect(pcap(fixture(mode)),mode);self.assertTrue(r['ok'],r['errors'])
 def test_no_query_or_duplicate_query_cannot_pass(self):
  rows=fixture('echo')
  for v in [rows[1:],rows+[rows[0]]]:self.assertFalse(inspect(pcap(v),'echo')['ok'])
 def test_checksum_corruption_and_truncated_capture_fail(self):
  rows=fixture('echo');bad=bytearray(rows[1]);bad[-1]^=1;self.assertFalse(inspect(pcap([rows[0],bytes(bad)]),'echo')['ok'])
  data=pcap(rows)
  for cut in [0,23,len(data)-1]:self.assertFalse(inspect(data[:cut],'echo')['ok'])
 def test_wrong_context_only_or_wrong_endpoint_cannot_supply_answer(self):
  for wrong in [packet(1,response(1,'wrong-id'),True),packet(1,response(1),True,peer_port=46080)]:self.assertFalse(inspect(pcap([packet(1,query(1)),wrong]),'echo')['ok'])
 def test_wrong_context_order_and_early_answer_rejected(self):
  rows=fixture('wrong-context');self.assertFalse(inspect(pcap([rows[0],rows[-1],*rows[1:-1]]),'wrong-context')['ok']);self.assertFalse(inspect(pcap([rows[1],rows[0]]),'echo')['ok'])
 def test_response_cannot_qualify_silence_or_close(self):
  for mode in ['silence','close']:self.assertFalse(inspect(pcap(fixture('echo')),mode)['ok'])
 def test_cancelled_tuple_and_host_send_attempt_are_distinct(self):
  rows=fixture('cancel-retry');attempt=dict(ticket=1,kind='answer',response_hex=response(1).hex(),sent_bytes=len(response(1)))
  r=inspect(pcap([rows[0],rows[1],rows[3]]),'cancel-retry',[attempt]);self.assertTrue(r['ok'],r['errors']);self.assertEqual(r['host_attempt_capture_join'][0]['captured_packets'],[])
  self.assertFalse(inspect(pcap([rows[0],rows[2],rows[1],rows[3]]),'cancel-retry')['ok'])
 def test_legal_zero_udp_checksum_is_explicit(self):
  r=inspect(pcap([packet(1,query(1),zero=True),packet(1,response(1),True,zero=True)]),'echo');self.assertTrue(r['ok'],r['errors']);self.assertEqual(r['ipv4_udp_zero_checksums'],2)
if __name__=='__main__':unittest.main()
