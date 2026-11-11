import importlib.util
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'make_boot_image.py'
spec = importlib.util.spec_from_file_location('boot_image', SCRIPT)
boot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boot)

class RecorderModules(unittest.TestCase):
    def test_missing_duplicate_and_unknown_modules_reject(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            line = '    MODULE_PATH=boot:///programs/record\n'
            with self.assertRaises(FileNotFoundError):
                boot.recorder_module(root, line)
            for config in (line + line, 'MODULE_PATH=boot:///programs/other\n'):
                with self.assertRaises(ValueError):
                    boot.recorder_module(root, config)

    def test_repository_default_and_compact_config_share_one_module(self):
        config=(SCRIPT.parents[1]/'limine.cfg').read_text()
        entries=sum(line.startswith(':')for line in config.splitlines())
        self.assertGreaterEqual(entries,2)
        self.assertEqual(config.count('MODULE_PATH=boot:///apps/hello'),entries)
        config=config.replace('MODULE_PATH=boot:///apps/hello','MODULE_PATH=boot:///apps/hello\n    MODULE_PATH=boot:///programs/record')
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);(root/'programs').mkdir();record=root/'programs/record';record.write_bytes(b'fixture exact module')
            self.assertEqual(boot.recorder_module(root,config),record)
            with self.assertRaises(ValueError):boot.recorder_module(root,config.replace('MODULE_PATH=boot:///programs/record','MODULE_PATH=boot:///programs/record\n    MODULE_PATH=boot:///programs/record',1))

    @unittest.skipUnless(all(shutil.which(x) for x in ('mformat','mmd','mcopy','mtype')), 'mtools required')
    def test_actual_fat_copies_only_configured_record_module(self):
        def run(*args):
            return subprocess.check_output(args, stderr=subprocess.STDOUT)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'root'
            for sub in ('boot/limine','servers','apps','programs'):
                (root/sub).mkdir(parents=True)
            # Synthetic labelled FAT fixture, not an executable OS image.
            efi_image = root/'boot/limine/limine-uefi-cd.bin'
            with efi_image.open('wb') as f:
                f.truncate(4*1024*1024)
            run('mformat','-i',str(efi_image),'::')
            run('mmd','-i',str(efi_image),'::/EFI','::/EFI/BOOT')
            efi = Path(tmp)/'efi.fixture'; efi.write_bytes(b'fixture EFI')
            run('mcopy','-i',str(efi_image),str(efi),'::/EFI/BOOT/BOOTX64.EFI')
            (root/'sexos-kernel').write_bytes(b'fixture kernel')
            record = root/'programs/record'; record.write_bytes(b'fixture recorder exact bytes')
            (root/'programs/unrelated').write_bytes(b'not a boot module')
            for enabled in (False, True):
                config = ':Default\nKERNEL_PATH=boot:///sexos-kernel\nMODULE_PATH=boot:///boot.img\n'
                if enabled:
                    config += '    MODULE_PATH=boot:///programs/record\n:Compact\nKERNEL_PATH=boot:///sexos-kernel\n    MODULE_PATH=boot:///programs/record\n'
                (root/'limine.cfg').write_text(config)
                image = Path(tmp)/('enabled.img' if enabled else 'normal.img')
                run('python3',str(SCRIPT),'--root',str(root),'--out',str(image))
                self.assertEqual(image.stat().st_size, (boot.SECTORS_WITH_FIRMWARE if enabled else boot.SECTORS)*512)
                installed = run('mtype','-i',str(image),'::/limine.cfg').decode()
                self.assertNotIn('boot:///boot.img', installed)
                if enabled:
                    self.assertIn('boot:///programs/record', installed)
                    self.assertEqual(run('mtype','-i',str(image),'::/programs/record'), record.read_bytes())
                else:
                    with self.assertRaises(subprocess.CalledProcessError):
                        run('mtype','-i',str(image),'::/programs/record')
                with self.assertRaises(subprocess.CalledProcessError):
                    run('mtype','-i',str(image),'::/programs/unrelated')

if __name__ == '__main__':
    unittest.main()
