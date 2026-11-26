"""Parse actual workflow scalars before asking Bash to validate their scripts.

Once, reading/places folded their build command and the next step's `truncate`
into one invalid shell line. The image is now built once by the `image` job, so
this checks every `run:` scalar of every job with a real YAML parser and `bash -n`,
and that the jobs testing the shared image unpack it instead of rebuilding.
"""
from pathlib import Path
import json
import subprocess
import unittest

ROOT=Path(__file__).resolve().parents[2]
SHARED=('boot','web','devices','media','editor','programs','reading','places','desktop',
        'js-jit','productivity','terminal-editor','documents-pictures')


def parse_yaml(source):
    # Psych ships with Ruby on the host/CI images. JSON is only the transport;
    # a real YAML parser determines folding and literal scalar boundaries.
    parsed=subprocess.run(['ruby','-rjson','-rpsych','-e',
                           'STDOUT.write(JSON.generate(Psych.safe_load(STDIN.read, aliases: true)))'],
                          input=source,text=True,capture_output=True,check=True,timeout=15)
    return json.loads(parsed.stdout)


def check_runs(workflow):
    for name,job in workflow['jobs'].items():
        for step in job['steps']:
            run=step.get('run')
            if run is None:continue
            syntax=subprocess.run(['bash','-n'],input=run,text=True,capture_output=True,timeout=10)
            assert syntax.returncode==0,f'{name}: {step.get("name")}: {syntax.stderr}'


class WorkflowBuildScalars(unittest.TestCase):
    def setUp(self):
        self.source=(ROOT/'.github/workflows/ci.yml').read_text()
        self.workflow=parse_yaml(self.source)

    def test_every_run_scalar_is_valid_shell(self):
        check_runs(self.workflow)

    def test_shared_image_jobs_unpack_it_and_do_not_rebuild(self):
        jobs=self.workflow['jobs']
        image=[s.get('run','') for s in jobs['image']['steps']]
        self.assertTrue(any('master_runtime_gate.sh' in r for r in image))
        self.assertTrue(any('ci_image_pack.sh' in r for r in image))
        for name in SHARED:
            with self.subTest(job=name):
                job=jobs[name]
                self.assertEqual(job.get('needs'),'image')
                self.assertIn(name,jobs['build']['needs'])
                uses=[s.get('uses') for s in job['steps']]
                self.assertEqual(uses.count('./.github/actions/image'),1)
                rebuilds=[s for s in job['steps'] if 'entrypoint_build.sh' in s.get('run','')]
                if name == 'web':
                    self.assertEqual(len(rebuilds),1)
                    fixture=rebuilds[0]
                    self.assertEqual(fixture.get('name'),'Browser test image with a temporary TLS fixture root')
                    self.assertIn('program_web_tls_gate.py --make-root "$RUNNER_TEMP/browser-ca"',fixture['run'])
                    self.assertIn('SEXOS_TLS_GATE_ROOT=',fixture['run'])
                    self.assertIn('./scripts/entrypoint_build.sh',fixture['run'])
                else:
                    self.assertFalse(rebuilds)
                runs='\n'.join(s.get('run','') for s in job['steps'] if s not in rebuilds)
                self.assertNotIn('entrypoint_build.sh',runs)
                self.assertNotIn('master_runtime_gate.sh',runs)
        # Jobs master keeps informational (such as the long leak gate) may stay out.
        self.assertLessEqual(set(SHARED)|{'image','web','boot'},set(jobs['build']['needs']))
        self.assertLessEqual(set(jobs['build']['needs']),set(jobs))

    def test_crash_probe_image_is_built_once_for_the_crash_and_leak_gates(self):
        jobs=self.workflow['jobs']
        crash='\n'.join(s.get('run','') for s in jobs['crash']['steps'])
        self.assertEqual(crash.count('SEXOS_CRASH_PROBE=1'),1)
        self.assertIn('ci_image_pack.sh',crash)
        self.assertEqual(jobs['leaks']['needs'],'crash')
        leaks='\n'.join(s.get('run','') for s in jobs['leaks']['steps'])
        self.assertNotIn('entrypoint_build.sh',leaks)
        image=[s for s in jobs['leaks']['steps'] if s.get('uses')=='./.github/actions/image']
        self.assertEqual(image[0]['with']['artifact'],'sexos-crash-image')
        self.assertNotIn('crash',jobs['profiles']['strategy']['matrix']['gate'])

    def test_every_gate_job_follows_the_scope(self):
        jobs=self.workflow['jobs']
        self.assertEqual(set(jobs['build']['needs']),set(jobs)-{'build'})
        for name,job in jobs.items():
            if name in ('changes','build'):continue
            with self.subTest(job=name):
                condition=job.get('if','')
                self.assertTrue(any(k in condition for k in ('needs.changes.outputs.scope','needs.image.outputs.full','needs.crash.outputs.image')),condition)

    def test_a_folded_command_is_caught(self):
        command='scripts/ci_image_pack.sh "$RUNNER_TEMP/sexos-image/image.tar.zst"'
        self.assertEqual(self.source.count('bash '+command),1)
        broken=self.source.replace('bash '+command,'bash '+command+' (',1)
        with self.assertRaises(AssertionError):check_runs(parse_yaml(broken))


if __name__=='__main__':unittest.main()
