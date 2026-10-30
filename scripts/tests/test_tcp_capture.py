import json,struct,unittest,hashlib,sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_tcp_capture import inspect,checksum,frames
F=Path(__file__).parent/'fixtures/sexnet-tcp-r76-first-flow.pcap'
META=F.with_suffix('.json')
def expected():return [dict(payload_hex=bytes(((n*17+7)&255)for n in range(16641)).hex(),aborted=False)]
def packets():return list(frames(F.read_bytes()))
def pcap(values):
 raw=F.read_bytes()[:24]
 for i,value in enumerate(values):raw+=struct.pack('<IIII',1,i,len(value),len(value))+value
 return raw

def recompute(value):
 ip=14;ihl=(value[ip]&15)*4;end=ip+int.from_bytes(value[ip+2:ip+4],'big');tcp=ip+ihl
 value[tcp+16:tcp+18]=b'\0\0';pseudo=value[ip+12:ip+20]+b'\0\x06'+(end-tcp).to_bytes(2,'big')+value[tcp:end]
 value[tcp+16:tcp+18]=checksum(bytes(pseudo)).to_bytes(2,'big');return bytes(value)

class TcpCapture(unittest.TestCase):
 def test_actual_guest_capture_and_full_endpoint_bytes_match(self):
  meta=json.loads(META.read_text());self.assertEqual(hashlib.sha256(F.read_bytes()).hexdigest(),meta['fixture_sha256']);r=inspect(F.read_bytes(),expected());self.assertTrue(r['ok'],r['errors']);self.assertEqual(r['connections'][0]['incoming_bytes'],16641);self.assertEqual(r['connections'][0]['outgoing_bytes'],16641)
 def test_known_checksum_vector_and_corrupt_packet_fail(self):
  self.assertEqual(checksum(bytes.fromhex('45000073000040004011b861c0a80001c0a800c7')),0)
  self.assertNotEqual(checksum(bytes.fromhex('45000073000040004011b860c0a80001c0a800c7')),0)
  values=packets();bad=bytearray(values[0]);bad[24]^=1;values[0]=bytes(bad);self.assertFalse(inspect(pcap(values),expected())['ok'])
 def test_missing_syn_or_handshake_ack_cannot_be_replaced_by_data_markers(self):
  values=packets();no_syn=[v for v in values if not(v[14+(v[14]&15)*4+13]&2)];self.assertFalse(inspect(pcap(no_syn),expected())['ok'])
  server=None;client=None
  for v in values:
   off=14+(v[14]&15)*4;t=v[off:]
   if t[13]&2:
    if int.from_bytes(t[:2],'big')==46076:server=int.from_bytes(t[4:8],'big')
    else:client=int.from_bytes(t[4:8],'big')
  bad=[]
  for v in values:
   off=14+(v[14]&15)*4;t=v[off:];modified=bytearray(v)
   if int.from_bytes(t[2:4],'big')==46076 and t[13]&0x10 and int.from_bytes(t[4:8],'big')==(client+1)&0xffffffff and int.from_bytes(t[8:12],'big')==(server+1)&0xffffffff:
    modified[off+8:off+12]=server.to_bytes(4,'big');v=recompute(modified)
   bad.append(v)
  r=inspect(pcap(bad),expected());self.assertFalse(r['ok']);self.assertIn('missing real three-way handshake acknowledgment',r['errors'])
 def test_conflicting_retransmission_or_missing_payload_is_rejected(self):
  values=packets()
  for v in values:
   ih=(v[14]&15)*4;off=14+ih;t=v[off:];total=int.from_bytes(v[16:18],'big');th=(t[12]>>4)*4
   if int.from_bytes(t[:2],'big')==46076 and total>ih+th:
    bad=bytearray(v);bad[14+total-1]^=1;duplicate=recompute(bad);self.assertFalse(inspect(pcap(values+[duplicate]),expected())['ok']);values.remove(v);self.assertFalse(inspect(pcap(values),expected())['ok']);break
  else:self.fail('native fixture lacks guest data')
 def test_truncation_wrong_stream_count_and_wrong_expected_bytes_fail(self):
  for data in [b'',F.read_bytes()[:-1],F.read_bytes()[:30]]:self.assertFalse(inspect(data,expected())['ok'])
  self.assertFalse(inspect(F.read_bytes(),expected()*2)['ok']);wrong=expected();wrong[0]['payload_hex']='00';self.assertFalse(inspect(F.read_bytes(),wrong)['ok'])

class OutboundCapture(unittest.TestCase):
 def test_real_guest_initiated_stream_and_explicit_role(self):
  f=F.parent/'sexnet-outbound-r78.pcap';data=f.read_bytes();meta=json.loads(f.with_suffix('.json').read_text());self.assertEqual(hashlib.sha256(data).hexdigest(),meta['fixture_sha256'])
  wanted=[dict(payload_hex=bytes((n*17+78)&255 for n in range(1025)).hex())]
  result=inspect(data,wanted,guest_ports=[48001],initiator='guest');self.assertTrue(result['ok'],result['errors']);self.assertEqual(result['connections'][0]['guest_port'],48001);self.assertEqual(result['connections'][0]['incoming_bytes'],1025);self.assertEqual(result['connections'][0]['outgoing_bytes'],1025)
  self.assertFalse(inspect(data,wanted,guest_ports=[48001],initiator='peer')['ok']);self.assertFalse(inspect(data,wanted,guest_ports=[48002],initiator='guest')['ok'])
 def test_outbound_handshake_and_real_payload_required(self):
  data=(F.parent/'sexnet-outbound-r78.pcap').read_bytes();values=list(frames(data));wanted=[dict(payload_hex=bytes((n*17+78)&255 for n in range(1025)).hex())]
  no_syn=[v for v in values if len(v)<34 or v[12:14]!=b'\x08\x00' or v[23]!=6 or not(v[14+(v[14]&15)*4+13]&2)]
  self.assertFalse(inspect(pcap(no_syn),wanted,guest_ports=[48001],initiator='guest')['ok'])
  wrong=[dict(payload_hex='00')];self.assertFalse(inspect(data,wrong,guest_ports=[48001],initiator='guest')['ok'])

if __name__=='__main__':unittest.main()
