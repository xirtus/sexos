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
                runs='\n'.join(s.get('run','') for s in job['steps'])
                self.assertNotIn('entrypoint_build.sh',runs)
                self.assertNotIn('master_runtime_gate.sh',runs)
        self.assertEqual(set(jobs['build']['needs']),set(jobs)-{'build'})

    def test_a_folded_command_is_caught(self):
        command='scripts/ci_image_pack.sh "$RUNNER_TEMP/sexos-image/image.tar.zst"'
        self.assertEqual(self.source.count('bash '+command),1)
        broken=self.source.replace('bash '+command,'bash '+command+' (',1)
        with self.assertRaises(AssertionError):check_runs(parse_yaml(broken))


if __name__=='__main__':unittest.main()
