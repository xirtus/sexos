"""Canonical stage/launch identity; synthetic fixture bytes are not runtime proof."""
import ast
import io
from pathlib import Path
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]


def fixture(script, root):
    path = ROOT / 'scripts' / script
    node = next(n for n in ast.parse(path.read_text()).body
                if isinstance(n, ast.Assign) and any(isinstance(t, ast.Name) and t.id == 'FILES' for t in n.targets))
    return eval(compile(ast.Expression(node.value), str(path), 'eval'),
                {'ROOT': root, 'b': io.BytesIO(b'png-fixture'), 'NOTE': b'note-fixture'})


class NativeGateStageIdentity(unittest.TestCase):
    def test_fixtures_require_canonical_staged_artifacts(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            stage = root / 'iso_root/programs'
            stage.mkdir(parents=True)
            for name in ('loom', 'gaze', 'jot'):
                (stage / name).write_bytes(name.encode())
            self.assertEqual(fixture('files_open_gate.py', root), {'/loom': b'loom', '/jot': b'jot'})
            self.assertEqual(fixture('drag_drop_gate.py', root),
                             {'/loom': b'loom', '/jot': b'jot', '/note.txt': b'note-fixture'})
            self.assertEqual(fixture('session_restore_gate.py', root), {'/gaze': b'gaze', '/pic.png': b'png-fixture'})
            for canonical, old, scripts in [('loom', 'files', ('files_open_gate.py', 'drag_drop_gate.py')),
                                            ('gaze', 'view', ('session_restore_gate.py',))]:
                (stage / canonical).unlink()
                (stage / old).write_bytes(b'obsolete-name')
                for script in scripts:
                    with self.assertRaises(FileNotFoundError, msg=script):
                        fixture(script, root)

    def test_loom_stage_matches_actual_launcher_alias_and_installed_path(self):
        stages = tomllib.loads((ROOT / 'sexos_build_spec.toml').read_text())['stage']
        stage = next(s for s in stages if s['id'] == 'build_files')
        self.assertEqual(stage['manifest'], 'apps/files/Cargo.toml')
        self.assertEqual(stage['dest_artifact'], 'iso_root/programs/loom')
        source = (ROOT / 'apps/spindle/src/main.rs').read_text()
        alias = source[source.index('b"loom" | b"files" => {'):]
        alias = alias[:alias.index('\n        }')]
        self.assertIn('submit_program(b"loom",', alias)
        for script in ('files_open_gate.py', 'drag_drop_gate.py'):
            gate = (ROOT / 'scripts' / script).read_text()
            self.assertNotIn('/bin/files', gate)
            self.assertIn('[files.app.', gate)
        self.assertIn("for n in ('loom',):", (ROOT / 'scripts/files_open_gate.py').read_text())
        self.assertIn("('loom','/bin/loom')", (ROOT / 'scripts/drag_drop_gate.py').read_text())

    def test_gaze_restore_identity_and_original_lifecycle_assertions(self):
        stages = tomllib.loads((ROOT / 'sexos_build_spec.toml').read_text())['stage']
        self.assertEqual(next(s for s in stages if s['id'] == 'build_view')['dest_artifact'], 'iso_root/programs/gaze')
        source = (ROOT / 'apps/view/src/main.rs').read_text()
        self.assertIn('window.title(b"Gaze")', source)
        self.assertIn('[view.shown]', source)
        gate = (ROOT / 'scripts/session_restore_gate.py').read_text()
        self.assertNotIn('/bin/view', gate)
        self.assertNotIn('/data/view', gate)
        self.assertEqual(gate.count("expect('run gaze @pic.png'"), 2)
        self.assertIn('path=/bin/gaze', gate)
        self.assertIn('root=/data/gaze', gate)
        self.assertIn('programs=2', gate)
        self.assertIn('scenes==[0,2]', gate)
        self.assertIn("==2,'both did not start'", gate)


if __name__ == '__main__':
    unittest.main()
