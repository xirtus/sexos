"""scripts/ci_scope.sh: which CI runs a change needs (full, smoke or docs)."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]


class CiScope(unittest.TestCase):
    def setUp(self):
        self.dir=tempfile.TemporaryDirectory();self.addCleanup(self.dir.cleanup)
        self.root=Path(self.dir.name)
        (self.root/'scripts').mkdir();shutil.copyfile(ROOT/'scripts/ci_scope.sh',self.root/'scripts/ci_scope.sh')
        (self.root/'docs').mkdir();(self.root/'src').mkdir()
        (self.root/'docs/NOTES.md').write_text('a\n');(self.root/'docs/READ_BY_CODE.md').write_text('a\n')
        (self.root/'src/lib.rs').write_text('const D: &str = include_str!("../docs/READ_BY_CODE.md");\n')
        self.git('init','-q');self.git('add','.');self.git('-c','user.name=t','-c','user.email=t@t','commit','-qm','base')

    def git(self,*args):
        subprocess.run(['git',*args],cwd=self.root,check=True,capture_output=True)

    def scope(self,event,change):
        for path,text in change.items():
            (self.root/path).parent.mkdir(parents=True,exist_ok=True);(self.root/path).write_text(text)
        self.git('add','.');self.git('-c','user.name=t','-c','user.email=t@t','commit','-qm','change')
        return subprocess.run(['bash','scripts/ci_scope.sh',event],cwd=self.root,check=True,capture_output=True,text=True).stdout.strip()

    def test_documents_nothing_reads_skip_the_gates(self):
        self.assertEqual(self.scope('pull_request',{'docs/NOTES.md':'b\n','README.md':'x\n'}),'docs')

    def test_a_document_code_reads_is_code(self):
        self.assertEqual(self.scope('pull_request',{'docs/READ_BY_CODE.md':'b\n'}),'full')

    def test_any_code_runs_every_gate(self):
        self.assertEqual(self.scope('pull_request',{'docs/NOTES.md':'b\n','src/lib.rs':'// x\n'}),'full')

    def test_moving_code_into_docs_is_code(self):
        (self.root/'docs/archived.md').write_text((self.root/'src/lib.rs').read_text())
        (self.root/'src/lib.rs').unlink()
        self.assertEqual(self.scope('pull_request',{}),'full')

    def test_master_push_is_smoke_and_nightly_is_full(self):
        self.assertEqual(self.scope('push',{'src/lib.rs':'// y\n'}),'smoke')
        self.assertEqual(self.scope('schedule',{'docs/NOTES.md':'c\n'}),'full')
        self.assertEqual(self.scope('workflow_dispatch',{'docs/NOTES.md':'d\n'}),'full')


if __name__=='__main__':unittest.main()
