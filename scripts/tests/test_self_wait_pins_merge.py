"""Two branches that each add a self-wait pin merge without a conflict."""
import ast
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT=Path(__file__).resolve().parents[2]
PINS=ROOT/'scripts/self_wait_pins.py'


def git(cwd,*args):
    return subprocess.run(['git','-c','user.name=t','-c','user.email=t@t',*args],cwd=cwd,check=True,capture_output=True,text=True)


def add_pin(text,line):
    # A re-pin adds one line right after `PINS = (`, as every branch would.
    return text.replace('PINS = (\n','PINS = (\n'+line+'\n\n',1)


class PinsMerge(unittest.TestCase):
    def test_file_is_marked_union_and_one_pin_per_line(self):
        self.assertIn('scripts/self_wait_pins.py merge=union',(ROOT/'.gitattributes').read_text().splitlines())
        body=PINS.read_text().split('PINS = (\n',1)[1]
        for line in body.splitlines():
            if line in ('',')'):continue
            self.assertTrue(line.startswith("    ('"),line)
            self.assertEqual(len(ast.literal_eval(line.split('),',1)[0].strip()+')')),3,line)

    def test_parallel_repins_both_survive_a_merge(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo=Path(tmp);(repo/'scripts').mkdir()
            shutil.copyfile(PINS,repo/'scripts/self_wait_pins.py')
            (repo/'.gitattributes').write_text('scripts/self_wait_pins.py merge=union\n')
            git(repo,'init','-q','-b','main');git(repo,'add','.');git(repo,'commit','-qm','base')
            base=(repo/'scripts/self_wait_pins.py').read_text()
            a="    ('tree', '', '"+'a'*64+"'),  # branch a"
            b="    ('tree', '', '"+'b'*64+"'),  # branch b"
            git(repo,'checkout','-qb','a');(repo/'scripts/self_wait_pins.py').write_text(add_pin(base,a));git(repo,'commit','-qam','a')
            git(repo,'checkout','-q','main');git(repo,'checkout','-qb','b');(repo/'scripts/self_wait_pins.py').write_text(add_pin(base,b));git(repo,'commit','-qam','b')
            git(repo,'merge','-q','--no-edit','a')
            merged=(repo/'scripts/self_wait_pins.py').read_text()
            self.assertIn(a,merged);self.assertIn(b,merged)
            pins=ast.literal_eval(merged.split('PINS = ',1)[1])
            self.assertIn(('tree','','a'*64),pins);self.assertIn(('tree','','b'*64),pins)


if __name__=='__main__':unittest.main()
