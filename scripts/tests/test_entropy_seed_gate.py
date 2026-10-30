"""Public checker fixtures only; never generated entropy/native evidence."""
import copy,sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import entropy_seed_gate as gate
from check_desktop_pixels import glyph
ROOT=Path(__file__).resolve().parents[2]

def command(mode='ready'):
 return ['qemu-system-x86_64','-M','q35','-cpu','max,+pku,-rdseed,-rdrand'if mode=='unsupported'else'max,+pku,+rdseed','-m','512M']
def metadata(mode='ready'):
 lines=['[boot.entropy.source] holder=13 task=13 slot=60 consumer=15 consumer_task=15 source=rdseed attempts=8 runtime_mint=0','[browser.entropy.source.denied] wrong_kind=1 absent=1 alias=1 outputs_zero=1',f'[sexnet.entropy.source] held=1 available={0 if mode=="unsupported"else 1} source=rdseed fallback=0 broker=private_seed32']
 for ticket in range(1,2 if mode=='close'else 3):
  lines.append('[browser.entropy.intent] accepted=1 explicit=1 fallback=0')
  if mode=='unsupported':lines.append('[browser.entropy.view] state=Unavailable private=1 bytes_visible=0')
  else:
   lines.append(f'[entropy.seed.admitted] owner=15 task=15 ticket={ticket} bytes=32 source=held_boot_grant')
   for count in([1,2,3,4]if ticket==1 and mode=='ready'else[1,2]if mode=='close'else[8]):lines.append(f'[entropy.source.sample] pd=13 task=13 slot=60 source=rdseed attempts=2 count={count} private=1')
   if mode=='close':
    lines.extend(['[browser.entropy.closed] target=15 task=15 pending=1 terminal=1 private_payload=erased','[entropy.seed.retired] target=15 task=15 before_cleanup_ack=1 private_payload=erased','[net.app.cleanup.ack] target=15 task=15 client_leases=0 shared_cache=retained ok=1'])
   else:lines.extend([f'[entropy.seed.progress] owner=Some(Identity {{ pd: 15, task: 15 }}) ticket={ticket} state=Some(Ready) produced=32 consumed=0 failure=0 private=1',f'[entropy.seed.released] owner=15 task=15 ticket={ticket} service_payload=erased receipt=retained','[browser.entropy.view] state=Ready private=1 bytes_visible=0'])
  if ticket==1 and mode!='close':lines.append('[browser.entropy.view] state=Cancelled private=1 bytes_visible=0')
 return '\n'.join(lines)+'\n'
def trace(state,contract,w=900,h=650):
 length=len(contract['prefix'])+50;base=length+258
 return (f'[browser.preview.layout] bytes={length} newline=1 cached_or_sample=1\n[shell.document.viewport] target=15 task=15 surface=300 sequence=7 x=20 y=100 w={w} h={h} enqueued=1\n[browser.viewport] target=15 task=15 sequence=7 w={w} h={h} origin=held_shell\n[browser.entropy.layout] base={base} bytes=64 viewport=7 state={state} pixels=unconfirmed\n[browser.view.applied] bytes={base+64} http=0 body_bytes=0 pixels=unconfirmed\n')
def pixels(text,state,contract):
 geometry=gate.seed_geometry(text,state,contract);width,height=1280,800;raw=bytearray(width*height*3);scale=geometry['scale'];color=contract['color'].to_bytes(3,'big')
 for row in geometry['rows']:
  for col,ch in enumerate(row['text']):
   for gy,bits in enumerate(glyph(contract['font'],ch)):
    for gx in range(5):
     if bits&(1<<(4-gx)):
      for dy in range(scale):
       for dx in range(scale):
        x=row['x']+(col*contract['advance']+gx)*scale+dx;y=row['y']+gy*scale+dy;at=(y*width+x)*3;raw[at:at+3]=color
 return b'P6\n1280 800\n255\n'+raw

class EntropyGateOracle(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.contract=gate.seed_contract(ROOT)
 def test_exact_supported_and_unsupported_cpu_commands(self):
  for mode in('ready','close','unsupported'):self.assertTrue(gate.command_profile(command(mode),mode)['ok'])
  self.assertFalse(gate.command_profile(command('unsupported'),'ready')['ok']);self.assertFalse(gate.command_profile(command(),'unsupported')['ok'])
 def test_seed_replay_incoming_and_instruction_count_flags_rejected(self):
  for args in[('-seed','1'),('-icount','shift=auto'),('-incoming','defer'),('-replay','record'),('-loadvm','state'),('-object','rng,id=x,seed=9'),('-machine','pc,rr=replay')]:self.assertFalse(gate.command_profile(command()+list(args),'ready')['ok'])
 def test_each_explicit_workflow_metadata_is_accepted_without_values(self):
  for mode in('ready','close','unsupported'):self.assertTrue(gate.inspect_seed(metadata(mode),mode,command(mode))['ok'])
 def test_foreign_tasks_attempt_overflow_and_secret_fields_are_rejected(self):
  text=metadata()
  for bad in[text.replace('holder=13 task=13','holder=13 task=99'),text.replace('pd=13 task=13','pd=13 task=15'),text.replace('attempts=2 count=','attempts=9 count='),text+'[entropy.source.sample] value=123 hash=0x1\n',text.replace('source=rdseed fallback=0','source=rdseed fallback=1')]:self.assertFalse(gate.inspect_seed(bad,'ready',command())['ok'])
 def test_ready_requires_produced32_positive_release_and_actual_clear(self):
  text=metadata()
  for bad in[text.replace('produced=32','produced=24'),text.replace('service_payload=erased receipt=retained','service_payload=retained receipt=retained'),text.replace('state=Cancelled private=1','state=Idle private=1'),text.replace('count=8','count=7')]:self.assertFalse(gate.inspect_seed(bad,'ready',command())['ok'])
 def test_unsupported_never_admits_samples_or_publishes_ready(self):
  text=metadata('unsupported')
  for extra in['[entropy.seed.admitted] owner=15 task=15 ticket=1 bytes=32 source=held_boot_grant\n','[entropy.source.sample] pd=13 task=13 slot=60 source=rdseed attempts=1 count=1 private=1\n','[browser.entropy.view] state=Ready private=1 bytes_visible=0\n']:self.assertFalse(gate.inspect_seed(text+extra,'unsupported',command('unsupported'))['ok'])
 def test_close_requires_pending_private_retirement_before_ack(self):
  text=metadata('close')
  for bad in[text.replace('pending=1','pending=0'),text.replace('private_payload=erased','private_payload=retained'),text.replace('target=15 task=15 before_cleanup','target=15 task=99 before_cleanup')]:self.assertFalse(gate.inspect_seed(bad,'close',command())['ok'])
 def test_real_font_source_bound_status_padding_and_erasure_pixels(self):
  for state in('Ready','Cancelled','Unavailable','Uncertain'):
   text=trace(state,self.contract);raw=pixels(text,state,self.contract);result=gate.verify_seed_pixels(text,raw,state,self.contract,(1280,800));self.assertTrue(result['ok'],result);self.assertGreater(result['checked_pixels'],0)
 def test_stale_ready_ink_or_wrong_state_cannot_pass_clear(self):
  text=trace('Ready',self.contract);raw=pixels(text,'Ready',self.contract);self.assertFalse(gate.verify_seed_pixels(trace('Cancelled',self.contract),raw,'Cancelled',self.contract)['ok'])
  self.assertFalse(gate.verify_seed_pixels(text,raw,'Cancelled',self.contract)['ok'])
 def test_geometry_and_applied_are_exact_and_clipping_never_skipped(self):
  for text in[trace('Ready',self.contract,h=100),trace('Ready',self.contract).replace('viewport=7','viewport=8'),trace('Ready',self.contract).replace('[browser.view.applied]','[missing.applied]'),trace('Ready',self.contract).replace('base=','base=1')]:
   self.assertFalse(gate.verify_seed_pixels(text,b'', 'Ready',self.contract)['ok'])

if __name__=='__main__':unittest.main()
