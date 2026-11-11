"""Independent literal catalog, complete BOOT source constraints and negatives."""
import sys,tempfile,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'scripts'))
from program_role_facts import SEAL,ordinary_rows,assert_ordinary_launch,assert_private_ordinary_launch,check_ordinary_launches

class ProgramFacts(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name);path=self.root/'crates/sex-pdx/src/app_catalog.rs';path.parent.mkdir(parents=True)
        path.write_text('\n'.join(f'app: App::Program, ordinal: {ordinal}, target: {target}, surface: {surface}' for ordinal,target,surface in [(0,41,358),(1,42,359),(2,43,360)]))
    def text(self,pinned=0):
        return (f'[boot.app.successor] logical=41 incarnation=2 predecessor=41 successor=42 task=42 surface=359 key=5 dormant=1 leaves=24 runtime_mint=0 source_pinned={pinned}\n'
                '[boot.app.successor] logical=41 incarnation=3 predecessor=42 successor=43 task=43 surface=360 key=6 dormant=1 leaves=24 runtime_mint=0 source_pinned=0\n'+SEAL+'\n')
    def test_default_and_reserved_policy_select_catalog_subjects(self):
        self.assertEqual([row['target']for row in ordinary_rows(self.root,self.text())],[42,43])
        self.assertEqual([row['target']for row in ordinary_rows(self.root,self.text(1))],[43])
        self.assertEqual(assert_ordinary_launch(self.root,self.text(1),43,1073741830,360)['key'],6)
        for target,task,surface in [(42,42,359),(43,0,360),(43,107,359),(44,44,361)]:
            with self.assertRaises(AssertionError):assert_ordinary_launch(self.root,self.text(1),target,task,surface)
    def test_private_subject_keeps_pinned_key_in_the_comparison(self):
        self.assertEqual(assert_private_ordinary_launch(self.root,self.text(1),43,107,360)['key'],6)
        shared=self.text(1).replace('key=5','key=6')
        with self.assertRaises(AssertionError):assert_private_ordinary_launch(self.root,shared,43,107,360)
    def test_complete_ordinary_receipts_are_checked_before_progress(self):
        text=self.text(1)
        partial='[shell.program.launch] target=42 task=42 surface=359'
        self.assertEqual(check_ordinary_launches(self.root,text+partial),len(text))
        with self.assertRaises(AssertionError):check_ordinary_launches(self.root,text+partial+'\n')
        valid=text+'[shell.program.launch] target=43 task=107 surface=360\n'
        self.assertEqual(check_ordinary_launches(self.root,valid),len(valid))
        for suffix in [' extra=1',' task=108']:
            with self.assertRaises(AssertionError):check_ordinary_launches(self.root,valid.rstrip('\n')+suffix+'\n')
    def test_complete_seal_and_constraint_bits_are_required(self):
        for text in [self.text(2),self.text().replace('source_pinned=0','source_pinned=00',1),self.text().replace(' source_pinned=0','',1),self.text()[:-1],self.text()+SEAL+'\n']:
            with self.assertRaises(AssertionError):ordinary_rows(self.root,text)
    def test_duplicates_unknown_fields_and_subjects_fail_closed(self):
        line=self.text().splitlines(keepends=True)[0]
        for text in ['[boot.app.successor]\n'+self.text(),line+self.text(),self.text().replace('successor=42','successor=99',1),self.text().replace('dormant=1','dormant=1 extra=0',1),self.text().replace('key=5','key=5 key=6',1),self.text().replace('surface=359','surface=360',1),self.text().replace('predecessor=41','predecessor=40',1)]:
            with self.assertRaises(AssertionError):ordinary_rows(self.root,text)
    def test_later_user_serial_cannot_remove_a_boot_source_constraint(self):
        forged=self.text().splitlines(keepends=True)[0]
        self.assertEqual([row['target']for row in ordinary_rows(self.root,self.text(1)+forged)],[43])
        self.assertEqual([row['target']for row in ordinary_rows(self.root,self.text(1)+forged[:-1])],[43])

    def test_constructor_rows_participate_in_the_complete_sealed_catalog(self):
        path=self.root/'crates/sex-pdx/src/app_catalog.rs'
        path.write_text('pub const INCARNATIONS: [Incarnation; 5] = [\n'+path.read_text()+'\n    quil(0, 9, 156, Some(201), 38, 41, None, 11),\n    quil(1, 16, 301, None, 45, 46, Some(44), 47),\n];')
        line='[boot.app.successor] logical=9 incarnation=2 predecessor=9 successor=16 task=16 surface=301 key=9 dormant=1 leaves=24 runtime_mint=0 source_pinned=0\n'
        self.assertEqual([row['target']for row in ordinary_rows(self.root,line+self.text())],[42,43])
        with self.assertRaises(AssertionError):ordinary_rows(self.root,self.text())
        path.write_text(path.read_text().replace('quil(1,','unknown_constructor(1,'))
        with self.assertRaises(AssertionError):ordinary_rows(self.root,line+self.text())

if __name__=='__main__':unittest.main()
