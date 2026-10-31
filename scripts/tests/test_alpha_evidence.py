"""Counterexamples for immutable alpha storage; these are not native evidence."""
import contextlib,hashlib,io,json,sys,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import alpha_evidence as gate
ROOT=Path(__file__).resolve().parents[2]

class AlphaEvidence(unittest.TestCase):
 def test_repeated_phase_names_preserve_distinct_prefixes_and_pixels(self):
  with tempfile.TemporaryDirectory()as name:
   out=Path(name);c=gate.Captures(out,dict(source_sha256={'real':'hash'}),(1024,768));records=[]
   for i,prefix in enumerate(['first\n','first\nsecond\n']):
    p=c.candidate('soak-current');raw=b'ppm'+bytes([i]);p.write_bytes(raw);records.append(c.commit('soak-current',p,prefix,dict(ok=True),'quil','#',9,None,raw))
   self.assertNotEqual(records[0]['screenshot'],records[1]['screenshot']);self.assertEqual((out/records[0]['screenshot']).read_bytes(),b'ppm\0');self.assertEqual(records[0]['serial_sha256'],hashlib.sha256(b'first\n').hexdigest());self.assertEqual(records[1]['serial_end'],len('first\nsecond\n'));self.assertFalse(json.loads((out/'captures.json').read_text())['complete'])
 def test_changed_candidate_and_existing_immutable_name_cannot_be_published(self):
  with tempfile.TemporaryDirectory()as name:
   out=Path(name);c=gate.Captures(out,dict(source_sha256={}),(1024,768));p=c.candidate('one');p.write_bytes(b'changed')
   with self.assertRaisesRegex(ValueError,'changed'):c.commit('one',p,'prefix',dict(ok=True),'quil','',9,None,b'original')
   p.with_name(p.name.replace('.candidate.ppm','.ppm')).write_bytes(b'prior')
   with self.assertRaisesRegex(ValueError,'reused'):c.commit('one',p,'prefix',dict(ok=True),'quil','',9,None,b'changed')
   self.assertEqual(p.with_name(p.name.replace('.candidate.ppm','.ppm')).read_bytes(),b'prior')
 def test_failed_proof_and_capture_count_bound_do_not_create_success(self):
  with tempfile.TemporaryDirectory()as name:
   c=gate.Captures(Path(name),dict(source_sha256={}),(1024,768));p=c.candidate('bad');p.write_bytes(b'raw')
   with self.assertRaises(ValueError):c.commit('bad',p,'x',dict(ok=False),'quil','',9,None,b'raw')
   self.assertEqual(c.frames,[]);c.next=gate.MAX_CAPTURES+1
   with self.assertRaisesRegex(ValueError,'bounded'):c.candidate('huge')
 def test_every_transitive_source_and_checker_must_be_in_binding(self):
  c=gate.contract(ROOT);names=c['source_sha256'];self.assertIn('scripts/alpha_session.py',names);self.assertIn('scripts/quil_third_probe.py',names);self.assertIn('scripts/workspace_refresh_probe.py',names)
  with tempfile.TemporaryDirectory()as name:
   p=Path(name)/'binding.json';p.write_text(json.dumps(dict(sources=names,runner_sha256={})));gate.require_inputs(ROOT,p,c)
   for missing in ['scripts/alpha_evidence.py','scripts/alpha_session.py','scripts/quil_third_probe.py','servers/quil/src/main.rs']:
    altered=names.copy();altered.pop(missing);p.write_text(json.dumps(dict(sources=altered)))
    with self.subTest(missing=missing),self.assertRaisesRegex(ValueError,'not sealed'):gate.require_inputs(ROOT,p,c)
 def test_check_cli_requires_no_source_disk_and_never_starts_guest(self):
  import quil_workflow_gate as runner
  with tempfile.TemporaryDirectory()as name:
   p=Path(name);iso=p/'iso';binding=p/'binding';iso.write_bytes(b'iso');binding.write_text('{}')
   with patch.object(sys,'argv',['quil_workflow_gate.py','--iso',str(iso),'--binding',str(binding),'--out',str(p),'--check']),patch.object(runner,'replay',return_value=dict(ok=True))as check,patch.object(runner.subprocess,'Popen',side_effect=AssertionError('read-only check')),contextlib.redirect_stdout(io.StringIO()):runner.main()
   self.assertEqual(check.call_count,1)

if __name__=='__main__':unittest.main()
