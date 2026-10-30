"""Checker fixtures only; these do not manufacture native guest evidence."""
import hashlib,json,struct,sys,tempfile,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import network_contention_gate as gate

def packet(ticket=1,destination=gate.REMOTE,port=46078,sequence=100):
 tcp=bytearray(struct.pack('!HHIIBBHHH',48000+ticket,port,sequence,0,0x50,2,4096,0,0))
 tcp[16:18]=gate.checksum(gate.LOCAL+destination+b'\0\6'+len(tcp).to_bytes(2,'big')+tcp).to_bytes(2,'big')
 ip=bytearray(b'\x45\0'+(20+len(tcp)).to_bytes(2,'big')+b'\0\1\x40\0\x40\6\0\0'+gate.LOCAL+destination)
 ip[10:12]=gate.checksum(ip).to_bytes(2,'big')
 return b'\x02\0\0\0\0\1\x52\x54\0\x12\x34\x56\x08\0'+ip+tcp
def pcap(*packets):
 return struct.pack('<IHHIIII',0xa1b2c3d4,2,4,0,0,65536,1)+b''.join(struct.pack('<IIII',1,n,len(p),len(p))+p for n,p in enumerate(packets))
def trace(frame,owner=12,task=120,ticket=1,seq=7):
 return (f'[spindle.net.intent] target={owner} task={task} command=net-probe accepted=1 seq={seq} fixed_peer=1\n'
 f'[net.job.state] owner=Some(Identity {{ pd: {owner}, task: {task} }}) ticket={ticket} from=Idle to=Staging\n'
 f'[spindle.net.reply] target={owner} task={task} tag=120 seq={seq} value={ticket} observed=1\n'
 f'[net.job.egress.submit] ticket={ticket} sequence=3 length={len(frame)} hash={gate.fingerprint(frame):#x} scope=dma_admission_not_delivery\n')
def cleanup(target=12,task=120):
 return ''.join(f'[app.service.ack] target={target} task={task} holder=13 role={role} acked=31 required=31\n'for role in [1,2,4,8,16])+f'[net.dns.records.retired] target={target} task={task} remaining=0 before_cleanup_ack=1\n[net.app.cleanup.ack] target={target} task={task} client_leases=0 shared_cache=retained ok=1\n[sext.app.pool.ready] target={target} task={task} ok=1\n'

class ContentionOracle(unittest.TestCase):
 def test_syn_tuple_checksums_and_retransmission_identity(self):
  frame=packet();self.assertEqual(len(gate.syn_records(pcap(frame,frame))),1)
  self.assertEqual(gate.syn_records(pcap(frame))[0]['ticket'],1)
  with self.assertRaises(ValueError):gate.syn_records(pcap(frame,packet(sequence=101)))
 def test_wrong_remote_or_port_and_corrupt_checksum_rejected(self):
  for frame in [packet(destination=bytes([10,0,2,99])),packet(port=46079),packet()[:-1]+b'\1']:
   with self.assertRaises(ValueError):gate.syn_records(pcap(frame))
 def test_owner_join_needs_actual_frame_identity_and_exact_intent_sequence(self):
  frame=packet();value=trace(frame);self.assertTrue(gate.owner_joins(value,pcap(frame),[(12,120)])['ok'])
  for bad in [value.replace('task=120','task=121'),value.replace('task: 120','task: 121'),value.replace('tag=120 seq=7','tag=120 seq=8'),value.replace('value=1','value=2'),value.replace('hash=0x','hash=0x1')]:
   self.assertFalse(gate.owner_joins(bad,pcap(frame),[(12,120)])['ok'])
 def test_historical_browser_label_is_not_spindle_owner_evidence(self):
  frame=packet();self.assertFalse(gate.owner_joins('[browser.net.job.released] ticket=1 view=Verified private_job=0',pcap(frame),[(12,120)])['ok'])
 def test_denied_extra_flow_or_wrong_ticket_is_detected(self):
  frame=packet();value=trace(frame)
  self.assertFalse(gate.owner_joins(value,pcap(frame,packet(2)),[(12,120)])['ok'])
  self.assertFalse(gate.owner_joins(trace(packet(2),ticket=2),pcap(packet(2)),[(12,120)])['ok'])
 def test_cleanup_requires_all_five_roles_exact_task_and_order(self):
  good=cleanup();self.assertEqual(gate.cleanup_receipts(good,12,120,0)['roles'],[1,2,4,8,16])
  for bad in [good.replace('role=16','role=8'),good.replace('task=120','task=12'),good.replace('task=120 holder','task=121 holder'),good.replace('remaining=0','remaining=1'),good.replace('[net.dns.records.retired]','[not.retired]')]:
   with self.assertRaises(ValueError):gate.cleanup_receipts(bad,12,120,0)
  lines=good.splitlines();lines[-2],lines[-1]=lines[-1],lines[-2]
  with self.assertRaises(ValueError):gate.cleanup_receipts('\n'.join(lines),12,120,0)
 def test_visible_text_joins_latest_left_right_panes_and_rejects_missing_bytes(self):
  value=''
  for sid,data in zip([154,160,161,162],[b'NET / ve\n',b'rified  \n',b'> echo  \n',b'ready   \n']):
   value+=f'[spindle.view.text] target=12 sid={sid} sequence=2 paint=3 cols=8 rows=1 bytes=9\n[spindle.view.bytes] target=12 sid={sid} seq=2 paint=3 off=0 hex={data.hex()}\n'
  self.assertIn(b'NET / verified',gate.visible_spindle_text(value,12))
  with self.assertRaises(ValueError):gate.visible_spindle_text(value.replace('sid=160 seq=2','sid=999 seq=2'),12)
 def test_binding_requires_exact_image_sources_and_frozen_runner(self):
  with tempfile.TemporaryDirectory()as directory:
   root=Path(directory);(root/'scripts').mkdir();(root/'source.rs').write_bytes(b'source');(root/'scripts/network_contention_gate.py').write_bytes(b'runner');iso=root/'image.iso';iso.write_bytes(b'image');path=root/'binding.json'
   sha=lambda data:hashlib.sha256(data).hexdigest()
   binding=dict(iso_sha256=sha(b'image'),build_provenance=dict(build_gate='PASS',source_image_binding='sealed_build_this_run',sources_unchanged_during_build=True,iso_sha256=sha(b'image')),sources={'source.rs':sha(b'source')},runner_sha256={'scripts/network_contention_gate.py':sha(b'runner')});path.write_text(json.dumps(binding))
   self.assertEqual(gate.frozen_binding(root,iso,path)['production_files'],1)
   (root/'source.rs').write_bytes(b'changed')
   with self.assertRaises(ValueError):gate.frozen_binding(root,iso,path)

if __name__=='__main__':unittest.main()
