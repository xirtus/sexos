"""Generated helper substitution cannot authorize a sealed consuming image."""
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import build_input_receipt as receipts
from freeze_native_evidence import freeze


class ConsumedInputs(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.helper = self.root / 'iso_root/programs/moanplug'
        self.helper.parent.mkdir(parents=True)
        self.helper.write_bytes(b'host fixture helper, not a guest ELF')
        self.output = self.root / 'iso_root/servers/sexfiles'
        self.output.parent.mkdir()
        self.output.write_bytes(b'host fixture Files image')
        self.stage = dict(id='build_sexfiles', action='cargo_manifest', manifest='servers/sexfiles/Cargo.toml',
                          dest_artifact='iso_root/servers/sexfiles', consumed_inputs=['iso_root/programs/moanplug'])
        self.spec = dict(stage=[self.stage])

    def seal(self):
        receipts.begin(self.root, self.stage)
        receipts.finish(self.root, self.stage)
        return receipts.collect(self.root, self.spec)

    def test_missing_changed_and_unfinished_input_refuse_seal(self):
        receipts.begin(self.root, self.stage)
        with self.assertRaises(ValueError): receipts.collect(self.root, self.spec)
        self.helper.write_bytes(b'replaced helper during Files build')
        with self.assertRaises(ValueError): receipts.finish(self.root, self.stage)
        self.helper.unlink()
        with self.assertRaises(FileNotFoundError): receipts.finish(self.root, self.stage)

    def test_changed_output_input_and_injected_receipt_refuse_freeze(self):
        records = self.seal()
        for path in (self.helper, self.output):
            before = path.read_bytes()
            path.write_bytes(b'replaced after compile')
            with self.assertRaises(ValueError): receipts.validate(self.root, self.spec, records)
            path.write_bytes(before)
        for mutation in ('missing', 'extra', 'output', 'incomplete'):
            bad = copy.deepcopy(records)
            if mutation == 'missing': bad.clear()
            elif mutation == 'extra': bad['unrelated'] = bad['build_sexfiles']
            elif mutation == 'output': bad['build_sexfiles']['output'] = 'iso_root/servers/other'
            else: bad['build_sexfiles']['complete'] = False
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                receipts.validate(self.root, self.spec, bad)

    def test_external_symlink_and_parent_path_are_refused(self):
        with tempfile.TemporaryDirectory() as outside:
            target = Path(outside) / 'helper'
            target.write_bytes(b'outside')
            self.helper.unlink(); self.helper.symlink_to(target)
            with self.assertRaises(ValueError): receipts.begin(self.root, self.stage)
        self.stage['consumed_inputs'] = ['../outside']
        with self.assertRaises((ValueError, FileNotFoundError)): receipts.begin(self.root, self.stage)

    def test_freezer_archives_declared_helper_and_consuming_output(self):
        records = self.seal()
        spec = self.root / 'sexos_build_spec.toml'
        spec.write_text('[[stage]]\nid="build_sexfiles"\naction="cargo_manifest"\n'
                        'manifest="servers/sexfiles/Cargo.toml"\ndest_artifact="iso_root/servers/sexfiles"\n'
                        'consumed_inputs=["iso_root/programs/moanplug"]\n')
        (self.root / 'scripts').mkdir(); (self.root / 'tests').mkdir()
        build = self.root / 'build'; build.mkdir()
        iso = build / 'image'; iso.write_bytes(b'host fixture ISO')
        kernel = build / 'kernel'; kernel.write_bytes(b'host fixture kernel')
        sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
        provenance = dict(build_gate='PASS', source_image_binding='sealed_build_this_run',
                          sources_unchanged_during_build=True, production_source_sha256={'sexos_build_spec.toml': sha(spec)},
                          iso_sha256=sha(iso), kernel_sha256=sha(kernel), consumed_build_inputs=records)
        (build / 'provenance.json').write_text(json.dumps(provenance))
        (build / 'report.md').write_text('| **FINAL_SCORE** | **GREEN_MASTER** |')
        out = self.root / 'frozen'
        result = freeze(self.root, build, out, iso, kernel)
        self.assertEqual(result['consumed_build_inputs'], records)
        for original in (self.helper, self.output):
            self.assertEqual((out / 'generated-inputs' / original.relative_to(self.root)).read_bytes(), original.read_bytes())
        # A record-less older build must not gain a newly declared trust input.
        del provenance['consumed_build_inputs']
        (build / 'provenance.json').write_text(json.dumps(provenance))
        with self.assertRaises(ValueError): freeze(self.root, build, self.root / 'invalid', iso, kernel)

    def test_actual_trace_rejects_helper_change_during_cargo(self):
        scripts = self.root / 'scripts'; scripts.mkdir()
        for name in ('sexos_build_trace.sh', 'build_input_receipt.py'):
            shutil.copyfile(ROOT / 'scripts' / name, scripts / name)
        binaries = self.root / 'bin'; binaries.mkdir()
        cargo = binaries / 'cargo'
        cargo.write_text('#!/usr/bin/env python3\nfrom pathlib import Path\n'
                         'Path("iso_root/programs/moanplug").write_bytes(b"substituted during cargo")\n')
        cargo.chmod(0o755)
        spec = self.root / 'spec.toml'
        spec.write_text('target = "x86_64-sex.json"\n[allowed]\ncrates = [\n"servers/sexfiles/Cargo.toml"\n]\n'
                        '[[stage]]\nid="build_sexfiles"\naction="cargo_manifest"\n'
                        'manifest="servers/sexfiles/Cargo.toml"\nsource_artifact="iso_root/servers/sexfiles"\n'
                        'dest_artifact="staged-files"\nconsumed_inputs=["iso_root/programs/moanplug"]\n')
        result = subprocess.run(['bash', str(scripts / 'sexos_build_trace.sh'), str(spec)], cwd=self.root,
                                env=dict(os.environ, SEXOS_TRACE_ACTIVE='1', PATH=str(binaries)+os.pathsep+os.environ['PATH']),
                                capture_output=True, text=True, timeout=15)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('consumed input changed during build stage', result.stderr, result.stdout + result.stderr)
        record = json.loads((self.root / receipts.RECEIPTS / 'build_sexfiles.json').read_text())
        self.assertFalse(record['complete'])


if __name__ == '__main__': unittest.main()
