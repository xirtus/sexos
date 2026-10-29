import hashlib
import json
import pathlib
import sys
import tempfile
import unittest
sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]))
from freeze_native_evidence import freeze

class FreezeTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=pathlib.Path(self.temp.name);self.build=self.root/'build';self.build.mkdir();self.out=self.root/'out'
        (self.root/'kernel/src').mkdir(parents=True);(self.root/'scripts').mkdir()
        (self.root/'kernel/src/main.rs').write_text('untracked new module fixture')
        (self.root/'scripts/check.py').write_text('checker fixture')
        (self.root/'tests').mkdir();(self.root/'tests/actual.rs').write_text('host production extraction fixture')
        self.iso=self.build/'image.iso';self.iso.write_bytes(b'image')
        self.kernel=self.build/'kernel';self.kernel.write_bytes(b'kernel')
        sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
        self.provenance=dict(build_gate='PASS',source_image_binding='sealed_build_this_run',sources_unchanged_during_build=True,
            production_source_sha256={'kernel/src/main.rs':sha(self.root/'kernel/src/main.rs')},iso_sha256=sha(self.iso),kernel_sha256=sha(self.kernel))
        self.save();(self.build/'report.md').write_text('| **FINAL_SCORE** | **GREEN_MASTER** |')
    def save(self):(self.build/'provenance.json').write_text(json.dumps(self.provenance))
    def run_freeze(self):return freeze(self.root,self.build,self.out,self.iso,self.kernel)
    def test_copies_bound_sources_artifacts_and_separate_checker_snapshot(self):
        r=self.run_freeze();self.assertEqual((self.out/'source/kernel/src/main.rs').read_text(),'untracked new module fixture')
        self.assertEqual((self.out/'tested.iso').read_bytes(),b'image');self.assertIn('scripts/check.py',r['runner_sha256'])
        self.assertIn('tests/actual.rs',r['runner_sha256']);self.assertEqual((self.out/'source/tests/actual.rs').read_text(),'host production extraction fixture')
        with self.assertRaises(FileExistsError):self.run_freeze()
        self.assertEqual(json.loads((self.out/'source-binding.json').read_text()),r)
    def test_changed_source_or_wrong_artifact_cannot_publish_binding(self):
        for which in ['source','image','kernel']:
            with self.subTest(which=which):
                path={'source':self.root/'kernel/src/main.rs','image':self.iso,'kernel':self.kernel}[which]
                before=path.read_bytes();path.write_bytes(b'changed')
                with self.assertRaises(ValueError):self.run_freeze()
                self.assertFalse((self.out/'source-binding.json').exists());path.write_bytes(before)
    def test_skip_build_or_failed_probe_is_not_a_source_image_binding(self):
        self.provenance['build_gate']='SKIP';self.save()
        with self.assertRaises(ValueError):self.run_freeze()
        self.provenance['build_gate']='PASS';self.save();(self.build/'report.md').write_text('RED_MASTER')
        with self.assertRaises(ValueError):self.run_freeze()
    def test_manifest_cannot_read_outside_repository(self):
        self.provenance['production_source_sha256']={'../outside':hashlib.sha256(b'x').hexdigest()};self.save()
        # Resolve fails or rejects before an output binding can be published.
        with self.assertRaises((ValueError,FileNotFoundError)):self.run_freeze()
        self.assertFalse((self.out/'source-binding.json').exists())
    def test_checker_symlink_cannot_copy_outside_repository(self):
        with tempfile.TemporaryDirectory()as outside:
            target=pathlib.Path(outside)/'external.py';target.write_text('must not copy')
            (self.root/'scripts/outside.py').symlink_to(target)
            with self.assertRaisesRegex(ValueError,'outside repository'):self.run_freeze()
            self.assertFalse((self.out/'source-binding.json').exists())
if __name__=='__main__':unittest.main()
