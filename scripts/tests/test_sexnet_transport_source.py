"""Isolated transport tests must compile the exact production dependency source."""
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import sexnet_transport_source as gate
ROOT=Path(__file__).resolve().parents[2]
LINE='smoltcp = { path = "vendor/smoltcp-0.14.0" }'

class TransportSourceCustody(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name).resolve()/'root';self.root.mkdir()
        self.out=Path(self.temp.name).resolve()/'isolated/crates/sex-net'
        for n in('Cargo.toml','Cargo.lock'):
            shutil.copyfile(ROOT/n,self.root/n)
        for n in('crates/sex-net','crates/sex-net-config'):
            (self.root/n).mkdir(parents=True)
            shutil.copyfile(ROOT/n/'Cargo.toml',self.root/n/'Cargo.toml')
            shutil.copytree(ROOT/n/'src',self.root/n/'src')
        (self.root/'vendor').mkdir()
        shutil.copytree(ROOT/gate.VENDOR,self.root/gate.VENDOR)

    def metadata(self,source=None,manifest=None):
        return {'packages':[{'name':'smoltcp','version':'0.14.0','source':source,
                             'manifest_path':str(manifest or self.out.parents[1]/gate.VENDOR/'Cargo.toml')} ]}

    def change_patch(self,value):
        p=self.root/'Cargo.toml';p.write_text(p.read_text().replace(LINE,'smoltcp = '+value))

    def test_exact_production_patch_is_copied_and_metadata_bound(self):
        gate.prepare(self.root,self.out)
        self.assertEqual(gate.vendor_custody(self.root/gate.VENDOR),gate.vendor_custody(self.out.parents[1]/gate.VENDOR))
        self.assertIn('smoltcp = { path = "../../vendor/smoltcp-0.14.0" }',(self.out/'Cargo.toml').read_text())
        gate.verify_selection(self.root,self.out,self.metadata())

    def test_registry_default_stays_available(self):
        p=self.root/'Cargo.toml';p.write_text(p.read_text().replace(LINE,''))
        gate.prepare(self.root,self.out)
        self.assertNotIn('[patch.crates-io]',(self.out/'Cargo.toml').read_text())
        registry='registry+https://github.com/rust-lang/crates.io-index'
        gate.verify_selection(self.root,self.out,self.metadata(source=registry))
        with self.assertRaises(ValueError):gate.verify_selection(self.root,self.out,self.metadata())

    def test_arbitrary_path_version_git_or_other_override_rejects(self):
        original=(self.root/'Cargo.toml').read_text()
        for value in('{ path = "/tmp/other" }','{ path = "vendor/smoltcp-0.14.0", version = "*" }',
                      '{ git = "https://example.invalid/other" }','{ path = "vendor/../vendor/smoltcp-0.14.0" }'):
            self.change_patch(value)
            with self.subTest(value=value),self.assertRaises(ValueError):gate.selected_patch(self.root)
            (self.root/'Cargo.toml').write_text(original)
        for changed in(original.replace(LINE,LINE+'\nother = { path = "vendor/other" }'),
                        original+'\n[replace]\n"smoltcp:0.14.0" = { path = "vendor/smoltcp-0.14.0" }\n'):
            (self.root/'Cargo.toml').write_text(changed)
            with self.assertRaises(ValueError):gate.selected_patch(self.root)

    def test_modified_manifest_source_license_or_provenance_rejects(self):
        for name in('Cargo.toml','src/iface/neighbor.rs','src/iface/interface/mod.rs','LICENSE-0BSD.txt','UPSTREAM.json'):
            p=self.root/gate.VENDOR/name;original=p.read_bytes();p.write_bytes(original+b'\n')
            with self.subTest(name=name),self.assertRaises(ValueError):gate.vendor_custody(self.root/gate.VENDOR)
            p.write_bytes(original)
        p=self.root/gate.VENDOR/'src/extra.rs';p.write_text('fn unknown(){}')
        with self.assertRaises(ValueError):gate.vendor_custody(self.root/gate.VENDOR)

    def test_symlink_or_outside_manifest_cannot_replace_exact_source(self):
        p=self.root/gate.VENDOR/'src/lib.rs';original=p.read_bytes();p.unlink();p.symlink_to(ROOT/gate.VENDOR/'src/lib.rs')
        with self.assertRaises(ValueError):gate.vendor_custody(self.root/gate.VENDOR)
        p.unlink();p.write_bytes(original)
        gate.prepare(self.root,self.out)
        for meta in(self.metadata(manifest=ROOT/gate.VENDOR/'Cargo.toml'),self.metadata(source='registry+https://github.com/rust-lang/crates.io-index')):
            with self.assertRaises(ValueError):gate.verify_selection(self.root,self.out,meta)

    def test_copy_mutation_cannot_pass_metadata_custody(self):
        gate.prepare(self.root,self.out)
        p=self.out.parents[1]/gate.VENDOR/'src/iface/neighbor.rs';p.write_bytes(p.read_bytes()+b'\n')
        with self.assertRaises(ValueError):gate.verify_selection(self.root,self.out,self.metadata())

if __name__=='__main__':unittest.main()
