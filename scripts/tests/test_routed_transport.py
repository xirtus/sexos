"""Pure oracle checks using mutations of an existing actual native TCP PCAP.

These are checker fixtures, never evidence that the routed native run passed.
"""
import copy,struct,sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import routed_transport_gate as gate
from check_tcp_capture import frames,checksum,inspect

FIXTURE=Path(__file__).parent/'fixtures/sexnet-tcp-r76-first-flow.pcap'
def record(direction,data,n=0):return dict(direction=direction,host_time=100+n/1000,hex=data.hex(),sha256=gate.digest(data))
def recompute(frame):
 value=bytearray(frame);ih=(value[14]&15)*4;end=14+int.from_bytes(value[16:18],'big');start=14+ih
 value[24:26]=b'\0\0';value[24:26]=checksum(value[14:14+ih]).to_bytes(2,'big')
 value[start+16:start+18]=b'\0\0';value[start+16:start+18]=checksum(value[26:34]+b'\0\x06'+(end-start).to_bytes(2,'big')+value[start:end]).to_bytes(2,'big');return bytes(value)
def captures():
 result=[record('guest_tx',gate.arp_query()),record('peer_fixture',gate.arp_reply(gate.GATEWAY_MAC),1)]
 for raw in frames(FIXTURE.read_bytes()):
  if raw[12:14]!=b'\x08\0' or raw[23]!=6:continue
  value=bytearray(raw);outgoing=value[26:30]==gate.LOCAL
  if outgoing:value[:6]=gate.GATEWAY_MAC;value[6:12]=gate.GUEST_MAC;value[30:34]=gate.REMOTE
  else:value[:6]=gate.GUEST_MAC;value[6:12]=gate.REMOTE_MAC;value[26:30]=gate.REMOTE
  result.append(record('guest_tx'if outgoing else'peer_fixture',recompute(value),len(result)))
 return result
def replace(row,data):row.update(hex=data.hex(),sha256=gate.digest(data))
def guest_tcp(rows):return next(row for row in rows if row['direction']=='guest_tx'and gate.decode_tcp(bytes.fromhex(row['hex']))is not None)
def renewal(rows):
 result=copy.deepcopy(rows);result.append(record('guest_tx',gate.arp_query(),len(result)));result.append(record('peer_fixture',gate.arp_reply(gate.RENEWED_MAC),len(result)))
 frame=bytearray.fromhex(guest_tcp(rows)['hex']);frame[:6]=gate.RENEWED_MAC;result.append(record('guest_tx',bytes(frame),len(result)));return result

class RoutedOracle(unittest.TestCase):
 def test_expiry_requires_post_gateway_rx_raw_guest_tick_progress(self):
  rows=[record('guest_tx',gate.arp_query()),record('peer_fixture',gate.arp_reply(gate.GATEWAY_MAC))]
  marker=f'[sexnet.ether.rx] sequence=1 index=0 length=60 hash={gate.fingerprint(gate.arp_reply(gate.GATEWAY_MAC)):#x}'
  def wait(n):return f'\n[sexnet.receive.wait] before={n-1} after={n} ticks=2 count=1024 kind=timer_assisted_poll irq=0 device=1 reply_pending=0\n'
  baseline=wait(426);self.assertIsNone(gate.expiry_receipt(wait(99999)+marker+baseline+wait(45742),rows))
  self.assertIsNone(gate.expiry_receipt(marker+baseline+wait(60426),rows))
  receipt=gate.expiry_receipt(marker+baseline+wait(60427),rows);self.assertEqual((receipt['fill_upper_tick'],receipt['target_tick'],receipt['observed_tick']),(426,60427,60427))
  self.assertIsNone(gate.expiry_receipt(marker.replace('hash=','absent=')+baseline+wait(60427),rows))
 def test_only_exact_bounded_startup_echo_syn_can_coexist_with_listener_proof(self):
  rows=captures();packet=bytearray.fromhex(guest_tcp(rows)['hex']);off=14+(packet[14]&15)*4
  packet[30:34]=gate.GATEWAY;packet[off:off+4]=struct.pack('!HH',48001,46078);packet[off+8:off+12]=b'\0'*4;packet[off+13]=2
  packet=recompute(packet);rows.append(record('guest_tx',packet));self.assertTrue(gate.inspect_routing(rows,'normal')['ok'])
  self.assertEqual(gate.inspect_routing(rows,'normal')['startup_echo_syn'],1)
  for offset,value in [(off,48002),(off+2,46077)]:
   changed=bytearray(packet);changed[offset:offset+2]=value.to_bytes(2,'big');bad=captures()+[record('guest_tx',recompute(changed))];self.assertFalse(gate.inspect_routing(bad,'normal')['ok'])
  changed=bytearray(packet);changed[off+13]=0x14;self.assertFalse(gate.inspect_routing(captures()+[record('guest_tx',recompute(changed))],'normal')['ok'])
  self.assertFalse(gate.inspect_routing(captures()+[record('guest_tx',packet)]*(gate.MAX_STARTUP_SYN+1),'normal')['ok'])
 def test_peer_tcp_has_real_checksums_explicit_remote_mac_and_wrapping_sequence(self):
  frame=gate.peer_tcp(62001,0xfffffff0,7,0x18,b'peer bytes');segment=gate.decode_tcp(frame)
  self.assertEqual((segment['source'],segment['destination'],segment['sport'],segment['dport'],segment['seq'],segment['body']),(gate.REMOTE,gate.LOCAL,62001,gate.PORT,0xfffffff0,b'peer bytes'))
  self.assertEqual(frame[6:12],gate.REMOTE_MAC);self.assertNotEqual(gate.REMOTE_MAC,gate.GATEWAY_MAC)
  for port,body in [(0,b''),(65536,b''),(62001,b'x'*257)]:
   with self.assertRaises(ValueError):gate.peer_tcp(port,1,0,2,body)
 def test_arp_exact_gateway_question_and_distinct_answer(self):
  query=gate.decode_arp(gate.arp_query());reply=gate.decode_arp(gate.arp_reply(gate.GATEWAY_MAC))
  self.assertEqual((query['opcode'],query['sender'],query['target']),(1,gate.LOCAL,gate.GATEWAY));self.assertEqual((reply['opcode'],reply['sender'],reply['target'],reply['mac']),(2,gate.GATEWAY,gate.LOCAL,gate.GATEWAY_MAC))
 def test_truncated_corrupt_fragmented_and_invalid_headers_reject(self):
  frame=gate.peer_tcp(62001,1,0,2,b'1234567')
  for cut in range(14,len(frame)):
   with self.assertRaises(ValueError):gate.decode_tcp(frame[:cut])
  for offset in [24,50]:
   value=bytearray(frame);value[offset]^=1
   with self.assertRaises(ValueError):gate.decode_tcp(bytes(value))
  value=bytearray(frame);value[20:22]=b'\x20\0';value=recompute(value)
  with self.assertRaises(ValueError):gate.decode_tcp(value)
 def test_actual_native_capture_mutation_checks_full_sequence_and_echo(self):
  rows=captures();r=gate.inspect_routing(rows,'normal');self.assertTrue(r['ok'],r)
  wanted=[dict(payload_hex=bytes((n*17+7)&255 for n in range(16641)).hex())]
  result=inspect(gate.pcap(rows),wanted);self.assertTrue(result['ok'],result['errors']);self.assertEqual(result['connections'][0]['outgoing_bytes'],16641)
 def test_remote_mac_wrong_ip_or_checksum_cannot_substitute_for_gateway_route(self):
  for kind in ['remote-mac','wrong-ip','checksum']:
   rows=captures();row=guest_tcp(rows);frame=bytearray.fromhex(row['hex'])
   if kind=='remote-mac':frame[:6]=gate.REMOTE_MAC
   elif kind=='wrong-ip':frame[30:34]=gate.GATEWAY;frame=bytearray(recompute(frame))
   else:frame[24]^=1
   replace(row,bytes(frame));self.assertFalse(gate.inspect_routing(rows,'normal')['ok'],kind)
 def test_remote_arp_or_missing_gateway_answer_cannot_pass(self):
  rows=captures();value=bytearray.fromhex(rows[0]['hex']);value[38:42]=gate.REMOTE;replace(rows[0],bytes(value));self.assertFalse(gate.inspect_routing(rows,'normal')['ok'])
  self.assertFalse(gate.inspect_routing(captures()[2:],'normal')['ok']);self.assertFalse(gate.inspect_routing([row for row in captures()if gate.decode_arp(bytes.fromhex(row['hex']))is None],'normal')['ok'])
 def test_recovery_needs_fresh_query_answer_and_actual_new_mac(self):
  rows=renewal(captures());self.assertTrue(gate.inspect_routing(rows,'recover')['ok'])
  self.assertFalse(gate.inspect_routing(captures(),'recover')['ok']);missing=copy.deepcopy(rows);del missing[-3];self.assertFalse(gate.inspect_routing(missing,'recover')['ok'])
  old=copy.deepcopy(rows);value=bytearray.fromhex(old[-1]['hex']);value[:6]=gate.GATEWAY_MAC;replace(old[-1],bytes(value));self.assertFalse(gate.inspect_routing(old,'recover')['ok'])
  pending=copy.deepcopy(rows);pending[-2],pending[-1]=pending[-1],pending[-2];self.assertFalse(gate.inspect_routing(pending,'recover')['ok'])
 def test_silent_failed_syn_must_have_no_guest_tcp_and_is_explicitly_excluded_from_success_streams(self):
  rows=captures();rows.insert(1,record('peer_fixture',gate.peer_tcp(gate.SILENT_PORT,70000,0,2)));self.assertTrue(gate.inspect_routing(rows,'silence')['ok'])
  packet=bytearray.fromhex(guest_tcp(rows)['hex']);off=14+(packet[14]&15)*4;packet[off+2:off+4]=gate.SILENT_PORT.to_bytes(2,'big');rows.append(record('guest_tx',recompute(packet)));self.assertFalse(gate.inspect_routing(rows,'silence')['ok'])
  self.assertNotEqual(gate.pcap(rows),gate.pcap(rows,exclude_port=gate.SILENT_PORT))
 def test_missing_data_cannot_pass_by_route_mac_and_counters_alone(self):
  rows=captures();wanted=[dict(payload_hex=bytes((n*17+7)&255 for n in range(16641)).hex())]
  for index,row in enumerate(rows):
   segment=gate.decode_tcp(bytes.fromhex(row['hex']))
   if row['direction']=='guest_tx'and segment and segment['body']:del rows[index];break
  self.assertFalse(inspect(gate.pcap(rows),wanted)['ok'])
 def test_sampled_descriptor_hash_index_and_completion_are_independent_checks(self):
  rows=captures();text=''
  for direction,labels in [('guest_tx',['tx.submit','tx.done']),('peer_fixture',['rx'])]:
   values=[bytes.fromhex(row['hex'])for row in rows if row['direction']==direction]
   for n,frame in enumerate(values,1):
    if n<=64 or n&(n-1)==0:
     for label in labels:text+=f'[sexnet.ether.{label}] sequence={n} index={(n-1)%8} length={len(frame)} hash={gate.fingerprint(frame):#x}\n'
  self.assertTrue(gate.custody(text,rows)['ok']);self.assertFalse(gate.custody(text.replace('index=0','index=1'),rows)['ok']);self.assertFalse(gate.custody(text.replace('tx.done','absent.done'),rows)['ok'])
 def test_capture_hash_bounds_and_unsupported_mode_fail(self):
  rows=captures();rows[0]['sha256']='00';self.assertFalse(gate.inspect_routing(rows,'normal')['ok'])
  self.assertFalse(gate.inspect_routing(captures(),'invented')['ok']);self.assertFalse(gate.inspect_routing([captures()[0]]*(gate.MAX_PACKETS+1),'normal')['ok'])

if __name__=='__main__':unittest.main()
