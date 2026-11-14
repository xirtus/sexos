"""Normal diagnostics, declared proof mode and protected-interval negative tests."""
import pathlib,re,sys,unittest,tomllib,tempfile,shutil,subprocess,os
ROOT=pathlib.Path(__file__).resolve().parents[2];sys.path.insert(0,str(ROOT/'scripts'))
from input_payload_profile import diagnostic_spec,assert_payload_proof,PROOF_RECEIPTS,wait_releases,STAGES,FLAG
from input_payload_audit import audit_interval,require_normal_image

class InputPrivacy(unittest.TestCase):
 def test_profile_changes_only_declared_userspace_flags_and_preserves_crypto_flags(self):
  source=(ROOT/'sexos_build_spec.toml').read_text();before=tomllib.loads(source);after=tomllib.loads(diagnostic_spec(source))
  self.assertEqual({k:v for k,v in before.items() if k!='stage'},{k:v for k,v in after.items() if k!='stage'})
  self.assertEqual(len(before['stage']),len(after['stage']))
  for a,b in zip(before['stage'],after['stage']):
   expected=dict(a)
   if a['id'] in STAGES:expected['rustflags']=' '.join(filter(None,[a.get('rustflags',''),FLAG]))
   self.assertEqual(expected,b)
  with self.assertRaises(ValueError):diagnostic_spec(diagnostic_spec(source))
 def test_normal_missing_duplicate_and_conflicting_images_cannot_claim_payload_proof(self):
  assert_payload_proof(PROOF_RECEIPTS)
  for trace in ['',PROOF_RECEIPTS.replace('proof','redacted'),PROOF_RECEIPTS*2,PROOF_RECEIPTS+'[input.payload.mode] component=shell mode=redacted\n']:
   with self.assertRaises(ValueError):assert_payload_proof(trace)
 def test_private_input_refuses_diagnostic_missing_and_ambiguous_mode(self):
  normal=''.join('[input.payload.mode] component='+c+' mode=redacted\n' for c in ('sexinput','shell','sexusb'))
  def check(trace):
   def wait(pattern,start,timeout):
    match=re.search(pattern,trace[start:])
    if match is None:raise TimeoutError('no receipt')
    return match
   require_normal_image(dict(wait=wait,text=lambda:trace))
  check(normal)
  for trace in (normal.replace('redacted','proof'),normal*2,normal+'[keyboard_cursor.gate] enabled=1',normal.replace('component=sexusb','component=missing')):
   with self.assertRaises((ValueError,TimeoutError)):check(trace)
 def test_ctrl_fence_uses_both_admitted_releases_without_key_contents(self):
  trace='[app.actual.ack]\n[input.keyboard.keyup.ok] source=ps2 payload=redacted\n[other]\n[input.keyboard.keyup.ok] source=ps2 payload=redacted\n'
  seen=[]
  def wait(pattern,start):
   seen.append(start);found=re.search(pattern,trace[start:]);self.assertIsNotNone(found);return found
  end=wait_releases(wait,0)
  self.assertEqual(len(seen),2);self.assertGreater(end,seen[1])
 def test_every_known_key_encoding_is_rejected_in_a_protected_interval(self):
  forbidden=['[pdx.listen.raw.wrapper] type=0x202 caller=3 a0=0x1e a1=1',
   '[sexinput.ps2.scancode] raw=0x1e','[sexinput.key.send] code=30 down=1 mod=0 source=1',
   '[input.keyboard.keydown.ok] code=30 source=usb','[sexinput.kbd.recv] key=0x4 mod=0x2',
   '[sexinput.key.rejected] source=ps2 raw=30','[silk-shell.hid.raw] class=1 a0=0x1e',
   '[silk-shell.hid.recv] class=1 code=30 value=1','[silk-shell.key.recv] code=30',
   '[silk.key.route] surface=359 code=30 printable=1','[shell.browser.input] code=30',
   '[silk.text.append] surface=100 len=1 ch=x','[usb.keyboard.shell.recv] key=30','[shell.media.key] code=30','[spindle.input.recv] key=printable ch=x len=1','[sexusb.input.rejected] op=609 arg0=30','[spindle.line.enter] text="private"']
  for line in forbidden:self.assertFalse(audit_interval(line)['userspace_safe'],line)
  self.assertTrue(audit_interval('[sexinput.key.send] source=1 payload=redacted\n[input.keyboard.keyup.ok] source=ps2 payload=redacted')['userspace_safe'])
 def test_kernel_residual_is_separate_and_never_becomes_a_complete_privacy_claim(self):
  for trace in ['', '[ps2.port60.read] n=1 sc=0x1e','[ps2.input_ring.enqueue] n=1 type=0x201 sc=0x1e','[ps2.input_ring.drop] reason=ring_full sc=0x1e']:
   result=audit_interval(trace);self.assertTrue(result['userspace_safe']);self.assertFalse(result['complete_keyboard_privacy_proven']);self.assertEqual(bool(result['kernel_payload_lines']),bool(trace))
 def test_profile_builder_restores_originals_on_failure_and_normal_after_success(self):
  for fail in (False,True):
   with tempfile.TemporaryDirectory() as temporary:
    root=pathlib.Path(temporary);(root/'scripts').mkdir();(root/'iso_root/servers').mkdir(parents=True)
    for name in ('build_input_payload_proof.sh','input_payload_profile.py'):shutil.copyfile(ROOT/'scripts'/name,root/'scripts'/name)
    original=(ROOT/'sexos_build_spec.toml').read_bytes();(root/'sexos_build_spec.toml').write_bytes(original)
    for name in ('sexos-v1.0.0.iso','iso_root/sexos-kernel','iso_root/servers/sexinput','iso_root/servers/silk-shell'):(root/name).write_bytes(b'original')
    (root/'scripts/master_runtime_gate.sh').write_text('#!/usr/bin/env bash\nset -eu\npython3 -c \'import tomllib; s=tomllib.load(open("sexos_build_spec.toml","rb")); assert all("sexos_input_payload_proof" in x["rustflags"] for x in s["stage"] if x["id"] in ("build_silk_shell","build_sexinput"))\'\nprintf proof > sexos-v1.0.0.iso\nif [[ "${MOCK_INPUT_BUILD_FAIL:-0}" = 1 ]]; then exit 43; fi\n')
    (root/'scripts/freeze_native_evidence.py').write_text("from pathlib import Path\nimport sys\nassert Path('sexos-v1.0.0.iso').read_bytes()==b'proof'\nout=Path(sys.argv[sys.argv.index('--out')+1]);out.mkdir();(out/'frozen').write_text('proof')\n")
    (root/'scripts/entrypoint_build.sh').write_text("#!/usr/bin/env bash\nset -eu\nif grep -q 'sexos_input_payload_proof' sexos_build_spec.toml; then exit 44; fi\nprintf normal > sexos-v1.0.0.iso\n")
    result=subprocess.run(['bash','scripts/build_input_payload_proof.sh',str(root/'evidence')],cwd=root,env=dict(os.environ,MOCK_INPUT_BUILD_FAIL='1' if fail else '0'),capture_output=True,text=True)
    self.assertEqual(result.returncode,43 if fail else 0,result.stderr)
    self.assertEqual((root/'sexos_build_spec.toml').read_bytes(),original)
    self.assertEqual((root/'sexos-v1.0.0.iso').read_bytes(),b'original' if fail else b'normal')
    if not fail:
     self.assertEqual((root/'evidence/UNSAFE-input-payload-proof.iso').read_bytes(),b'proof')
     self.assertEqual((root/'evidence/normal.iso').read_bytes(),b'normal')
 def test_diagnostic_ci_keeps_frozen_proof_separate_and_restores_normal(self):
  builder=(ROOT/'scripts/build_input_payload_proof.sh').read_text()
  self.assertLess(builder.index('master_runtime_gate.sh --probe'),builder.index('freeze_native_evidence.py'))
  self.assertLess(builder.index('freeze_native_evidence.py'),builder.index('bash scripts/entrypoint_build.sh > "$EVIDENCE_DIR/normal-build.log"'))
  workflow=(ROOT/'.github/workflows/ci.yml').read_text()
  self.assertIn('profiles, input-payload-proof, js-jit',workflow)
  for gate in ('https_asset_gate.py','connected_desktop_gate.py','browser_reserve_gate.py'):
   self.assertIn('"$runner/'+gate+'" --iso "$iso" --binding "$binding"',workflow)
  self.assertIn('runner="$evidence/frozen/source/scripts"',workflow)
  self.assertIn('UNSAFE-keyboard-payload-proof-never-private-input',workflow)
  commands=[line for line in workflow.splitlines() if 'program_chastity_gate.py' in line]
  self.assertEqual(len(commands),1);self.assertIn('--input-privacy',commands[0]);self.assertIn('--iso sexos-v1.0.0.iso',commands[0])
 def test_generic_wrapper_and_usb_errors_never_format_arguments(self):
  wrapper=(ROOT/'crates/sex-pdx/src/lib.rs').read_text();start=wrapper.index('"[pdx.listen.raw.wrapper]');end=wrapper.index(');',start);self.assertNotRegex(wrapper[start:end],r'\ba[012]=|arg[012]')
  usb=(ROOT/'servers/sexusb/src/main.rs').read_text()
  for line in usb.splitlines():
   if '[sexusb.input.rejected]' in line:self.assertNotIn('arg0=',line);self.assertNotIn('arg1=',line);self.assertNotIn('arg2=',line)

if __name__=='__main__':unittest.main()
