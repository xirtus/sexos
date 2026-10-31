"""Counterexamples for immutable alpha storage; these are not native evidence."""
import contextlib,gzip,hashlib,io,json,sys,tempfile,unittest
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
   for missing in ['scripts/alpha_evidence.py','scripts/alpha_session.py','scripts/quil_third_probe.py','servers/quil/src/main.rs','scripts/check_launcher_pixels.py','scripts/launcher_workflow_gate.py','servers/silk-shell/src/launcher.rs']:
    altered=names.copy();altered.pop(missing);p.write_text(json.dumps(dict(sources=altered)))
    with self.subTest(missing=missing),self.assertRaisesRegex(ValueError,'not sealed'):gate.require_inputs(ROOT,p,c)
 def test_launcher_uses_existing_full_pixel_and_applied_checker(self):
  c=dict(launcher={'held':'contract'},bar={'clock':'contract'});detail=dict(title='Open Quil',status='CLOSED / RESERVE EXHAUSTED');bar=dict(ok=True,errors=[])
  with patch.object(gate,'launcher_frame',return_value=detail)as launcher,patch.object(gate,'bar_pixels',return_value=bar):
   result=gate.verify_frame('prefix',b'ppm',c,(1024,768),'launcher','Open Quil',9)
   self.assertTrue(result['ok']);self.assertIs(result['view'],detail);launcher.assert_called_once_with('prefix',b'ppm',dict(kind='launcher',target=9,name='launcher-exhausted',title='Open Quil'),c['launcher'],(1024,768))
   with self.assertRaises(ValueError):gate.verify_frame('prefix',b'ppm',c,(1024,768),'launcher','Open Quil',18)
  with patch.object(gate,'launcher_frame',side_effect=ValueError('missing current Applied')):
   with self.assertRaisesRegex(ValueError,'Applied'):gate.verify_frame('prefix',b'ppm',c,(1024,768),'launcher','Open Quil',9)
 def test_third_launcher_requires_exact_registry_and_current_cleanup(self):
  text='[task.finish] pd=18 task=18 reason=0 terminal=1\n[sext.app.pool.ready] target=18 task=18 bytes=1 zeroed=1 ok=1\n[shell.palette.select] old=0 new=1\n[launcher.text.view] selected=1\n[launcher.text.row] index=1 row=1 name=Open Quil status=CLOSED / RESERVE EXHAUSTED\n'
  edit=dict(name='third-quil-edit',kind='quil',target=18,expected='t#',serial_end=1);exhausted=dict(name='third-quil-exhausted-launcher',kind='launcher',target=9,expected='Open Quil',serial_end=len(text));third=dict(frame=edit,exhaustion_frame=exhausted,exhaustion_selection_offset=text.index('[shell.palette.select]'),exhaustion_row_offset=text.index('[launcher.text.row]'));frames=[edit,exhausted]
  gate.verify_third_captures(third,frames,text)
  for changed,records,raw in [(dict(third,exhaustion_frame=dict(exhausted,serial_end=1)),frames,text),(third,[edit],text),(dict(third,exhaustion_row_offset=0),frames,text),(third,frames,text.replace('ok=1','ok=0')),(third,frames,text.replace('[task.finish]','[missing.one]'))]:
   with self.assertRaises(ValueError):gate.verify_third_captures(changed,records,raw)
 def test_check_cli_requires_no_source_disk_and_never_starts_guest(self):
  import quil_workflow_gate as runner
  with tempfile.TemporaryDirectory()as name:
   p=Path(name);iso=p/'iso';binding=p/'binding';iso.write_bytes(b'iso');binding.write_text('{}')
   with patch.object(sys,'argv',['quil_workflow_gate.py','--iso',str(iso),'--binding',str(binding),'--out',str(p),'--check']),patch.object(runner,'replay',return_value=dict(ok=True))as check,patch.object(runner.subprocess,'Popen',side_effect=AssertionError('read-only check')),contextlib.redirect_stdout(io.StringIO()):runner.main()
   self.assertEqual(check.call_count,1)

 def test_spindle_frames_use_real_grid_oracle_with_exact_root_and_sparse_pane_policy(self):
  import check_spindle_viewport_pixels as pixels
  from test_spindle_viewport_pixels import fixture,FONT
  c=dict(root=str(ROOT),font=FONT,bar={'clock':'contract'})
  for expected,caption in [('> echo spindle17',None),('NET / verified','NET / verified')]:
   text,ppm=fixture(640,134,24,240,caption=caption)
   with patch.object(pixels,'verify',wraps=pixels.verify)as oracle,patch.object(gate,'bar_pixels',return_value=dict(ok=True,errors=[]))as bar:
    result=gate.verify_frame(text,ppm,c,(1280,800),'spindle',expected,24)
    self.assertTrue(result['ok'],result)
    oracle.assert_called_once_with(text,ppm,FONT,target=24,require_prompt=expected if expected.startswith('> ')else None,
      require_echo=False,require_all_ink=False,resolution=(1280,800),root=str(ROOT),require_text=None if expected.startswith('> ')else expected)
    bar.assert_called_once_with(text,ppm,c['bar'],(1280,800),'Snapshot')
  text,ppm=fixture(640,134,24,240,caption='NET / verified')
  with patch.object(gate,'bar_pixels',return_value=dict(ok=False,errors=['actual civil clock failed']))as bar:
   self.assertFalse(gate.verify_frame(text,ppm,c,(1280,800),'spindle','NET / verified',24)['ok']);bar.assert_called_once()

 def test_spindle_exhaustion_forwards_logical_target_and_terminal_launcher_name(self):
  c=dict(launcher={'held':'contract'},bar={'clock':'contract'});detail=dict(title='Open Spindle',status='CLOSED / RESERVE EXHAUSTED')
  with patch.object(gate,'launcher_frame',return_value=detail)as launcher,patch.object(gate,'bar_pixels',return_value=dict(ok=True,errors=[]))as bar:
   result=gate.verify_frame('prefix',b'ppm',c,(1024,768),'launcher','Open Spindle',12)
   self.assertTrue(result['ok']);self.assertIs(result['view'],detail)
   launcher.assert_called_once_with('prefix',b'ppm',dict(kind='launcher',target=12,name='launcher-exhausted',title='Open Spindle'),c['launcher'],(1024,768))
   bar.assert_called_once_with('prefix',b'ppm',c['bar'],(1024,768),'Snapshot')
   with self.assertRaises(ValueError):gate.verify_frame('prefix',b'ppm',c,(1024,768),'launcher','Open Spindle',24)

 def test_requested_third_spindle_cannot_pass_canonical_replay_without_soak(self):
  # Transport/pixels are separately tested above; this fixture exercises the
  # actual immutable closure and canonical option/result join without a guest.
  with tempfile.TemporaryDirectory()as name:
   root=Path(name).resolve();out=root/'evidence';out.mkdir();iso=root/'held.iso';iso.write_bytes(b'held image');binding=root/'binding.json';binding.write_text('{}')
   source={'held':'source'};c=dict(source_sha256=source);held={'sealed':'binding'};text='complete immutable prefix\n';proof=dict(ok=True)
   (out/'serial.log').write_text(text);(out/'frame.ppm').write_bytes(b'pixel fixture')
   disk=b'unchanged terminal disk';disk_hash=hashlib.sha256(disk).hexdigest()
   for file in ['seed.img.gz','nvme.img.gz']:(out/file).write_bytes(gzip.compress(disk))
   frame=dict(sequence=1,name='baseline',kind='quil',target=9,expected='',region=None,screenshot='frame.ppm',screenshot_sha256=gate.sha(out/'frame.ppm'),serial_end=len(text),serial_sha256=hashlib.sha256(text.encode()).hexdigest(),source_sha256=source,proof=proof)
   (out/'captures.json').write_text(json.dumps(dict(version=gate.VERSION,complete=True,serial_offset_units=gate.SERIAL_UNITS,frames=[frame])))
   result=dict(ok=True,acceptance=False,stages=[frame],documents=[])
   manifest=dict(evidence_version=gate.VERSION,error=None,binding=held,pixel_source_sha256=source,model='e1000e',resolution='1024x768',
    command=['qemu-system-x86_64','-M','q35','-rtc',gate.RTC,'-cdrom',str(iso),'-device','VGA,xres=1024,yres=768','-device','e1000e,netdev=peer,mac=52:54:00:12:34:56'],
    input_disk_sha256=disk_hash,final_disk_sha256=disk_hash,host_utc_start=1,host_utc_end=2,options=dict(third_spindle=False,pending_io_close=False))
   def seal():
    (out/'result.json').write_text(json.dumps(result));manifest['raw_sha256']={str(p.relative_to(out)):gate.sha(p)for p in out.rglob('*')if p.is_file()and p.name!='manifest.json'}
    (out/'manifest.json').write_text(json.dumps(manifest))
   with patch.object(gate,'contract',return_value=c),patch.object(gate,'require_inputs'),patch.object(gate,'frozen_binding',return_value=held),patch.object(gate,'verify_frame',return_value=proof):
    seal();self.assertTrue(gate.replay(root,iso,binding,out)['ok'])
    manifest['options'].update(third_spindle=True,soak_seconds=1)
    for missing in ['absent',None,{}]:
     result.pop('soak',None)
     if missing!='absent':result['soak']=missing
     seal();before={str(p.relative_to(out)):p.read_bytes()for p in out.rglob('*')if p.is_file()}
     with self.subTest(soak=missing),self.assertRaisesRegex(ValueError,'third Spindle option/result differs'):
      gate.replay(root,iso,binding,out)
     self.assertEqual(before,{str(p.relative_to(out)):p.read_bytes()for p in out.rglob('*')if p.is_file()})

if __name__=='__main__':unittest.main()
