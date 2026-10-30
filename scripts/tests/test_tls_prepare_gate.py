"""Adverse checker fixtures only; never native TLS, trust or entropy evidence."""
import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import tls_prepare_gate as gate
from check_desktop_pixels import glyph
ROOT=Path(__file__).resolve().parents[2]
SECONDS=1793367907

def command(mode='ready'):
 return ['qemu-system-x86_64','-M','q35','-cpu','max,+pku,-rdseed,-rdrand'if mode=='unsupported'else'max,+pku,+rdseed','-rtc',gate.RTC_INVALID if mode=='invalid-year'else gate.RTC_NORMAL]
def metadata(mode='ready',pending=1):
 lines=['[boot.entropy.source] holder=13 task=13 slot=60 consumer=15 consumer_task=15 source=rdseed attempts=8 runtime_mint=0','[boot.clock.source] holder=13 task=13 slot=61 consumer=15 consumer_task=15 source=q35_cmos_utc attempts=4 precision=1 runtime_mint=0','[browser.entropy.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1','[browser.clock.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1',f'[sexnet.entropy.source] held=1 available={0 if mode=="unsupported"else 1} source=rdseed fallback=0 broker=private_seed32','[sexnet.clock.source] held=1 source=q35_cmos_utc no_ticks_fallback=1 precision=1','[boot.trust.policy] holder=13 task=13 slot=62 consumer=15 consumer_task=15 profile=1 root_bytes=376 runtime_mint=0','[browser.trust.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1']
 for seq in range(1,2 if mode=='close'else 3):
  lines+=['[browser.tls.intent] accepted=1 explicit=1 network=0','[browser.tls.view] state=Preparing authenticated=0 network=0',f'[trust.policy.snapshot] owner=15 task=15 seq={seq} profile=1 root_bytes=376 origin=held_boot_grant','[browser.trust.view] state=Ready(1) immutable_boot_profile=1 transport=0']
  if mode!='unsupported':
   lines.extend([f'[entropy.seed.admitted] owner=15 task=15 ticket={seq} bytes=32 source=held_boot_grant',f'[entropy.source.sample] pd=13 task=13 slot=60 source=rdseed attempts=2 count={seq} private=1'])
   if mode!='close'or pending:lines.extend([f'[entropy.seed.progress] owner=Some(Identity {{ pd: 15, task: 15 }}) ticket={seq} state=Some(Ready) produced=32 consumed=0 failure=0 private=1',f'[entropy.seed.released] owner=15 task=15 ticket={seq} service_payload=erased receipt=retained'])
  if mode=='ready'or(mode=='close'and pending):
   lines.extend([f'[clock.source.snapshot] pd=13 task=13 slot=61 seconds={SECONDS} attempts=1 precision=1 profile=q35_cmos_utc ordinal={seq}',f'[certificate.time.snapshot] owner=15 task=15 seq={seq} seconds={SECONDS} source=q35_cmos_utc precision=1 receipt=retained','[browser.trust.self_check] profile=1 passed=1 fixture=public_controlled_profile authenticated_peer=0',f'[browser.tls.prepared] task_bytes=10000 private_bytes=18248 read=4096 write=2048 tx_bytes=144 utc_snapshot={SECONDS} authenticated=0 network=0','[browser.tls.view] state=HelloPending authenticated=0 network=0'])
  elif mode!='close':lines+=['[browser.tls.view] state=Unavailable authenticated=0 network=0']
  if mode=='close':lines.extend([f'[browser.tls.erased] bytes={18192 if pending else 0} owned_storage=1',f'[browser.tls.closed] pending={pending} terminal=1 owned_storage=erased','[browser.trust.closed] target=15 task=15 pending=0 terminal=1','[trust.policy.retired] target=15 task=15 before_cleanup_ack=1','[entropy.seed.retired] target=15 task=15 before_cleanup_ack=1 private_payload=erased','[certificate.time.retired] target=15 task=15 before_cleanup_ack=1 high_water_retained=1','[net.app.cleanup.ack] target=15 task=15 client_leases=0 shared_cache=retained ok=1'])
  elif seq==1:lines.extend([f'[browser.tls.erased] bytes={18192 if mode=="ready"else 0} owned_storage=1','[browser.tls.view] state=Cancelled authenticated=0 network=0'])
 return '\n'.join(lines)+'\n'
def trace(state,c,w=900,h=650):
 n=len(c['prefix'])+50;base=n+258
 return f'[browser.preview.layout] bytes={n} newline=1 cached_or_sample=1\n[shell.document.viewport] target=15 task=15 surface=300 sequence=7 x=20 y=100 w={w} h={h} enqueued=1\n[browser.viewport] target=15 task=15 sequence=7 w={w} h={h} origin=held_shell\n[browser.tls.layout] base={base} bytes=64 viewport=7 state={state} pixels=unconfirmed\n[browser.view.applied] bytes={base+64} http=0 body_bytes=0 pixels=unconfirmed\n'
def pixels(text,state,c):
 g=gate.tls_geometry(text,state,c);raw=bytearray(1280*800*3);scale=g['scale'];color=c['color'].to_bytes(3,'big')
 for row in g['rows']:
  for col,ch in enumerate(row['text']):
   for gy,bits in enumerate(glyph(c['font'],ch)):
    for gx in range(5):
     if bits&(1<<(4-gx)):
      for dy in range(scale):
       for dx in range(scale):
        x=row['x']+(col*c['advance']+gx)*scale+dx;y=row['y']+gy*scale+dy;at=(y*1280+x)*3;raw[at:at+3]=color
 return b'P6\n1280 800\n255\n'+raw

class TlsPreparationOracle(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.c=gate.load_pixel_contract(ROOT)
 def inspect(self,text,mode='ready'):return gate.inspect_tls(text,mode,command(mode),SECONDS-5,SECONDS+5)
 def test_actual_source_contract_binds_tls_backend_and_prerequisites(self):
  c=gate.tls_contract(ROOT)
  for name in('crates/sex-tls/src/lib.rs','apps/kaleidoscope/src/tls_probe.rs','x86_64-sex.json','Cargo.lock','crates/sex-pdx/src/trust_source.rs','apps/kaleidoscope/src/trust_probe.rs','crates/sex-tls/src/verifier.rs','crates/sex-tls/src/probe_fixture.rs','crates/sex-trust-config/src/lib.rs'):self.assertIn(name,c['source_sha256'])
 def test_each_explicit_cpu_rtc_profile(self):
  for mode in gate.MODES:self.assertTrue(gate.command_profile(command(mode),mode)['ok'])
  self.assertFalse(gate.command_profile(command('unsupported'),'ready')['ok']);self.assertFalse(gate.command_profile(command('invalid-year'),'ready')['ok'])
 def test_replay_duplicate_profile_and_time_override_fail(self):
  for extra in[('-seed','1'),('-icount','1'),('-incoming','defer'),('-loadvm','old'),('-rtc',gate.RTC_NORMAL),('-cpu','max,+pku'),('-localtime',),('-machine','pc,rr=replay')]:self.assertFalse(gate.command_profile(command()+list(extra),'ready')['ok'])
 def test_all_metadata_modes_and_actual_close_pending_report(self):
  for mode in gate.MODES:self.assertTrue(self.inspect(metadata(mode),mode)['ok'],self.inspect(metadata(mode),mode))
  self.assertEqual(self.inspect(metadata('close',1),'close')['closed_pending'],1)
  self.assertFalse(self.inspect(metadata('close',0),'close')['ok'])
 def test_missing_or_foreign_exact_prerequisite_fail(self):
  text=metadata()
  for bad in[text.replace('holder=13 task=13','holder=13 task=99'),text.replace('owner=15 task=15 ticket=2','owner=15 task=99 ticket=2'),text.replace('task=13 slot=61','task=99 slot=61'),text.replace('service_payload=erased','service_payload=retained'),text.replace('produced=32','produced=24')]:self.assertFalse(self.inspect(bad)['ok'])
 def test_stale_seed_or_utc_sequence_and_out_of_bracket_fail(self):
  text=metadata()
  for bad in[text.replace('ticket=2','ticket=1'),text.replace('seq=2','seq=1'),text.replace('ordinal=2','ordinal=1'),text.replace('utc_snapshot='+str(SECONDS),'utc_snapshot='+str(SECONDS+1))]:self.assertFalse(self.inspect(bad)['ok'])
  self.assertFalse(gate.inspect_tls(text,'ready',command(),SECONDS+100,SECONDS+200)['ok'])
 def test_crypto_memory_record_and_output_bounds_fail(self):
  text=metadata()
  for old,new in [('task_bytes=10000','task_bytes=32769'),('private_bytes=18248','private_bytes=18247'),('read=4096','read=4097'),('write=2048','write=4096'),('tx_bytes=144','tx_bytes=0'),('tx_bytes=144','tx_bytes=4097')]:self.assertFalse(self.inspect(text.replace(old,new))['ok'])
 def test_private_values_derivatives_or_secure_claim_fail(self):
  for extra in ['[browser.tls.prepared] hash=0x123\n','[entropy.source.sample] value=123\n','[browser.tls.view] state=HelloPending authenticated=1 network=0\n','[browser.tls.view] state=HelloPending authenticated=0 network=1\n']:self.assertFalse(self.inspect(metadata()+extra)['ok'])
 def test_q_must_erase_before_clear_and_retry(self):
  text=metadata()
  for bad in[text.replace('[browser.tls.erased] bytes=18192 owned_storage=1','[browser.tls.erased] bytes=0 owned_storage=1'),text.replace('state=Cancelled','state=Unavailable'),text.replace('owned_storage=1','owned_storage=0')]:self.assertFalse(self.inspect(bad)['ok'])
 def test_unsupported_and_invalid_utc_cannot_prepare(self):
  extra=f'[browser.tls.prepared] task_bytes=10000 private_bytes=18248 read=4096 write=2048 tx_bytes=144 utc_snapshot={SECONDS} authenticated=0 network=0\n[browser.tls.view] state=HelloPending authenticated=0 network=0\n'
  for mode in('unsupported','invalid-year'):self.assertFalse(self.inspect(metadata(mode)+extra,mode)['ok'])
 def test_close_requires_erasure_and_retirement_before_ack(self):
  text=metadata('close')
  for bad in[text.replace('owned_storage=erased','owned_storage=retained'),text.replace('[browser.tls.erased]','[missing.erased]'),text.replace('before_cleanup_ack=1','before_cleanup_ack=0'),text.replace('target=15 task=15 client_leases','target=15 task=99 client_leases')]:self.assertFalse(self.inspect(bad,'close')['ok'])
 def test_exact_boot_trust_slot_identity_profile_root_and_denial_are_required(self):
  text=metadata()
  for old,new in [('slot=62','slot=61'),('consumer_task=15 profile=1','consumer_task=99 profile=1'),('profile=1 root_bytes=376 runtime_mint=0','profile=2 root_bytes=376 runtime_mint=0'),('root_bytes=376','root_bytes=375'),('[browser.trust.source.denied]','[missing.trust.denied]'),('[boot.trust.policy]','[missing.boot.policy]')]:self.assertFalse(self.inspect(text.replace(old,new))['ok'])
  boot=next(line for line in text.splitlines()if line.startswith('[boot.trust.policy]'))
  self.assertFalse(self.inspect(text+boot+'\n')['ok'])
 def test_each_l_requires_fresh_exact_held_route_policy_before_sources(self):
  text=metadata();snapshot='[trust.policy.snapshot] owner=15 task=15 seq=1 profile=1 root_bytes=376 origin=held_boot_grant\n'
  for bad in[text.replace('seq=2 profile=1','seq=1 profile=1'),text.replace('owner=15 task=15 seq=','owner=15 task=99 seq='),text.replace('origin=held_boot_grant','origin=ambient'),text.replace(snapshot,''),text+snapshot,text.replace('state=Ready(1) immutable_boot_profile=1','state=Ready(2) immutable_boot_profile=1')]:self.assertFalse(self.inspect(bad)['ok'])
  late=text.replace(snapshot,'',1).replace('[entropy.seed.released] owner=15 task=15 ticket=1',snapshot+'[entropy.seed.released] owner=15 task=15 ticket=1',1);self.assertFalse(self.inspect(late)['ok'])
 def test_controlled_fixture_selfcheck_must_be_exact_once_and_before_preparation(self):
  text=metadata();check='[browser.trust.self_check] profile=1 passed=1 fixture=public_controlled_profile authenticated_peer=0\n'
  for bad in[text.replace(check,'',1),text+check,text.replace('profile=1 passed=1','profile=2 passed=1'),text.replace('passed=1 fixture=','passed=0 fixture='),text.replace('authenticated_peer=0','authenticated_peer=1'),text.replace('fixture=public_controlled_profile','fixture=live_peer')]:self.assertFalse(self.inspect(bad)['ok'])
  late=text.replace(check,'',1).replace('[browser.tls.view] state=HelloPending',check+'[browser.tls.view] state=HelloPending',1);self.assertFalse(self.inspect(late)['ok'])
  early=text.replace(check,'',1).replace('[entropy.seed.admitted]',check+'[entropy.seed.admitted]',1);self.assertFalse(self.inspect(early)['ok'])
 def test_policy_retirement_is_exact_and_precedes_cleanup_ack(self):
  text=metadata('close');retire='[trust.policy.retired] target=15 task=15 before_cleanup_ack=1\n'
  for bad in[text.replace(retire,''),text+retire,text.replace('target=15 task=15 before_cleanup_ack=1','target=15 task=99 before_cleanup_ack=1'),text.replace('[browser.trust.closed] target=15 task=15 pending=0','[browser.trust.closed] target=15 task=15 pending=1')]:self.assertFalse(self.inspect(bad,'close')['ok'])
  self.assertFalse(self.inspect(text.replace(retire,'')+retire,'close')['ok'])
 def test_dynamic_actual_task_extent_drives_both_capacity_and_erasure(self):
  for task in(8920,12000,20000):
   text=metadata().replace('task_bytes=10000',f'task_bytes={task}').replace('private_bytes=18248',f'private_bytes={task+8248}').replace('bytes=18192 owned_storage=1',f'bytes={task+8192} owned_storage=1')
   r=self.inspect(text);self.assertTrue(r['ok'],r);self.assertEqual(r['joins'][0]['task_bytes'],task)
   self.assertEqual(r['trust_policy']['self_checks'],2)
 def test_whole_status_labels_and_padding_pixels(self):
  for state in gate.LABELS:
   t=trace(state,self.c);r=gate.verify_tls_pixels(t,pixels(t,state,self.c),state,self.c,(1280,800));self.assertTrue(r['ok'],r)
 def test_stale_hello_ink_clipping_and_unapplied_extent_fail(self):
  t=trace('HelloPending',self.c);raw=pixels(t,'HelloPending',self.c);self.assertFalse(gate.verify_tls_pixels(trace('Cancelled',self.c),raw,'Cancelled',self.c)['ok'])
  for t in[trace('HelloPending',self.c,h=100),trace('HelloPending',self.c).replace('[browser.view.applied]','[missing.applied]'),trace('HelloPending',self.c).replace('viewport=7','viewport=8')]:self.assertFalse(gate.verify_tls_pixels(t,b'','HelloPending',self.c)['ok'])

if __name__=='__main__':unittest.main()
