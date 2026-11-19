"""The installed image must build the approved helper before its Files pin."""
import tomllib
import unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
class MoanHelperStage(unittest.TestCase):
    def test_helper_is_scanner_qualified_app_input_to_files(self):
        spec=tomllib.loads((ROOT/'sexos_build_spec.toml').read_text())
        stages=spec['stage'];helper=[(i,s)for i,s in enumerate(stages)if s['id']=='build_moanplug'];files=[(i,s)for i,s in enumerate(stages)if s['id']=='build_sexfiles']
        self.assertEqual(len(helper),1);self.assertEqual(len(files),1)
        index,stage=helper[0];self.assertLess(index,files[0][0])
        self.assertEqual(stage['action'],'cargo_manifest');self.assertEqual(stage['target'],'x86_64-sex-app.json')
        self.assertEqual(stage['manifest'],'apps/moanplug/Cargo.toml');self.assertIn(stage['manifest'],spec['allowed']['crates'])
        self.assertEqual(stage['source_artifact'],'target/x86_64-sex-app/release/moanplug')
        self.assertEqual(stage['dest_artifact'],'iso_root/programs/moanplug')
        self.assertEqual(files[0][1]['consumed_inputs'],[stage['dest_artifact']])
        self.assertFalse(any(s.get('manifest')=='apps/moanrack/Cargo.toml'for s in stages),'qualification fixture must not ship')
if __name__=='__main__':unittest.main()
