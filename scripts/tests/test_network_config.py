"""Synthetic adverse metadata receipts; actual authority joins run natively."""
import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from check_nic_command import config_receipts
BOOT='[boot.network.config] owner=13 version=1 profile=1 origin=boot_descriptor immutable=1 runtime_mint=0\n'
SEAL='[boot.memory.authority.sealed]\n'
QUERY='[pci.network.config] pd=13 slot=19 origin=boot_descriptor runtime_mint=0\n'
WORDS=(0x0a00020f|(24<<32)|(1<<40)|(1<<56),0x0a000202|(0x0a000202<<32),46079|(46078<<16)|(46076<<32)|(48000<<48),0x0a000202)
COPY='[sexnet.network.config] words='+','.join(f'{n:x}'for n in WORDS)+' origin=held_boot_nic immutable=1 route=not_installed\n'
DENIED='[browser.network.config.denied] slot=18 kind=route metadata=0\n[sexnet.network.config.denied] requests=7 output_zero=1 ok=1\n'
def inspect(text,n=1):return config_receipts(text,n,text.find(SEAL))
class ConfigEvidence(unittest.TestCase):
 def test_exact_boot_query_and_checked_copy(self):
  self.assertTrue(inspect(BOOT+SEAL+QUERY+COPY+DENIED)['ok'])
 def test_missing_duplicate_foreign_and_reordered_receipts_fail(self):
  for text in [SEAL+QUERY+COPY+DENIED,BOOT+SEAL+COPY+DENIED,BOOT+SEAL+QUERY+DENIED,BOOT+SEAL+QUERY+COPY,BOOT+SEAL+QUERY*2+COPY+DENIED,SEAL+BOOT+QUERY+COPY+DENIED,BOOT+QUERY+SEAL+COPY+DENIED,(BOOT+SEAL+QUERY+COPY+DENIED).replace('slot=19','slot=18')]:
   self.assertFalse(inspect(text)['ok'])
 def test_every_word_mutation_and_late_preparation_fail(self):
  for word in WORDS:
   changed=COPY.replace(f'{word:x}',f'{word^1:x}',1)
   self.assertFalse(inspect(BOOT+SEAL+QUERY+changed+DENIED)['ok'])
  self.assertFalse(inspect(BOOT+SEAL+QUERY+'[sexnet.nic.prepare]\n'+COPY+DENIED)['ok'])
 def test_absent_grant_requires_zero_metadata_and_no_fallback(self):
  trace=BOOT+SEAL+'[sexnet.network.config.unavailable] network=0 fallback=0\n'+DENIED
  self.assertTrue(inspect(trace,0)['ok'])
  for text in [trace.replace('fallback=0','fallback=1'),trace+QUERY,trace+COPY]:self.assertFalse(inspect(text,0)['ok'])
if __name__=='__main__':unittest.main()
