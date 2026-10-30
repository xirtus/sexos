import copy,unittest
from test_nic_prepare import prepared
from check_nic_command import inspect
from ethernet_peer_gate import validate,packet,query,reply,fingerprint,LOCAL,GATEWAY,PEER

MAC=bytes.fromhex('525400123456')
def initialized():
 t=prepared().replace('[sexnet.nic.prepare] from=Cold','[pci.device.identity] pd=13 slot=19 vendor=32902 product=4307 runtime_mint=0\n[sexnet.nic.prepare] from=Cold')
 return t+'''[sexnet.nic.identity.denied] requests=4 output_zero=1 ok=1
[pci.command.apply] pd=13 slot=19 before=3 after=3 set=0 clear=0 changed=0
[sexnet.ether.phase] from=Check to=Zero failure=None
[sexnet.ether.phase] from=Zero to=Descriptors failure=None
[sexnet.ether.phase] from=Descriptors to=Registers failure=None
[sexnet.ether.phase] from=Registers to=Enable failure=None
[pci.command.apply] pd=13 slot=19 before=3 after=7 set=4 clear=0 changed=1
[sexnet.ether.phase] from=Enable to=Link failure=None
[sexnet.ether.phase] from=Link to=Running failure=None
[sexnet.ether.ready] model=I82574 region=2 budget=262144 used=40960 descriptors=8 buffer=2048 polling=cooperative timing_verified=1 network=0 dns=0 tcp=0 http=0 tls=0
[app.surface.presented] target=15 surface=300 sequence=1 ok=1
'''

def exchanges():
 rows=[dict(guest_tx=query(MAC).hex(),peer_fixture=packet(MAC,PEER,2,GATEWAY,MAC,LOCAL).hex())]
 for i in range(1,25):
  ip=bytes([10,0,2,32+i]);pad=bytes(((i*17+n)&255)for n in range(18))
  rows.append(dict(guest_tx=reply(MAC,PEER,ip).hex(),peer_fixture=packet(b'\xff'*6,PEER,1,ip,b'\0'*6,LOCAL,pad).hex()))
 return rows

def trace(rows):
 t=initialized()
 for i,row in enumerate(rows):
  tx=bytes.fromhex(row['guest_tx']);rx=bytes.fromhex(row['peer_fixture'])
  for kind in ['submit','done']:t+=f'[sexnet.ether.tx.{kind}] sequence={i+1} index={i%8} length={len(tx)} hash={fingerprint(tx):#x}\n'
  t+=f'[sexnet.ether.rx] sequence={i+1} index={i%8} length={len(rx)} hash={fingerprint(rx):#x}\n'
 return t

class EthernetEvidence(unittest.TestCase):
 def test_exact_lease_initialization_bus_master_and_full_duplex_bytes(self):
  rows=exchanges();self.assertTrue(validate(trace(rows),rows,MAC)['ok']);self.assertFalse(inspect(initialized(),preparation=True)['ok'])
 def test_identity_or_bus_master_order_model_budget_and_online_claim_fail(self):
  for old,new in [('product=4307 runtime','product=4110 runtime'),('model=I82574','model=I82540'),('used=40960','used=262144'),('set=4 clear=0','set=6 clear=0'),('changed=1','changed=0'),('from=Registers to=Enable','from=Enable to=Registers'),('failure=None','failure=Some(Deadline)'),('network=0 dns=0 tcp=0 http=0 tls=0','network=1 dns=1 tcp=1 http=1 tls=1')]:
   self.assertFalse(inspect(initialized().replace(old,new),ethernet=True)['ok'],new)
 def test_missing_duplicate_wrong_sequence_index_hash_or_capture_fails(self):
  rows=exchanges();t=trace(rows)
  for old,new in [('sequence=25','sequence=24'),('index=7','index=8'),('length=60','length=64'),('hash=0x','hash=0x1'),('[sexnet.ether.rx]','[absent.rx]')]:self.assertFalse(validate(t.replace(old,new),rows,MAC)['ok'],new)
  bad=copy.deepcopy(rows);bad[-1]['guest_tx']=bad[0]['guest_tx'];self.assertFalse(validate(t,bad,MAC)['ok']);self.assertFalse(validate(t,rows[:-1],MAC)['ok']);self.assertFalse(validate(t+t,rows,MAC)['ok'])
 def test_legacy_zero_timing_readback_is_explicit_and_never_allowed_for_82574(self):
  self.assertFalse(inspect(initialized().replace('timing_verified=1','timing_verified=0'),ethernet=True)['ok'])
  legacy=initialized().replace('product=4307','product=4110').replace('device=10d3','device=100e').replace('0x10D3','0x100E').replace('model=I82574','model=I82540').replace('timing_verified=1','timing_verified=0')
  self.assertTrue(inspect(legacy,ethernet=True)['ok'])
 def test_wrong_arp_payload_padding_or_peer_fixture_cannot_match_by_counter_only(self):
  for field in ['guest_tx','peer_fixture']:
   rows=exchanges();bad=bytearray.fromhex(rows[5][field]);bad[-1]^=1;rows[5][field]=bad.hex()
   self.assertFalse(validate(trace(rows),rows,MAC)['ok'])
  self.assertEqual(fingerprint(b'hello'),0xa430d84680aabd0b)

if __name__=='__main__':unittest.main()
