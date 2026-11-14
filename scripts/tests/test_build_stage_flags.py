"""Run the sealed production trace with a recording Cargo, including quoted cfgs."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class BuildStageFlags(unittest.TestCase):
    def test_trace_preserves_literal_escaped_empty_and_missing_flags(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'scripts').mkdir()
            (root / 'bin').mkdir()
            (root / 'out').mkdir()
            shutil.copy(ROOT / 'scripts/sexos_build_trace.sh', root / 'scripts/trace.sh')
            (root / 'artifact').write_bytes(b'fixture')
            cargo = root / 'bin/cargo'
            cargo.write_text('#!/usr/bin/env python3\nimport os,json\n'
                             'with open("cargo.jsonl","a") as f: '
                             'f.write(json.dumps(os.environ["RUSTFLAGS"])+"\\n")\n')
            cargo.chmod(0o755)
            flags = [
                "rustflags = '--cfg getrandom_backend=\"custom\" --cfg aes_force_soft'",
                'rustflags = "--cfg getrandom_backend=\\"custom\\""',
                'rustflags = ""',
                '',
            ]
            spec = ['target = "x86_64-sex.json"\n[allowed]\ncrates = [',
                    '"fake/Cargo.toml", "kernel"', ']']
            for i, flag in enumerate(flags):
                action = 'cargo_pkg' if i == 0 else 'cargo_manifest'
                selector = 'package = "kernel"' if i == 0 else 'manifest = "fake/Cargo.toml"'
                spec.append(f'[[stage]]\nid = "s{i}"\naction = "{action}"\n{selector}\n'
                            f'source_artifact = "artifact"\ndest_artifact = "out/s{i}"\n{flag}')
            (root / 'spec.toml').write_text('\n'.join(spec))
            env = dict(os.environ, SEXOS_TRACE_ACTIVE='1', PATH=str(root / 'bin') + os.pathsep + os.environ['PATH'])
            result = subprocess.run(['bash', 'scripts/trace.sh', 'spec.toml'], cwd=root,
                                    env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            actual = [json.loads(line) for line in (root / 'cargo.jsonl').read_text().splitlines()]
            self.assertEqual(actual, [
                '-C link-arg=-Tkernel/linker.ld --cfg getrandom_backend="custom" --cfg aes_force_soft',
                '-C relocation-model=pic -C link-arg=-pie --cfg getrandom_backend="custom"',
                '-C relocation-model=pic -C link-arg=-pie ',
                '-C relocation-model=pic -C link-arg=-pie ',
            ])
            self.assertEqual(len(list((root / 'out').iterdir())), 4)


if __name__ == '__main__':
    unittest.main()
