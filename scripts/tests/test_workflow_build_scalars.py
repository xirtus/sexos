"""Parse actual workflow scalars before asking Bash to validate their scripts."""
from pathlib import Path
import json
import subprocess
import unittest

ROOT=Path(__file__).resolve().parents[2]
JOBS=('reading','places')


def parse_yaml(source):
    # Psych ships with Ruby on the host/CI images. JSON is only the transport;
    # a real YAML parser determines folding and literal scalar boundaries.
    parsed=subprocess.run(['ruby','-rjson','-rpsych','-e',
                           'STDOUT.write(JSON.generate(Psych.safe_load(STDIN.read, aliases: true)))'],
                          input=source,text=True,capture_output=True,check=True,timeout=15)
    return json.loads(parsed.stdout)


def build_steps(workflow):
    result={}
    for name in JOBS:
        jobs=workflow['jobs']
        assert name in jobs['build']['needs'], 'required job was removed from aggregator'
        matches=[step for step in jobs[name]['steps'] if step.get('name')=='Build the image']
        assert len(matches)==1, 'missing or duplicate build step'
        run=matches[0]['run']
        assert run.endswith('\n'), 'build command must retain a literal newline'
        syntax=subprocess.run(['bash','-n'],input=run,text=True,capture_output=True,timeout=10)
        assert syntax.returncode==0, syntax.stderr
        result[name]=run
    return result


class WorkflowBuildScalars(unittest.TestCase):
    def test_actual_literal_build_commands_parse_with_newlines_and_valid_shell(self):
        workflow=parse_yaml((ROOT/'.github/workflows/ci.yml').read_text())
        runs=build_steps(workflow)
        for name,run in runs.items():
            with self.subTest(job=name):
                self.assertEqual(run.count('./scripts/entrypoint_build.sh'),1)
                self.assertIn(f'"$RUNNER_TEMP/{name}-build.log"',run)
                # Preserve the seed's real command boundary whether truncate is
                # in this literal block or the following independent app step.
                commands='\n'.join(step.get('run','') for step in workflow['jobs'][name]['steps'])
                self.assertRegex(commands,r'\)\n+truncate -s 256M')

    def test_the_exact_old_plain_multiline_form_folds_into_invalid_shell(self):
        source=(ROOT/'.github/workflows/ci.yml').read_text()
        for name in JOBS:
            with self.subTest(job=name):
                command=f'./scripts/entrypoint_build.sh > "$RUNNER_TEMP/{name}-build.log" 2>&1 || (tail -80 "$RUNNER_TEMP/{name}-build.log"; exit 1)'
                literal='      run: |\n        '+command
                self.assertEqual(source.count(literal),1)
                broken=source.replace(literal,'      run: '+command+f'\n        truncate -s 256M "$RUNNER_TEMP/blank-{name}.img"',1)
                parsed=parse_yaml(broken)
                run=next(step['run'] for step in parsed['jobs'][name]['steps'] if step.get('name')=='Build the image')
                self.assertIn(') truncate -s 256M',run)
                self.assertNotIn('\ntruncate',run)
                syntax=subprocess.run(['bash','-n'],input=run,text=True,capture_output=True,timeout=10)
                self.assertNotEqual(syntax.returncode,0)
                with self.assertRaises(AssertionError):build_steps(parsed)

    def test_folded_block_indicator_is_rejected_too(self):
        source=(ROOT/'.github/workflows/ci.yml').read_text()
        for name in JOBS:
            command=f'./scripts/entrypoint_build.sh > "$RUNNER_TEMP/{name}-build.log" 2>&1 || (tail -80 "$RUNNER_TEMP/{name}-build.log"; exit 1)'
            broken=source.replace('      run: |\n        '+command,
                                  '      run: >\n        '+command+f'\n        truncate -s 256M "$RUNNER_TEMP/blank-{name}.img"',1)
            with self.subTest(job=name),self.assertRaises(AssertionError):build_steps(parse_yaml(broken))


if __name__=='__main__':unittest.main()
